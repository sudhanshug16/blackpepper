//! State-grounded values for the command argument currently being entered.

use super::Candidate;
use crate::client::ClientState;
use crate::ports::ForwardStatus;

pub(super) fn forward_targets(state: &ClientState) -> Vec<Candidate> {
    let Some(workspace) = state
        .selected_workspace
        .or(state.active_workspace)
        .and_then(|workspace_id| {
            state
                .snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.id == workspace_id)
        })
    else {
        return Vec::new();
    };
    let Some(snapshot) = state.ports.get(&workspace.host_id) else {
        return Vec::new();
    };
    let listeners = snapshot
        .listeners
        .iter()
        .filter(|listener| {
            state.show_all_host_ports
                || crate::client::runtime::ports::listener_matches_workspace(
                    listener.workspace_path.as_deref(),
                    &workspace.root_path,
                )
        })
        .collect::<Vec<_>>();
    listeners
        .iter()
        .filter_map(|listener| {
            let target = listener.forward_target().ok()?;
            if crate::ports::target_is_ambiguous(&snapshot.listeners, &target) {
                return None;
            }
            // Prefer the short port selector only when the command resolver
            // proves it identifies this exact listener. Otherwise insert the
            // full endpoint so clicking a completion can never hit a later
            // ambiguity error.
            let selector = if crate::ports::resolve_forward_target(
                listeners.iter().copied(),
                target.remote_port,
                None,
            )
            .is_ok()
            {
                target.remote_port.to_string()
            } else {
                target.endpoint()
            };
            Some(Candidate::complete(
                format!("forward {selector}"),
                format!(
                    "discovered · {}",
                    listener.process.as_deref().unwrap_or("unknown")
                ),
            ))
        })
        .collect()
}

pub(super) fn forward_cancels(state: &ClientState) -> Vec<Candidate> {
    let workspace = state.selected_workspace.or(state.active_workspace);
    let forwards = state
        .forwards
        .iter()
        .filter(|forward| {
            Some(forward.workspace_id) == workspace
                && matches!(
                    forward.status,
                    ForwardStatus::Active | ForwardStatus::Direct
                )
        })
        .collect::<Vec<_>>();
    forwards
        .iter()
        .filter_map(|forward| {
            let target = forward.target();
            let exact_matches = forwards
                .iter()
                .filter(|candidate| candidate.target() == target)
                .count();
            if exact_matches > 1 {
                return None;
            }
            let same_port = forwards
                .iter()
                .filter(|candidate| candidate.remote_port == forward.remote_port)
                .count();
            let selector = if same_port == 1 {
                forward.remote_port.to_string()
            } else {
                target.endpoint()
            };
            Some(Candidate::complete(
                format!("forward cancel {selector}"),
                "active in this workspace",
            ))
        })
        .collect()
}

pub(super) fn providers() -> Vec<Candidate> {
    ["codex", "claude", "opencode"]
        .into_iter()
        .map(|provider| {
            Candidate::complete(format!("agent spawn {provider}"), "integrated provider")
        })
        .collect()
}

pub(super) fn themes(state: &ClientState) -> Vec<Candidate> {
    let current = state.config.ui.theme.name;
    crate::client_config::theme::THEMES
        .iter()
        .map(|theme| {
            Candidate::complete(
                format!("theme {}", theme.name),
                if theme.name == current {
                    format!("current · {}", theme.summary)
                } else {
                    theme.summary.to_owned()
                },
            )
        })
        .collect()
}

pub(super) fn services(state: &ClientState) -> Vec<Candidate> {
    state
        .config
        .startup
        .iter()
        .map(|entry| {
            Candidate::complete(
                format!("service start {}", quote(&entry.name)),
                if entry.auto_start {
                    "configured · auto-start"
                } else {
                    "configured"
                },
            )
        })
        .collect()
}

