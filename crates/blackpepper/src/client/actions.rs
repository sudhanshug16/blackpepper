mod agents;
mod hosts;
mod ports;
mod workspaces;
mod worktrees;

use super::command::ClientCommand;
use super::{ClientState, HostConnection, COMMAND_HELP};
use crate::client::runtime::ClientRuntime;

pub(super) use agents::{apply_explain, apply_spawned};
pub(super) use hosts::apply_import_preview;
pub(super) use ports::existing_forward_message;
pub(super) use ports::{
    apply_cancelled, apply_forwarded, apply_list as apply_port_list, start_forward_target,
};
pub(super) use workspaces::apply_ungrouped_workspace;
pub(super) use worktrees::{
    apply_change as apply_worktree_change, apply_list as apply_worktree_list,
};

pub(super) fn execute_command(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    command: ClientCommand,
) {
    let result = execute(state, runtime, command);
    if let Err(error) = result {
        state.set_output(error);
    }
    state.rebuild_tree();
}

fn execute(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    command: ClientCommand,
) -> Result<(), String> {
    match command {
        ClientCommand::HostAdd { name, destination } => {
            hosts::add(state, runtime, name, destination)?;
        }
        ClientCommand::HostImport => hosts::import(state, runtime)?,
        ClientCommand::HostConnect { name } => hosts::connect(state, runtime, &name)?,
        ClientCommand::HostDisconnect { name } => hosts::disconnect(state, runtime, &name)?,
        ClientCommand::WorkspaceRegister { path } => {
            workspaces::register(state, runtime, &path)?;
        }
        ClientCommand::WorkspaceSwitch { selector } => {
            workspaces::switch(state, runtime, &selector)?;
        }
        ClientCommand::WorkspaceUngroup => workspaces::ungroup(state, runtime)?,
        ClientCommand::WorkspaceTerminate => workspaces::terminate(state, runtime)?,
        ClientCommand::WorktreeList => worktrees::list(state, runtime)?,
        ClientCommand::WorktreeCreate { branch, base } => {
            worktrees::create(state, runtime, branch, base)?;
        }
        ClientCommand::WorktreeOpen { selector } => {
            worktrees::open(state, runtime, selector)?;
        }
        ClientCommand::WorktreeRemove => worktrees::remove(state, runtime)?,
        ClientCommand::Ports { all_host } => ports::list(state, runtime, all_host)?,
        ClientCommand::Forward {
            remote_port,
            bind_address,
        } => ports::forward(state, runtime, remote_port, bind_address)?,
        ClientCommand::ForwardCancel {
            remote_port,
            bind_address,
        } => ports::cancel(state, runtime, remote_port, bind_address)?,
        ClientCommand::StatusExplain => agents::explain(state, runtime)?,
        ClientCommand::Approve => worktrees::approve(state, runtime)?,
        ClientCommand::Refresh => {
            let remote_count = state
                .connections
                .values()
                .filter(|connection| matches!(connection, HostConnection::Connected))
                .count();
            state
                .event_tx
                .send(super::ClientEvent::ManualRefreshRequested)
                .map_err(|_| "The background refresh queue is unavailable.".to_owned())?;
            if remote_count == 0 {
                state.set_output("Refreshed 0 connected remote host(s).");
            } else {
                state.set_output(
                    "Refresh requested in the background; host results update as each finishes.",
                );
            }
        }
        ClientCommand::Help => show_help(state),
        ClientCommand::Quit => state.should_quit = true,
        ClientCommand::AgentSpawn { provider } => agents::spawn(state, runtime, provider)?,
        ClientCommand::ServiceStart { name } => {
            agents::start_service(state, runtime, &name)?;
        }
    }
    Ok(())
}

fn show_help(state: &mut ClientState) {
    let mut sections = COMMAND_HELP
        .iter()
        .map(|(command, description)| format!("{command}\n  {description}"))
        .collect::<Vec<_>>();
    sections.push(format!(
        "{}\n  Toggle Work/Manage mode\n\n{}\n  Attach the next workspace\n\n{}\n  Open the workspace list",
        state.config.keymap.toggle_mode,
        state.config.keymap.switch_workspace,
        state.config.keymap.workspace_overlay,
    ));
    state.set_detail("Command reference", sections.join("\n\n"));
    state.set_output("Command reference open. Use ↑/↓ or Page Up/Down; Esc closes it.");
}

pub(super) fn selected_workspace(state: &ClientState) -> Result<crate::core::WorkspaceId, String> {
    state
        .selected_workspace
        .ok_or_else(|| "Select a workspace first.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::RegistrySnapshot;
    use std::sync::mpsc;

    #[test]
    fn refresh_with_no_remote_hosts_completes_immediately_without_blocking_local_polling() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = ClientRuntime::test_fixture(root.path());
        let (event_tx, event_rx) = mpsc::channel();
        let mut state = ClientState::new(
            crate::client_config::load_contents(None, None, None).unwrap(),
            RegistrySnapshot {
                hosts: runtime.snapshot().unwrap().hosts,
                ..RegistrySnapshot::default()
            },
            event_tx,
        );
        state
            .connections
            .insert(runtime.local_host_id(), HostConnection::Local);

        execute_command(&mut state, &mut runtime, ClientCommand::Refresh);

        assert_eq!(
            state.output.as_deref(),
            Some("Refreshed 0 connected remote host(s).")
        );
        assert!(matches!(
            event_rx.try_recv(),
            Ok(super::super::ClientEvent::ManualRefreshRequested)
        ));
    }
}
