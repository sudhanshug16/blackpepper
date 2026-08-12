//! Progressive command-palette completion grounded in observed client state.
//!
//! The palette first narrows command paths (`host` -> `host connect`), then
//! offers values for the argument currently being entered. Dynamic values come
//! only from state this client has observed, and values containing whitespace
//! are shell-quoted before they are inserted.

mod paths;
mod values;

use super::ClientState;
use paths::command_paths;
use values::{
    forward_cancels, forward_targets, hosts, prefixed, prefixed_workspaces, providers, quote,
    services, themes, workspaces,
};

pub const MAX_VISIBLE_CANDIDATES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The full command line this candidate would produce, without the `:`.
    pub value: String,
    /// Where the value came from, shown in the right column.
    pub note: String,
    /// Whether accepting this candidate should leave a space for a required
    /// next argument.
    pub expects_more: bool,
}

impl Candidate {
    fn complete(value: impl Into<String>, note: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            note: note.into(),
            expects_more: false,
        }
    }

    fn path(value: impl Into<String>, note: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            note: note.into(),
            expects_more: true,
        }
    }
}

/// The trailing prompt shown after the caret when the next argument is known.
pub fn ghost(input: &str) -> Option<String> {
    let trimmed = input.trim_start();
    if !trimmed.is_empty() && !trimmed.ends_with(char::is_whitespace) {
        return None;
    }
    let words = shell_words::split(trimmed).ok()?;
    let words = words.iter().map(String::as_str).collect::<Vec<_>>();
    Some(
        match words.as_slice() {
            [] => "<command>",
            ["host"] => "<add|import|connect|disconnect>",
            ["host", "add"] => "<name>",
            ["host", "add", _] => "<ssh-alias>",
            ["host", "connect" | "disconnect"] => "<name>",
            ["workspace"] => "<add|switch|ungroup|terminate>",
            ["workspace", "add"] => "<path>",
            ["workspace", "switch"] => "<name|id>",
            ["worktree"] => "<list|create|open|remove>",
            ["worktree", "create"] => "<branch>",
            ["worktree", "create", _] => "[--base <ref>] · Enter runs",
            ["worktree", "create", _, "--base"] => "<ref>",
            ["worktree", "open"] => "<branch|pr:123|url>",
            ["agent"] => "<spawn>",
            ["agent", "spawn"] => "<codex|claude|opencode>",
            ["service"] => "<start>",
            ["service", "start"] => "<name>",
            ["theme"] => "<name>",
            ["ports"] => "[--all-host] · Enter runs",
            ["forward"] => "<port|address:port>",
            ["forward", "cancel"] => "<port|address:port>",
            ["status"] => "<explain>",
            _ => return None,
        }
        .to_owned(),
    )
}

/// Candidates for the current input. The normal palette capacity is bounded;
/// the renderer clips it further when an unusually short terminal cannot fit
/// every row.
pub fn candidates(state: &ClientState, input: &str) -> Vec<Candidate> {
    let trimmed = input.trim_start();
    let words = shell_words::split(trimmed)
        .unwrap_or_else(|_| trimmed.split_whitespace().map(str::to_owned).collect());
    let words = words.iter().map(String::as_str).collect::<Vec<_>>();
    let open = trimmed.ends_with(char::is_whitespace) || trimmed.is_empty();

    let dynamic = match (words.as_slice(), open) {
        (["forward"], true) => Some(forward_choices(state, "")),
        (["forward", partial], false) => Some(forward_choices(state, partial)),
        (["forward", "cancel"], true) => Some(forward_cancels(state)),
        (["forward", "cancel", partial], false) => {
            Some(prefixed(forward_cancels(state), "forward cancel", partial))
        }
        (["agent", "spawn"], true) => Some(providers()),
        (["agent", "spawn", partial], false) => Some(prefixed(providers(), "agent spawn", partial)),
        (["service", "start"], true) => Some(services(state)),
        (["service", "start", partial], false) => {
            Some(prefixed(services(state), "service start", partial))
        }
        (["host", verb @ ("connect" | "disconnect")], true) => Some(hosts(state, verb)),
        (["host", verb @ ("connect" | "disconnect"), partial], false) => Some(prefixed(
            hosts(state, verb),
            &format!("host {verb}"),
            partial,
        )),
        (["theme"], true) => Some(themes(state)),
        (["theme", partial], false) => Some(prefixed(themes(state), "theme", partial)),
        (["workspace", "switch"], true) => Some(workspaces(state)),
        (["workspace", "switch", partial], false) => {
            Some(prefixed_workspaces(workspaces(state), partial))
        }
        (["ports"], true) => Some(vec![Candidate::complete(
            "ports --all-host",
            "include every listener on this host",
        )]),
        (["worktree", "create", branch], true) => Some(vec![Candidate::path(
            format!("worktree create {} --base", quote(branch)),
            "optional base ref",
        )]),
        _ => None,
    };

    dynamic
        .unwrap_or_else(|| command_paths(state, trimmed, open))
        .into_iter()
        .take(MAX_VISIBLE_CANDIDATES)
        .collect()
}

