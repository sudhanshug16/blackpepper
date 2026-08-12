//! Command-bar completion, grounded in discovered state.
//!
//! Candidates are built from what this client has actually observed — probed
//! listeners, live forwards, configured hosts and services — never from a
//! static argument list. An argument that would be rejected on Enter is never
//! offered, so the completion list and the command parser cannot drift apart.

use super::catalog;
use super::ClientState;
use crate::ports::ForwardStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The full command line this candidate would produce, without the ':'.
    pub value: String,
    /// Where the value came from, shown in the right column.
    pub note: String,
}

/// The trailing placeholder shown after the caret while an argument is still
/// unwritten, e.g. `<port|address:port>`.
pub fn ghost(input: &str) -> Option<&'static str> {
    let trimmed = input.trim_start();
    let expects_argument = trimmed.ends_with(' ') || trimmed.is_empty();
    if !expects_argument {
        return None;
    }
    let words = trimmed.split_whitespace().collect::<Vec<_>>();
    Some(match words.as_slice() {
        ["forward"] => "<port|address:port>",
        ["forward", "cancel"] => "<port|address:port>",
        ["host", "add"] => "<name> <ssh-alias>",
        ["host", "connect" | "disconnect"] => "<name>",
        ["workspace", "add"] => "<path>",
        ["workspace", "switch"] => "<name|id>",
        ["worktree", "create"] => "<branch> [--base <ref>]",
        ["worktree", "open"] => "<branch|pr:123|url>",
        ["agent", "spawn"] => "<codex|claude|opencode>",
        ["service", "start"] => "<name>",
        _ => return None,
    })
}

/// The rule that governs the command being typed, shown beside the run/cancel
/// hint. Only commands with a boundary worth restating carry one — a hint that
/// says nothing costs a row.
pub fn constraint(input: &str) -> Option<&'static str> {
    let words = input.split_whitespace().collect::<Vec<_>>();
    Some(match words.as_slice() {
        ["forward", ..] => "binds to client loopback only",
        ["worktree", "create" | "remove", ..] => "previews first, then :approve",
        ["workspace", "terminate", ..] => "keeps the folder",
        ["host", "add", ..] => "openssh alias, literal",
        _ => return None,
    })
}

/// Candidates for the current input. The command bar shows these in order; an
/// empty result means there is nothing this client can offer, which is itself
/// worth seeing.
pub fn candidates(state: &ClientState, input: &str) -> Vec<Candidate> {
    let trimmed = input.trim_start();
    let words = trimmed.split_whitespace().collect::<Vec<_>>();
    let open = trimmed.ends_with(' ') || trimmed.is_empty();

    match (words.as_slice(), open) {
        (["forward"], true) => forward_targets(state),
        (["forward", partial], false) => prefixed(forward_targets(state), "forward", partial),
        (["forward", "cancel"], true) => forward_cancels(state),
        (["forward", "cancel", partial], false) => {
            prefixed(forward_cancels(state), "forward cancel", partial)
        }
        (["agent", "spawn"], true) => providers(),
        (["agent", "spawn", partial], false) => prefixed(providers(), "agent spawn", partial),
        (["service", "start"], true) => services(state),
        (["service", "start", partial], false) => {
            prefixed(services(state), "service start", partial)
        }
        (["host", verb @ ("connect" | "disconnect")], true) => hosts(state, verb),
        (["host", verb @ ("connect" | "disconnect"), partial], false) => {
            prefixed(hosts(state, verb), &format!("host {verb}"), partial)
        }
        (["workspace", "switch"], true) => workspaces(state),
        (["workspace", "switch", partial], false) => {
            prefixed(workspaces(state), "workspace switch", partial)
        }
        (_, _) => commands(state, trimmed),
    }
}

/// Bare command-name completion. Unavailable commands are offered with the
/// reason attached rather than hidden, matching `:help`.
fn commands(state: &ClientState, partial: &str) -> Vec<Candidate> {
    if partial.contains(' ') {
        return Vec::new();
    }
    catalog::entries(state)
        .into_iter()
        .filter_map(|entry| {
            let value = entry.syntax.trim_start_matches(':');
            let name = value.split_whitespace().next().unwrap_or(value);
            name.starts_with(partial).then(|| Candidate {
                value: value.to_owned(),
                note: entry.note,
            })
        })
        .take(8)
        .collect()
}

fn forward_targets(state: &ClientState) -> Vec<Candidate> {
    let Some(snapshot) = state
        .selected_workspace
        .or(state.active_workspace)
        .and_then(|id| state.host_for_workspace(id))
        .and_then(|host_id| state.ports.get(&host_id))
    else {
        return Vec::new();
    };
    snapshot
        .listeners
        .iter()
        .filter(|listener| listener.forward_target().is_ok())
        .map(|listener| Candidate {
            value: format!("forward {}", listener.port),
            note: format!(
                "discovered · {}",
                listener.process.as_deref().unwrap_or("unknown")
            ),
        })
        .collect()
}

fn forward_cancels(state: &ClientState) -> Vec<Candidate> {
    state
        .forwards
        .iter()
        .filter(|forward| {
            matches!(
                forward.status,
                ForwardStatus::Active | ForwardStatus::Direct
            )
        })
        .map(|forward| Candidate {
            value: format!("forward cancel {}", forward.target().remote_port),
            note: "active on this client".to_owned(),
        })
        .collect()
}

fn providers() -> Vec<Candidate> {
    ["codex", "claude", "opencode"]
        .into_iter()
        .map(|provider| Candidate {
            value: format!("agent spawn {provider}"),
            note: "provider".to_owned(),
        })
        .collect()
}

fn services(state: &ClientState) -> Vec<Candidate> {
    state
        .config
        .startup
        .iter()
        .map(|entry| Candidate {
            value: format!("service start {}", entry.name),
            note: if entry.auto_start {
                "configured · auto-start".to_owned()
            } else {
                "configured".to_owned()
            },
        })
        .collect()
}

fn hosts(state: &ClientState, verb: &str) -> Vec<Candidate> {
    state
        .tree
        .iter()
        .map(|host| Candidate {
            value: format!("host {verb} {}", host.label),
            note: host.connection.public_word().to_owned(),
        })
        .collect()
}

fn workspaces(state: &ClientState) -> Vec<Candidate> {
    state
        .snapshot
        .workspaces
        .iter()
        .filter_map(|workspace| {
            let label = workspace.display_name.clone().or_else(|| {
                std::path::Path::new(&workspace.root_path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })?;
            let host = state
                .snapshot
                .hosts
                .iter()
                .find(|host| host.id == workspace.host_id)
                .map(|host| host.display_name.clone())
                .unwrap_or_else(|| "unknown host".to_owned());
            Some(Candidate {
                value: format!("workspace switch {label}"),
                note: host,
            })
        })
        .collect()
}

fn prefixed(candidates: Vec<Candidate>, command: &str, partial: &str) -> Vec<Candidate> {
    let prefix = format!("{command} {partial}");
    candidates
        .into_iter()
        .filter(|candidate| candidate.value.starts_with(&prefix))
        .collect()
}
