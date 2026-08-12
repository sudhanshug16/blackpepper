use super::super::{control::attach_selected, ClientMode, ClientState};
use crate::client::runtime::{ClientRuntime, HostOperationContext, HostOperationValue};
use std::path::Path;

pub(super) fn register(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    path: &Path,
) -> Result<(), String> {
    let host_id = state.selected_host.unwrap_or(runtime.local_host_id());
    state.selected_host = Some(host_id);
    let path = path.to_path_buf();
    let worker_path = path.clone();
    let label = format!("Registering and preparing {}", path.display());
    let token = runtime.start_host_operation(
        host_id,
        label.clone(),
        HostOperationContext::RegisterAndAttach {
            host_id,
            path: path.clone(),
        },
        state.event_tx.clone(),
        Box::new(move |runtime| {
            let workspace_id = runtime.register_workspace(host_id, &worker_path)?;
            let attachment = runtime.attach_workspace(workspace_id, 24, 80);
            Ok(HostOperationValue::RegisteredAndAttached {
                workspace_id,
                path: worker_path,
                attachment,
            })
        }),
    )?;
    state
        .host_operations
        .insert(host_id, (token, label.clone()));
    state.set_output(format!(
        "{label}… Repository detection, managed sidecars, and startup services run in the background."
    ));
    Ok(())
}

pub(super) fn switch(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    selector: &str,
) -> Result<(), String> {
    let workspace = runtime.find_workspace(selector)?;
    state.selected_workspace = Some(workspace.id);
    state.selected_host = Some(workspace.host_id);
    attach_selected(state, runtime);
    Ok(())
}

pub(super) fn ungroup(state: &mut ClientState, runtime: &mut ClientRuntime) -> Result<(), String> {
    let workspace_id = super::selected_workspace(state)?;
    schedule_ungroup(state, runtime, workspace_id, move |runtime| {
        runtime.ungroup_workspace(workspace_id)
    })
}

fn schedule_ungroup<F>(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    workspace_id: crate::core::WorkspaceId,
    work: F,
) -> Result<(), String>
where
    F: FnOnce(&mut ClientRuntime) -> Result<crate::core::WorkspaceRecord, String> + Send + 'static,
{
    let host_id = state
        .host_for_workspace(workspace_id)
        .ok_or_else(|| "The selected workspace host is unavailable.".to_owned())?;
    let label = "Ungrouping workspace".to_owned();
    let token = runtime.start_host_operation(
        host_id,
        label.clone(),
        HostOperationContext::WorkspaceUngroup { workspace_id },
        state.event_tx.clone(),
        Box::new(move |runtime| work(runtime).map(HostOperationValue::WorkspaceUngrouped)),
    )?;
    state
        .host_operations
        .insert(host_id, (token, label.clone()));
    state.set_output(format!("{label}… Press Esc in Manage mode to cancel."));
    Ok(())
}

pub(in crate::client) fn apply_ungrouped_workspace(
    state: &mut ClientState,
    workspace: crate::core::WorkspaceRecord,
) {
    if let Some(existing) = state
        .snapshot
        .workspaces
        .iter_mut()
        .find(|existing| existing.id == workspace.id)
    {
        *existing = workspace;
    }
    state.set_output("Workspace will remain outside automatic repository grouping.");
}