/// The safety boundary worth keeping visible beside run/cancel.
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

pub(in crate::client) fn forward_target_count(state: &ClientState) -> usize {
    forward_targets(state).len()
}

/// Convert catalog syntax into the safe command text inserted by a click.
pub fn prefill_from_syntax(syntax: &str) -> String {
    let (value, expects_more) = runnable_prefix(syntax);
    format!(":{value}{}", if expects_more { " " } else { "" })
}

fn forward_choices(state: &ClientState, partial: &str) -> Vec<Candidate> {
    let active = forward_cancels(state).len();
    let mut candidates = Vec::new();
    if "cancel".starts_with(partial.trim_start_matches(['\'', '"'])) {
        candidates.push(Candidate::path(
            "forward cancel",
            match active {
                0 => "no active forwards".to_owned(),
                1 => "1 active forward".to_owned(),
                count => format!("{count} active forwards"),
            },
        ));
    }
    candidates.extend(prefixed(forward_targets(state), "forward", partial));
    candidates
}

fn runnable_prefix(syntax: &str) -> (String, bool) {
    let syntax = syntax.trim_start_matches(':');
    let mut expects_more = false;
    let value = syntax
        .split_whitespace()
        .take_while(|word| {
            let placeholder = word.starts_with('<') || word.starts_with('[');
            if placeholder && word.starts_with('<') {
                expects_more = true;
            }
            !placeholder
        })
        .collect::<Vec<_>>()
        .join(" ");
    (value, expects_more)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_state() -> ClientState {
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        ClientState::new(
            crate::client_config::load_contents(None, None, None).unwrap(),
            crate::core::RegistrySnapshot::default(),
            event_tx,
        )
    }

    #[test]
    fn syntax_prefill_stops_before_placeholders() {
        assert_eq!(
            prefill_from_syntax(":host add <name> <ssh-alias>"),
            ":host add "
        );
        assert_eq!(prefill_from_syntax(":host import"), ":host import");
        assert_eq!(prefill_from_syntax(":ports [--all-host]"), ":ports");
    }

    #[test]
    fn ghost_advances_one_argument_at_a_time() {
        assert_eq!(ghost("host add ").as_deref(), Some("<name>"));
        assert_eq!(ghost("host add lab ").as_deref(), Some("<ssh-alias>"));
        assert_eq!(
            ghost("worktree create feature ").as_deref(),
            Some("[--base <ref>] · Enter runs")
        );
    }

    #[test]
    fn opening_palette_shows_command_families_before_leaf_commands() {
        let state = empty_state();
        let values = candidates(&state, "")
            .into_iter()
            .map(|candidate| candidate.value)
            .collect::<Vec<_>>();

        assert_eq!(
            values,
            [
                "host",
                "theme",
                "refresh",
                "help",
                "quit",
                "workspace",
                "agent",
                "service",
            ]
        );
    }

    #[test]
    fn exact_command_family_immediately_lists_its_children() {
        let state = empty_state();
        let values = candidates(&state, "host")
            .into_iter()
            .map(|candidate| candidate.value)
            .collect::<Vec<_>>();

        assert_eq!(
            values,
            ["host add", "host import", "host connect", "host disconnect"]
        );
    }

    #[test]
    fn forward_arguments_keep_cancel_discoverable_beside_listener_values() {
        let state = empty_state();
        let choices = candidates(&state, "forward ");

        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].value, "forward cancel");
        assert!(choices[0].expects_more);
    }
}
