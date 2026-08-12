use super::{connection, ClientRuntime, HostSlot};
use crate::client::ClientEvent;
use crate::core::{
    HelperRequest, HostId, HostPeriodicRefresh, RequestOperation, ResponsePayload, ResponseResult,
    PROTOCOL_VERSION,
};
use crate::transport::{HostCommand, HostTransport, RunningCommand};
use std::io::Write;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

const PERIODIC_REFRESH_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) struct PeriodicRefreshJob {
    pub host_id: HostId,
    child: RunningCommand,
}

impl PeriodicRefreshJob {
    pub fn wait_cancellable(
        self,
        cancellation: Receiver<()>,
    ) -> Result<HostPeriodicRefresh, String> {
        self.wait_until(|| {
            matches!(
                cancellation.try_recv(),
                Ok(()) | Err(TryRecvError::Disconnected)
            )
        })
    }

    pub(crate) fn wait_with_token(
        self,
        cancellation: &crate::transport::CommandCancellation,
    ) -> Result<HostPeriodicRefresh, String> {
        self.wait_until(|| cancellation.is_cancelled())
    }

    fn wait_until(
        mut self,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<HostPeriodicRefresh, String> {
        const POLL_INTERVAL: Duration = Duration::from_millis(10);
        let started = Instant::now();
        loop {
            if self
                .child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                break;
            }
            if cancelled() {
                let _ = self.child.cancel();
                return Err("Periodic refresh was cancelled during shutdown.".to_owned());
            }
            if started.elapsed() >= PERIODIC_REFRESH_TIMEOUT {
                let process_id = self.child.id().unwrap_or_default();
                let cancellation_error = self.child.cancel().err().map(|error| error.to_string());
                let suffix = cancellation_error
                    .map(|error| format!("; cancellation also failed: {error}"))
                    .unwrap_or_default();
                return Err(format!(
                    "periodic refresh process {process_id} exceeded its {}ms deadline{suffix}",
                    PERIODIC_REFRESH_TIMEOUT.as_millis()
                ));
            }
            std::thread::sleep(
                POLL_INTERVAL.min(PERIODIC_REFRESH_TIMEOUT.saturating_sub(started.elapsed())),
            );
        }
        let host_id = self.host_id;
        let output = self
            .child
            .wait_with_output()
            .map_err(|error| error.to_string())?;
        Self::parse_output(host_id, output)
    }