pub(super) fn terminate(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
) -> Result<(), String> {
    let workspace_id = super::selected_workspace(state)?;
    let host_id = state
        .host_for_workspace(workspace_id)
        .ok_or_else(|| "The selected workspace host is unavailable.".to_owned())?;
    let label = "Terminating workspace session".to_owned();
    let token = runtime.start_host_operation(
        host_id,
        label.clone(),
        HostOperationContext::Terminate { workspace_id },
        state.event_tx.clone(),
        Box::new(move |runtime| {
            runtime
                .terminate_workspace(workspace_id)
                .map(|()| HostOperationValue::Terminated)
        }),
    )?;
    state
        .host_operations
        .insert(host_id, (token, label.clone()));
    state.terminals.remove(&workspace_id);
    state.connected_clients.remove(&workspace_id);
    if state.active_workspace == Some(workspace_id) {
        state.active_workspace = None;
        state.mode = ClientMode::Manage;
    }
    state.set_output(format!(
        "{label}… The folder is being kept; press Esc in Manage mode to cancel."
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::HostConnection;
    use crate::core::{
        GroupingPolicy, HostRecord, HostTransport, RegistrySnapshot, RepositoryIdentity,
        WorkspaceRecord,
    };
    use ratatui::{backend::TestBackend, Terminal};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn completion(
        receiver: &mpsc::Receiver<crate::client::ClientEvent>,
    ) -> (uuid::Uuid, crate::core::HostId, u64) {
        loop {
            match receiver.recv_timeout(Duration::from_secs(3)).unwrap() {
                crate::client::ClientEvent::HostOperationComplete {
                    token,
                    host_id,
                    generation,
                } => return (token, host_id, generation),
                _ => continue,
            }
        }
    }

    #[test]
    fn ungroup_result_updates_snapshot_and_folder_tree_without_another_registry_read() {
        let host = HostRecord::new("local", HostTransport::Local);
        let mut workspace = WorkspaceRecord::new(host.id, "/repo/feature");
        workspace.repository =
            Some(RepositoryIdentity::remote("https://example.invalid/acme/project.git").unwrap());
        let (event_tx, _event_rx) = mpsc::channel();
        let mut state = ClientState::new(
            crate::client_config::load_contents(None, None, None).unwrap(),
            RegistrySnapshot {
                hosts: vec![host.clone()],
                workspaces: vec![workspace.clone()],
                sessions: Vec::new(),
                pending_worktree_removals: Vec::new(),
            },
            event_tx,
        );
        state.connections.insert(host.id, HostConnection::Local);
        workspace.grouping = GroupingPolicy::Ungrouped;

        apply_ungrouped_workspace(&mut state, workspace.clone());
        state.rebuild_tree();

        assert_eq!(state.snapshot.workspaces[0], workspace);
        assert_eq!(state.tree[0].repositories[0].label, "folder");
    }

    #[test]
    fn ungroup_applies_authoritative_record_and_worker_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let workspace_root = root.path().join("workspace");
        std::fs::create_dir(&workspace_root).unwrap();
        let mut runtime = ClientRuntime::test_fixture(root.path());
        let host_id = runtime.local_host_id();
        let workspace_id = runtime
            .register_workspace(host_id, &workspace_root)
            .unwrap();
        let (event_tx, event_rx) = mpsc::channel();
        let mut state = ClientState::new(
            crate::client_config::load_contents(None, None, None).unwrap(),
            runtime.snapshot().unwrap(),
            event_tx,
        );
        state.connections.insert(host_id, HostConnection::Local);
        state.selected_workspace = Some(workspace_id);

        ungroup(&mut state, &mut runtime).unwrap();

        assert!(runtime.host_operation_active(host_id));
        assert!(state
            .output
            .as_deref()
            .is_some_and(|message| message.starts_with("Ungrouping workspace…")));
        let (token, completed_host, generation) = completion(&event_rx);
        crate::client::runner::operations::complete(
            &mut state,
            &mut runtime,
            token,
            completed_host,
            generation,
        );

        let workspace = state
            .snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .unwrap();
        assert_eq!(workspace.grouping, GroupingPolicy::Ungrouped);
        assert_eq!(
            runtime.snapshot().unwrap().workspaces,
            state.snapshot.workspaces
        );
        assert_eq!(
            state.output.as_deref(),
            Some("Workspace will remain outside automatic repository grouping.")
        );
    }

    #[test]
    fn stalled_remote_ungroup_keeps_render_responsive_and_disconnect_discards_result() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = ClientRuntime::test_fixture(root.path());
        let host_id = runtime.test_add_ssh_slot("remote", "remote.example");
        let host = runtime.host_record(host_id).unwrap();
        let workspace = WorkspaceRecord::new(host_id, "/srv/project");
        let workspace_id = workspace.id;
        let (event_tx, event_rx) = mpsc::channel();
        let mut state = ClientState::new(
            crate::client_config::load_contents(None, None, None).unwrap(),
            RegistrySnapshot {
                hosts: vec![host],
                workspaces: vec![workspace.clone()],
                sessions: Vec::new(),
                pending_worktree_removals: Vec::new(),
            },
            event_tx,
        );
        state.connections.insert(host_id, HostConnection::Connected);
        state.selected_workspace = Some(workspace_id);
        let (entered_tx, entered_rx) = mpsc::channel();
        let mut returned = workspace.clone();
        returned.grouping = GroupingPolicy::Ungrouped;

        let started = Instant::now();
        schedule_ungroup(&mut state, &mut runtime, workspace_id, move |_| {
            entered_tx.send(()).unwrap();
            while !crate::transport::CommandCancellation::scope_is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            // A remote helper can finish after the user disconnects. The
            // generation/discard gate must reject even a successful reply.
            Ok(returned)
        })
        .unwrap();
        assert!(started.elapsed() < Duration::from_millis(250));
        entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let rendered = Instant::now();
        terminal
            .draw(|frame| crate::client::render(&mut state, frame))
            .unwrap();
        assert!(rendered.elapsed() < Duration::from_millis(250));

        runtime.disconnect_host_with_restores(host_id).unwrap();
        let (token, completed_host, generation) = completion(&event_rx);
        crate::client::runner::operations::complete(
            &mut state,
            &mut runtime,
            token,
            completed_host,
            generation,
        );

        assert_eq!(
            state.snapshot.workspaces[0].grouping,
            GroupingPolicy::Automatic
        );
        assert!(!state.host_operations.contains_key(&host_id));
        assert_ne!(
            state.output.as_deref(),
            Some("Workspace will remain outside automatic repository grouping.")
        );
    }
}
