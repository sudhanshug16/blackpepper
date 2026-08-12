//! The command catalog, grounded in what this client can currently see.
//!
//! `:help` and command completion both read from here, so the two can never
//! disagree about what will run. Nothing is filtered out for being
//! unavailable — an entry that cannot run stays listed with the reason it
//! cannot, because a command silently missing from help reads as a command
//! that does not exist.

mod helpers;

use self::helpers::{active_forwards, entry, explain_note};
use super::ClientState;
use crate::ports::ProbeCompleteness;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommandGroup {
    Workspace,
    Repository,
    Hosts,
}

impl CommandGroup {
    pub const fn heading(self) -> &'static str {
        match self {
            Self::Workspace => "THIS WORKSPACE",
            Self::Repository => "REPOSITORY",
            Self::Hosts => "HOSTS",
        }
    }

    pub const ORDER: [Self; 3] = [Self::Workspace, Self::Repository, Self::Hosts];
}

#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub group: CommandGroup,
    /// Exactly what you would type, argument placeholders included.
    pub syntax: &'static str,
    /// The right-hand column: what this command would do to the state this
    /// client can actually see, or why it would refuse.
    pub note: String,
    /// False dims the row. The row still appears.
    pub available: bool,
}

pub fn entries(state: &ClientState) -> Vec<CatalogEntry> {
    let workspace = state.selected_workspace.or(state.active_workspace);
    let has_workspace = workspace.is_some();
    let attached = state.active_workspace.is_some();

    let providers = "codex · claude · opencode".to_owned();
    let services = if state.config.startup.is_empty() {
        "no [[startup]] entries configured".to_owned()
    } else {
        state
            .config
            .startup
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>()
            .join(" · ")
    };
    let listeners = workspace
        .and_then(|id| state.host_for_workspace(id))
        .and_then(|host_id| state.ports.get(&host_id));
    let listener_note = match listeners {
        Some(snapshot) if snapshot.completeness != ProbeCompleteness::Full => format!(
            "{} listeners known · partial probe",
            snapshot.listeners.len()
        ),
        Some(snapshot) => format!("{} listeners known", snapshot.listeners.len()),
        None => "no probe on this host yet".to_owned(),
    };
    let forwardable = super::completion::forward_target_count(state);

    let repository = workspace.and_then(|id| {
        state
            .snapshot
            .workspaces
            .iter()
            .find(|record| record.id == id)
            .and_then(|record| record.repository.as_ref())
    });
    let approval = state
        .pending_approval
        .as_ref()
        .map(|_| "reviewed plan is waiting".to_owned());

    let hosts = state
        .snapshot
        .hosts
        .iter()
        .filter(|host| matches!(host.transport, crate::core::HostTransport::Ssh { .. }))
        .count();
    let connecting = state
        .tree
        .iter()
        .find(|host| {
            matches!(
                host.connection,
                crate::client::HostConnection::Authenticating
                    | crate::client::HostConnection::Reconnecting
            )
        })
        .map(|host| host.label.clone());

    let mut entries = vec![
        entry(
            CommandGroup::Workspace,
            ":workspace switch <name|id>",
            "attach by name across every host".to_owned(),
            !state.snapshot.workspaces.is_empty(),
            "no workspaces registered",
        ),
        entry(
            CommandGroup::Workspace,
            ":workspace add <path>",
            "register a folder on the selected host".to_owned(),
            state.selected_host.is_some(),
            "select a host first",
        ),
        entry(
            CommandGroup::Workspace,
            ":agent spawn <provider>",
            providers,
            has_workspace,
            "select a workspace first",
        ),
        entry(
            CommandGroup::Workspace,
            ":service start <name>",
            services,
            has_workspace && !state.config.startup.is_empty(),
            "select a workspace first",
        ),
        entry(
            CommandGroup::Workspace,
            ":ports [--all-host]",
            listener_note,
            has_workspace,
            "select a workspace first",
        ),
        entry(
            CommandGroup::Workspace,
            ":forward <port|address:port>",
            if forwardable == 0 {
                "no listener discovered yet".to_owned()
            } else {
                "to client loopback".to_owned()
            },
            forwardable > 0,
            "no listener discovered yet",
        ),
        entry(
            CommandGroup::Workspace,
            ":forward cancel <port|address:port>",
            format!("{} active on this client", active_forwards(state)),
            active_forwards(state) > 0,
            "nothing forwarded from this client",
        ),
        entry(
            CommandGroup::Workspace,
            ":workspace ungroup",
            "keep this folder outside repository grouping".to_owned(),
            has_workspace,
            "select a workspace first",
        ),
        entry(
            CommandGroup::Workspace,
            ":workspace terminate",
            "ends the zellij session, keeps the folder".to_owned(),
            attached,
            "no session attached",
        ),
        entry(
            CommandGroup::Repository,
            ":worktree list",
            "branches and worktrees".to_owned(),
            repository.is_some(),
            "this workspace is not a git repository",
        ),
        entry(
            CommandGroup::Repository,
            ":worktree create <branch> [--base <ref>]",
            "previews first, then :approve".to_owned(),
            repository.is_some(),
            "this workspace is not a git repository",
        ),
        entry(
            CommandGroup::Repository,
            ":worktree open <branch|pr:123|url>",
            "branch · PR · URL".to_owned(),
            repository.is_some(),
            "this workspace is not a git repository",
        ),
        entry(
            CommandGroup::Repository,
            ":worktree remove",
            "journaled, never forced".to_owned(),
            repository.is_some(),
            "this workspace is not a git repository",
        ),
        entry(
            CommandGroup::Repository,
            ":approve",
            approval.clone().unwrap_or_default(),
            approval.is_some(),
            "nothing under review",
        ),
        entry(
            CommandGroup::Hosts,
            ":host add <name> <ssh-alias>",
            "openssh alias, literal".to_owned(),
            true,
            "",
        ),
        entry(
            CommandGroup::Hosts,
            ":host import",
            "preview aliases from ~/.ssh/config".to_owned(),
            true,
            "",
        ),
        entry(
            CommandGroup::Hosts,
            ":host connect <name>",
            connecting
                .map(|host| format!("{host} connecting"))
                .unwrap_or_else(|| format!("{hosts} configured")),
            hosts > 0,
            "no [hosts] entries configured",
        ),
        entry(
            CommandGroup::Hosts,
            ":host disconnect <name>",
            "keeps sessions running on the host".to_owned(),
            hosts > 0,
            "no [hosts] entries configured",
        ),
        entry(
            CommandGroup::Hosts,
            ":theme [<name>]",
            format!(
                "{} of {} palettes",
                state.config.ui.theme.name,
                crate::client_config::theme::THEMES.len()
            ),
            true,
            "",
        ),
        entry(
            CommandGroup::Hosts,
            ":refresh",
            "registries, agents, and ports".to_owned(),
            true,
            "",
        ),
        entry(
            CommandGroup::Hosts,
            ":status explain",
            explain_note(state, workspace),
            has_workspace,
            "select a workspace first",
        ),
        entry(
            CommandGroup::Hosts,
            ":help",
            "commands, context, and key bindings".to_owned(),
            true,
            "",
        ),
        entry(
            CommandGroup::Hosts,
            ":quit",
            "detach and exit".to_owned(),
            true,
            "",
        ),
    ];
    entries.sort_by_key(|entry| entry.group);
    entries
}