    fn parse_output(
        host_id: HostId,
        output: crate::transport::CommandOutput,
    ) -> Result<HostPeriodicRefresh, String> {
        if !output.success {
            return Err(format!(
                "bp-host periodic refresh failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let response = stdout
            .lines()
            .last()
            .ok_or_else(|| "bp-host returned no periodic refresh response.".to_owned())?;
        let response: crate::core::HelperResponse = serde_json::from_str(response)
            .map_err(|error| format!("bp-host returned invalid refresh JSON: {error}"))?;
        match response.result {
            ResponseResult::Ok {
                payload: ResponsePayload::HostService { payload },
            } => match *payload {
                crate::core::HostServicePayload::PeriodicRefresh { refresh }
                    if refresh.host_id == host_id =>
                {
                    Ok(*refresh)
                }
                crate::core::HostServicePayload::PeriodicRefresh { .. } => {
                    Err("bp-host periodic refresh changed host identity.".to_owned())
                }
                _ => Err("bp-host returned an unexpected periodic refresh payload.".to_owned()),
            },
            ResponseResult::Ok { .. } => {
                Err("bp-host returned an unexpected periodic refresh response.".to_owned())
            }
            ResponseResult::Error { error } => Err(error.message),
        }
    }
}

impl ClientRuntime {
    pub(super) fn spawn_fail_closed_background_exec_with_stdin(
        &mut self,
        host_id: HostId,
        command: &HostCommand,
    ) -> Result<RunningCommand, String> {
        match self.hosts.get_mut(&host_id) {
            Some(HostSlot::Local(transport)) => transport.spawn_exec_with_stdin(command),
            Some(HostSlot::Ssh(host)) => host.transport.spawn_background_exec_with_stdin(command),
            None => return Err(format!("Host {host_id} is not connected.")),
        }
        .map_err(|error| error.to_string())
    }

    pub(crate) fn start_periodic_refresh(
        &mut self,
        host_id: HostId,
        attached_workspaces: Vec<crate::core::WorkspaceId>,
    ) -> Result<PeriodicRefreshJob, String> {
        let helper = self.helper_path(host_id)?;
        let operations = [
            RequestOperation::Handshake {
                client_version: crate::BUILD_ID.to_owned(),
            },
            RequestOperation::PeriodicRefresh {
                attached_workspaces,
            },
        ];
        let mut requests = Vec::new();
        for (index, operation) in operations.into_iter().enumerate() {
            serde_json::to_writer(
                &mut requests,
                &HelperRequest {
                    request_id: index as u64 + 1,
                    protocol_version: PROTOCOL_VERSION,
                    operation,
                },
            )
            .map_err(|error| error.to_string())?;
            requests.push(b'\n');
        }
        let command = HostCommand::new(helper);
        let mut child = match self.hosts.get_mut(&host_id) {
            Some(HostSlot::Local(_)) => crate::transport::LocalTransport::process_spec(&command)
                .and_then(|spec| RunningCommand::spawn_in_process_group(&spec, true)),
            Some(HostSlot::Ssh(host)) => host.transport.spawn_background_exec_with_stdin(&command),
            None => return Err(format!("Host {host_id} is not connected.")),
        }
        .map_err(|error| error.to_string())?;
        let mut stdin = child
            .take_stdin()
            .ok_or_else(|| "bp-host periodic refresh stdin was unavailable.".to_owned())?;
        stdin
            .write_all(&requests)
            .map_err(|error| format!("Could not dispatch periodic refresh: {error}"))?;
        drop(stdin);
        Ok(PeriodicRefreshJob { host_id, child })
    }

    /// Merge the host registry only after a complete, identity-checked helper
    /// response. The potentially slow observation work has already finished
    /// on the background thread; this step is local SQLite metadata only.
    pub(crate) fn apply_periodic_registry(
        &mut self,
        refresh: &HostPeriodicRefresh,
    ) -> Result<crate::core::RegistrySnapshot, String> {
        if refresh.host_id != self.local_host_id {
            connection::reconcile_remote_snapshot(self, refresh.host_id, &refresh.registry)?;
        }
        self.snapshot()
    }

    /// Start only missing launch-scoped blocker streams. Starting an exec
    /// channel does not wait for remote output; all Zellij/status observation
    /// that can stall is part of the completed background refresh.
    pub(crate) fn ensure_periodic_blocker_watchers(
        &mut self,
        refresh: &HostPeriodicRefresh,
        sender: Sender<ClientEvent>,
    ) -> Vec<String> {
        // Restore workers prune exited watchers before handing ownership back
        // to the UI runtime. This method only starts missing streams, which is
        // non-waiting and safe after the connection generation is merged.
        self.prune_periodic_blocker_watchers(refresh);
        let watchable = refresh
            .watchable_agent_runs
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let mut errors = Vec::new();
        for run in refresh
            .agent_runs
            .iter()
            .filter(|run| watchable.contains(&run.run_id))
        {
            if self.blocker_watchers.contains_key(&run.run_id) {
                continue;
            }
            if let Err(error) = self.start_blocker_watcher(
                run.host_id,
                run.workspace_id,
                run.run_id,
                run.pane_id,
                run.provider,
                &run.binding.session_name,
                &run.binding.zellij_version,
                &run.binding.zellij_pane_id,
                0,
                sender.clone(),
            ) {
                errors.push(format!("run {} blocker watcher: {error}", run.run_id));
            }
        }
        errors
    }

    pub(crate) fn prune_periodic_blocker_watchers(&mut self, refresh: &HostPeriodicRefresh) {
        for (run_id, snapshot) in &refresh.agent_snapshots {
            if snapshot.snapshot.state == crate::agent_status::AgentState::Exited {
                self.stop_blocker_watcher(*run_id);
            }
        }
    }
}