pub(super) fn hosts(state: &ClientState, verb: &str) -> Vec<Candidate> {
    state
        .tree
        .iter()
        .filter(|host| host.connection != crate::client::HostConnection::Local)
        .map(|host| {
            Candidate::complete(
                format!("host {verb} {}", quote(&host.label)),
                host.connection.public_word(),
            )
        })
        .collect()
}

pub(super) fn workspaces(state: &ClientState) -> Vec<Candidate> {
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
            // Names and folder basenames are convenient until the same value
            // matches more than one workspace. Use the exact ID in that case,
            // while keeping the readable label searchable in the note.
            let ambiguous = state
                .snapshot
                .workspaces
                .iter()
                .filter(|candidate| workspace_matches(candidate, &label))
                .count()
                > 1;
            let selector = if ambiguous {
                workspace.id.to_string()
            } else {
                label.clone()
            };
            Some(Candidate::complete(
                format!("workspace switch {}", quote(&selector)),
                if ambiguous {
                    format!("{label} · {host}")
                } else {
                    host
                },
            ))
        })
        .collect()
}

fn workspace_matches(workspace: &crate::core::WorkspaceRecord, selector: &str) -> bool {
    workspace.display_name.as_deref() == Some(selector)
        || std::path::Path::new(&workspace.root_path)
            .file_name()
            .is_some_and(|name| name == selector)
}

pub(super) fn prefixed(candidates: Vec<Candidate>, command: &str, partial: &str) -> Vec<Candidate> {
    let command_words = command.split_whitespace().count();
    let partial = partial.trim_start_matches(['\'', '"']);
    candidates
        .into_iter()
        .filter(|candidate| {
            shell_words::split(&candidate.value)
                .ok()
                .and_then(|words| words.get(command_words).cloned())
                .is_some_and(|argument| argument.starts_with(partial))
        })
        .collect()
}

pub(super) fn prefixed_workspaces(candidates: Vec<Candidate>, partial: &str) -> Vec<Candidate> {
    let partial = partial.trim_start_matches(['\'', '"']);
    candidates
        .into_iter()
        .filter(|candidate| {
            shell_words::split(&candidate.value)
                .ok()
                .and_then(|words| words.get(2).cloned())
                .is_some_and(|argument| argument.starts_with(partial))
                || candidate
                    .note
                    .split_once(" · ")
                    .is_some_and(|(label, _)| label.starts_with(partial))
        })
        .collect()
}

pub(super) fn quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_./:@+".contains(character))
    {
        return value.to_owned();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::{prefixed_workspaces, quote, workspaces};

    #[test]
    fn quoted_completion_round_trips_spaces_and_quotes() {
        let value = "api worker's beta";
        let quoted = quote(value);
        assert_eq!(shell_words::split(&quoted).unwrap(), [value]);
    }

    #[test]
    fn duplicate_workspace_labels_complete_to_exact_searchable_ids() {
        let first_host = crate::core::HostRecord::new("first", crate::core::HostTransport::Local);
        let second_host = crate::core::HostRecord::new("second", crate::core::HostTransport::Local);
        let mut first = crate::core::WorkspaceRecord::new(first_host.id, "/one/shared");
        let mut second = crate::core::WorkspaceRecord::new(second_host.id, "/two/shared");
        first.display_name = Some("shared".to_owned());
        second.display_name = Some("shared".to_owned());
        let expected_ids = [first.id.to_string(), second.id.to_string()];
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let state = crate::client::ClientState::new(
            crate::client_config::load_contents(None, None, None).unwrap(),
            crate::core::RegistrySnapshot {
                hosts: vec![first_host, second_host],
                workspaces: vec![first, second],
                ..crate::core::RegistrySnapshot::default()
            },
            event_tx,
        );

        let candidates = prefixed_workspaces(workspaces(&state), "sha");
        assert_eq!(candidates.len(), 2);
        for (candidate, id) in candidates.iter().zip(expected_ids) {
            assert_eq!(candidate.value, format!("workspace switch {id}"));
            assert!(candidate.note.starts_with("shared · "));
        }
    }
}
