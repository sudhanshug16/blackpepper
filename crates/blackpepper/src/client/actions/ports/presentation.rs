use crate::client::ClientState;

pub(in crate::client) fn apply_list(
    state: &mut ClientState,
    host_id: crate::core::HostId,
    all_host: bool,
    snapshot: crate::ports::PortSnapshot,
) {
    let workspace_root = state.selected_workspace.and_then(|workspace_id| {
        state
            .snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .map(|workspace| workspace.root_path.as_str())
    });
    let visible = snapshot
        .listeners
        .iter()
        .filter(|listener| {
            all_host
                || workspace_root.is_some_and(|root| {
                    crate::client::runtime::ports::listener_matches_workspace(
                        listener.workspace_path.as_deref(),
                        root,
                    )
                })
        })
        .map(|listener| {
            let ambiguous = listener.forward_target().is_ok_and(|target| {
                crate::ports::target_is_ambiguous(&snapshot.listeners, &target)
            });
            format!(
                "{}  {}{}",
                listener.bind_endpoint(),
                listener.process.as_deref().unwrap_or("unknown process"),
                if ambiguous {
                    " — shared/overlapping; forwarding disabled"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>();
    let count = visible.len();
    state.ports.insert(host_id, snapshot);
    state.show_all_host_ports = all_host;
    state.ports_scroll = 0;
    if !visible.is_empty() {
        state.set_detail("Listening ports", visible.join("\n"));
    }
    state.set_output(format!(
        "Found {count} listening port(s){}{}.",
        if all_host {
            " on this host"
        } else {
            " for workspace attribution"
        },
        if count == 0 {
            ""
        } else {
            "; Esc closes the scrollable list"
        },
    ));
}

pub(in crate::client) fn apply_forwarded(
    state: &mut ClientState,
    forward: crate::ports::ForwardState,
) {
    let local = forward.local_address;
    let remote = forward.remote_endpoint();
    let message = if forward.status == crate::ports::ForwardStatus::Direct {
        format!("Local service is already available at http://{local}; no tunnel was created for {remote}.")
    } else {
        format!("Forward active: http://{local} → remote {remote}.")
    };
    state.forwards.push(forward);
    state.set_output(message);
}

pub(in crate::client) fn apply_cancelled(
    state: &mut ClientState,
    workspace_id: crate::core::WorkspaceId,
    forward_id: uuid::Uuid,
    forward: crate::ports::ForwardState,
) {
    state
        .forwards
        .retain(|candidate| candidate.id != forward_id);
    state.set_output(if forward.status == crate::ports::ForwardStatus::Direct {
        format!(
            "Removed the local URL shortcut for {}; the service itself is still listening.",
            forward.remote_endpoint()
        )
    } else {
        format!(
            "Cancelled forward for remote {}.",
            forward.remote_endpoint()
        )
    });
    debug_assert_eq!(workspace_id, forward.workspace_id);
}

pub(in crate::client) fn existing_forward_message(forward: &crate::ports::ForwardState) -> String {
    let endpoint = forward.remote_endpoint();
    match &forward.status {
        crate::ports::ForwardStatus::Direct => format!("Local service URL: http://{} (no tunnel is needed).", forward.local_address),
        crate::ports::ForwardStatus::Active => format!("Forward already active: http://{} → remote {endpoint}.", forward.local_address),
        crate::ports::ForwardStatus::Reconnecting => format!("Forward for {endpoint} is reconnecting; wait, or cancel it before retrying."),
        crate::ports::ForwardStatus::Cancelling => format!("Tunnel cleanup for {endpoint} is already running in the background."),
        crate::ports::ForwardStatus::PortConflict => format!("Forward for {endpoint} has a local port conflict at {}; free that port or cancel the forward before retrying.", forward.local_address),
        crate::ports::ForwardStatus::Failed(reason) => {
            let reason = reason.chars().take(240).collect::<String>();
            format!("Forward for {endpoint} failed: {reason}. Cancel it before retrying.")
        }
    }
}
