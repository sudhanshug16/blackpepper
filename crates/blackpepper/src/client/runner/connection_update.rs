//! Applies SSH connection state without leaking authentication transcript data.

use super::super::{ClientMode, ClientState, HostConnection};
use super::periodic;
use super::{ConnectionRestoreReport, ConnectionUpdate};

const HOST_KEY_CHANGED_MESSAGE: &str =
    "SSH host key changed. Verify the host before updating known_hosts and reconnecting.";

pub(super) fn apply(state: &mut ClientState, update: ConnectionUpdate) {
    match update {
        ConnectionUpdate::Ready { previous, host_id } => {
            state.connections.remove(&previous);
            state
                .connections
                .insert(host_id, HostConnection::Reconnecting);
            state.selected_host = Some(host_id);
            state.authentication_host = None;
            state.authentication_output.clear();
            state.mode = ClientMode::Manage;
            for forward in state
                .forwards
                .iter_mut()
                .filter(|forward| forward.host_id == host_id)
            {
                forward.mark_reconnecting();
            }
            let workspace_ids = state
                .agent_runs
                .keys()
                .copied()
                .filter(|workspace_id| state.host_for_workspace(*workspace_id) == Some(host_id))
                .collect::<Vec<_>>();
            for workspace_id in workspace_ids {
                if let Some(runs) = state.agent_runs.get_mut(&workspace_id) {
                    for run in runs {
                        run.mark_snapshot_error("SSH status recovery is still running.".to_owned());
                    }
                }
                state.refresh_workspace_status(workspace_id);
            }
            state.rebuild_tree();
            state.set_output("SSH connected; restoring workspaces, agent status, and tunnels…");
        }
        ConnectionUpdate::Failed { host_id, message } => {
            for forward in state
                .forwards
                .iter_mut()
                .filter(|forward| forward.host_id == host_id)
            {
                forward.mark_reconnecting();
            }
            let (connection, message) = classify_failure(&state.authentication_output, message);
            state.connections.insert(host_id, connection);
            state.authentication_host = None;
            // The transcript can contain host-key material or authentication prompts. Once the
            // failure is classified, retain only the stable, actionable summary shown below.
            state.authentication_output.clear();
            state.mode = ClientMode::Manage;
            state.set_output(message);
        }
    }
}

pub(super) fn apply_restored(state: &mut ClientState, report: ConnectionRestoreReport) {
    let host_id = report.host_id;
    let mut errors = report.errors;
    match report.snapshot {
        Ok(snapshot) => state.snapshot = snapshot,
        Err(error) => errors.push(format!("Final registry snapshot: {error}")),
    }
    state.forwards.retain(|forward| forward.host_id != host_id);
    state.forwards.extend(report.forwards);
    if let Some(refresh) = &report.refresh {
        errors.extend(periodic::apply_connection_refresh(state, host_id, refresh));
    } else if !errors.is_empty() {
        periodic::mark_connection_refresh_failed(state, host_id, &errors.join(" | "));
    }
    errors.extend(report.watcher_errors);
    state.rebuild_tree();

    let failed_forwards = state
        .forwards
        .iter()
        .filter(|forward| {
            forward.host_id == host_id
                && !matches!(
                    forward.status,
                    crate::ports::ForwardStatus::Active | crate::ports::ForwardStatus::Direct
                )
        })
        .count();
    if errors.is_empty() && failed_forwards == 0 {
        let restored = report.restored_workspaces.unwrap_or_default();
        state.set_output(format!(
            "SSH connected; restored {restored} workspace shell(s), agent status, and tunnels."
        ));
    } else {
        let mut summary = Vec::new();
        if failed_forwards > 0 {
            summary.push(format!("{failed_forwards} tunnel(s) need attention"));
        }
        if !errors.is_empty() {
            summary.push(format!("{} recovery warning(s)", errors.len()));
            state.set_detail("SSH recovery warnings", errors.join("\n\n"));
        }
        state.set_output(format!("SSH connected with {}.", summary.join(" and ")));
    }
}

fn classify_failure(authentication_output: &[u8], fallback: String) -> (HostConnection, String) {
    let output = String::from_utf8_lossy(authentication_output);
    if output
        .to_ascii_lowercase()
        .contains("remote host identification has changed")
    {
        (
            HostConnection::HostKeyBlocked,
            HOST_KEY_CHANGED_MESSAGE.to_owned(),
        )
    } else {
        (HostConnection::Failed, fallback)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_host_key_is_actionable_without_echoing_key_material() {
        let transcript = b"WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!\r\n\
            ED25519 key fingerprint is SHA256:secret-material\r\n";
        let (connection, message) = classify_failure(
            transcript,
            "SSH control master exited (Some(255)).".to_owned(),
        );

        assert_eq!(connection, HostConnection::HostKeyBlocked);
        assert_eq!(message, HOST_KEY_CHANGED_MESSAGE);
        assert!(!message.contains("secret-material"));
    }

    #[test]
    fn ordinary_failure_preserves_transport_summary() {
        let fallback = "SSH control master exited (Some(255)).";
        let (connection, message) =
            classify_failure(b"Permission denied (publickey).", fallback.to_owned());

        assert_eq!(connection, HostConnection::Failed);
        assert_eq!(message, fallback);
    }
}
