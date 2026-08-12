mod presentation;

use super::super::ClientState;
use crate::client::runtime::{ClientRuntime, HostOperationContext, HostOperationValue};

pub(in crate::client) use presentation::{
    apply_cancelled, apply_forwarded, apply_list, existing_forward_message,
};

pub(super) fn list(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    all_host: bool,
) -> Result<(), String> {
    let host_id = state
        .selected_host
        .or_else(|| {
            state
                .selected_workspace
                .and_then(|id| state.host_for_workspace(id))
        })
        .unwrap_or(runtime.local_host_id());
    let label = if all_host {
        "Discovering all host ports"
    } else {
        "Discovering workspace ports"
    }
    .to_owned();
    let token = runtime.start_host_operation(
        host_id,
        label.clone(),
        HostOperationContext::PortList { host_id, all_host },
        state.event_tx.clone(),
        Box::new(move |runtime| {
            runtime
                .discover_ports(host_id)
                .map(|snapshot| HostOperationValue::Ports { snapshot })
        }),
    )?;
    state
        .host_operations
        .insert(host_id, (token, label.clone()));
    state.set_output(format!("{label}… Press Esc in Manage mode to cancel."));
    Ok(())
}

pub(super) fn forward(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    remote_port: u16,
    bind_address: Option<String>,
) -> Result<(), String> {
    let workspace_id = super::selected_workspace(state)?;
    let target = resolve_command_target(state, workspace_id, remote_port, bind_address.as_deref())?;
    if let Some(existing) = state
        .forwards
        .iter()
        .find(|forward| forward.workspace_id == workspace_id && forward.target() == target)
    {
        return Err(existing_forward_message(existing));
    }
    start_forward_target(state, runtime, workspace_id, target)
}

pub(in crate::client) fn start_forward_target(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    workspace_id: crate::core::WorkspaceId,
    target: crate::ports::RemotePortTarget,
) -> Result<(), String> {
    let host_id = state
        .host_for_workspace(workspace_id)
        .ok_or_else(|| "The selected workspace host is unavailable.".to_owned())?;
    let label = format!("Forwarding {}", target.endpoint());
    let token = runtime.start_host_operation(
        host_id,
        label.clone(),
        HostOperationContext::ForwardStart { workspace_id },
        state.event_tx.clone(),
        Box::new(move |runtime| {
            runtime
                .forward_workspace_port(workspace_id, target)
                .map(HostOperationValue::Forwarded)
        }),
    )?;
    state
        .host_operations
        .insert(host_id, (token, label.clone()));
    state.set_output(format!("{label}… Press Esc in Manage mode to cancel."));
    Ok(())
}

pub(super) fn cancel(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    remote_port: u16,
    bind_address: Option<String>,
) -> Result<(), String> {
    let workspace_id = super::selected_workspace(state)?;
    let requested_target = bind_address
        .as_deref()
        .map(|address| crate::ports::RemotePortTarget::from_bind_address(address, remote_port))
        .transpose()?;
    let matches = state
        .forwards
        .iter()
        .enumerate()
        .filter(|(_, forward)| {
            forward.workspace_id == workspace_id
                && forward.remote_port == remote_port
                && requested_target
                    .as_ref()
                    .is_none_or(|target| forward.target() == *target)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let index = match matches.as_slice() {
        [index] => *index,
        [] => return Err(format!("No matching forward uses port {remote_port}.")),
        _ => {
            return Err(format!(
                "Several forwards use port {remote_port}; cancel one with :forward cancel <address>:{remote_port}."
            ))
        }
    };
    let forward = state.forwards[index].clone();
    if forward.status == crate::ports::ForwardStatus::Cancelling {
        return Err(existing_forward_message(&forward));
    }
    let host_id = forward.host_id;
    let forward_id = forward.id;
    let label = format!("Cancelling forward {}", forward.remote_endpoint());
    let worker_forward = forward.clone();
    let token = runtime.start_host_operation(
        host_id,
        label.clone(),
        HostOperationContext::ForwardCancel {
            workspace_id,
            forward_id,
        },
        state.event_tx.clone(),
        Box::new(move |runtime| {
            runtime
                .cancel_workspace_forward(workspace_id, &worker_forward)
                .map(|()| HostOperationValue::ForwardCancelled(worker_forward))
        }),
    )?;
    state.forwards[index].status = crate::ports::ForwardStatus::Cancelling;
    state
        .host_operations
        .insert(host_id, (token, label.clone()));
    state.set_output(format!("{label}… Press Esc in Manage mode to cancel."));
    Ok(())
}

fn resolve_command_target(
    state: &ClientState,
    workspace_id: crate::core::WorkspaceId,
    remote_port: u16,
    bind_address: Option<&str>,
) -> Result<crate::ports::RemotePortTarget, String> {
    let workspace = state
        .snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.id == workspace_id)
        .ok_or_else(|| "The selected workspace no longer exists.".to_string())?;
    let snapshot = state.ports.get(&workspace.host_id).ok_or_else(|| {
        "No port discovery result is available. Run :ports first (or :ports --all-host for unattributed services)."
            .to_string()
    })?;
    let visible = snapshot.listeners.iter().filter(|listener| {
        state.show_all_host_ports
            || crate::client::runtime::ports::listener_matches_workspace(
                listener.workspace_path.as_deref(),
                &workspace.root_path,
            )
    });
    let target = crate::ports::resolve_forward_target(visible, remote_port, bind_address)?;
    if crate::ports::target_is_ambiguous(&snapshot.listeners, &target) {
        return Err(format!(
            "Multiple processes can accept {}; TCP forwarding cannot select one process. Stop the overlapping listener or use another port.",
            target.endpoint()
        ));
    }
    Ok(target)
}
