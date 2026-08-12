use super::{connection, ClientRuntime};
use crate::agent_status::Provider;
use crate::client::ClientEvent;
use crate::core::{
    AgentProcessObservation, AgentRunBinding, AgentRunId, HostAgentRun, HostId, PaneId,
    RequestOperation, ResponsePayload, WorkspaceId,
};
use std::sync::mpsc::Sender;

impl ClientRuntime {
    pub(crate) fn agent_snapshot(
        &mut self,
        host_id: HostId,
        run_id: AgentRunId,
    ) -> Result<Option<crate::core::HostAgentSnapshot>, String> {
        match connection::registry_operation(
            self,
            host_id,
            RequestOperation::AgentSnapshot { run_id },
        )? {
            ResponsePayload::HostService { payload } => match *payload {
                crate::core::HostServicePayload::AgentSnapshot { snapshot } => Ok(snapshot),
                _ => Err("bp-host returned an unexpected agent snapshot response.".to_owned()),
            },
            _ => Err("bp-host returned an unexpected agent snapshot response.".to_owned()),
        }
    }

    /// Rediscovers only bound, active runs and reconciles each descriptor
    /// against its exact Zellij session, tab, and terminal pane. A surviving
    /// process gets a fresh host-local blocker watcher; its provider command is
    /// never relaunched.
    pub(crate) fn rediscover_agent_runs(
        &mut self,
        host_id: HostId,
        sender: Sender<ClientEvent>,
    ) -> Result<Vec<HostAgentRun>, String> {
        let runs = self.list_agent_runs(host_id, None)?;
        let mut live = Vec::with_capacity(runs.len());
        for run in runs {
            let observed = self.observe_zellij_pane(host_id, &run.binding, run.run_id)?;
            let pane_identity_verified = matches!(observed, crate::zellij::PaneProcessState::Live);
            let observation = match observed {
                crate::zellij::PaneProcessState::Live => AgentProcessObservation::Live,
                crate::zellij::PaneProcessState::Exited { code } => {
                    AgentProcessObservation::Exited { exit_code: code }
                }
                crate::zellij::PaneProcessState::Missing => AgentProcessObservation::Missing,
                crate::zellij::PaneProcessState::UnverifiedIdentity { .. } => {
                    AgentProcessObservation::StateUnknown
                }
            };
            let reconciled = self.reconcile_agent_run(host_id, &run, observation)?;
            if reconciled.snapshot.state == crate::agent_status::AgentState::Exited {
                self.stop_blocker_watcher(reconciled.run_id);
                continue;
            }
            if !pane_identity_verified {
                // Screen rules must never inspect a pane that could be a
                // reused numeric ID. Provider hooks can still prove later
                // activity for the launch-scoped run without terminal text.
                self.stop_blocker_watcher(reconciled.run_id);
                live.push(reconciled);
                continue;
            }
            if !self.blocker_watchers.contains_key(&reconciled.run_id) {
                self.start_blocker_watcher(
                    host_id,
                    reconciled.workspace_id,
                    reconciled.run_id,
                    reconciled.pane_id,
                    reconciled.provider,
                    &reconciled.binding.session_name,
                    &reconciled.binding.zellij_version,
                    &reconciled.binding.zellij_pane_id,
                    0,
                    sender.clone(),
                )?;
            }
            live.push(reconciled);
        }
        Ok(live)
    }

    /// A newly created Zellij backend is a new process generation even when
    /// it reuses the persisted session name. End runs from the missing
    /// generation before auto-start services can reuse their numeric pane IDs.
    pub(super) fn end_runs_for_recreated_session(
        &mut self,
        host_id: HostId,
        workspace_id: WorkspaceId,
        session_name: &str,
    ) -> Result<usize, String> {
        let runs = self.list_agent_runs(host_id, Some(workspace_id))?;
        let mut ended = 0;
        let mut errors = Vec::new();
        for run in runs
            .into_iter()
            .filter(|run| run.binding.session_name == session_name)
        {
            self.stop_blocker_watcher(run.run_id);
            match self.reconcile_agent_run(host_id, &run, AgentProcessObservation::Missing) {
                Ok(_) => ended += 1,
                Err(error) => match self.agent_snapshot(host_id, run.run_id) {
                    Ok(Some(snapshot))
                        if snapshot.snapshot.state == crate::agent_status::AgentState::Exited =>
                    {
                        // Reconciliation commits exit/deactivation before
                        // best-effort asset cleanup. A cleanup warning must
                        // not make the replacement generation unsafe.
                        ended += 1;
                    }
                    _ => errors.push(error),
                },
            }
        }
        if errors.is_empty() {
            Ok(ended)
        } else {
            Err(format!(
                "The replacement Zellij session was created, but {} prior agent run(s) could not be retired: {}",
                errors.len(),
                errors.join(" | ")
            ))
        }
    }

    /// Persists the conservative status used after input interruption without
    /// trusting a late provider `Stop` event to mean successful completion.
    pub(crate) fn mark_agent_state_unknown(
        &mut self,
        host_id: HostId,
        run_id: AgentRunId,
    ) -> Result<HostAgentRun, String> {
        let run = self
            .list_agent_runs(host_id, None)?
            .into_iter()
            .find(|run| run.run_id == run_id)
            .ok_or_else(|| "Agent run is not active on this host.".to_owned())?;
        self.reconcile_agent_run(host_id, &run, AgentProcessObservation::StateUnknown)
    }

    fn list_agent_runs(
        &mut self,
        host_id: HostId,
        workspace_id: Option<WorkspaceId>,
    ) -> Result<Vec<HostAgentRun>, String> {
        match connection::registry_operation(
            self,
            host_id,
            RequestOperation::ListAgentRuns { workspace_id },
        )? {
            ResponsePayload::HostService { payload } => match *payload {
                crate::core::HostServicePayload::AgentRuns { runs } => {
                    if runs.iter().any(|run| {
                        run.host_id != host_id
                            || workspace_id.is_some_and(|workspace| run.workspace_id != workspace)
                    }) {
                        return Err(
                            "bp-host returned an agent run from another host or workspace."
                                .to_owned(),
                        );
                    }
                    Ok(runs)
                }
                _ => Err("bp-host returned an unexpected agent-run list response.".to_owned()),
            },
            _ => Err("bp-host returned an unexpected agent-run list response.".to_owned()),
        }
    }

    fn reconcile_agent_run(
        &mut self,
        host_id: HostId,
        run: &HostAgentRun,
        observation: AgentProcessObservation,
    ) -> Result<HostAgentRun, String> {
        let payload = connection::registry_operation(
            self,
            host_id,
            RequestOperation::ReconcileAgentRun {
                workspace_id: run.workspace_id,
                run_id: run.run_id,
                pane_id: run.pane_id,
                provider: run.provider,
                binding: run.binding.clone(),
                observation,
            },
        )?;
        match payload {
            ResponsePayload::HostService { payload } => match *payload {
                crate::core::HostServicePayload::AgentRunReconciled { run: reconciled }
                    if reconciled.host_id == host_id
                        && reconciled.run_id == run.run_id
                        && reconciled.workspace_id == run.workspace_id
                        && reconciled.pane_id == run.pane_id
                        && reconciled.provider == run.provider
                        && reconciled.binding == run.binding =>
                {
                    Ok(*reconciled)
                }
                _ => Err("bp-host returned a mismatched agent reconciliation response.".to_owned()),
            },
            _ => Err("bp-host returned a mismatched agent reconciliation response.".to_owned()),
        }
    }

    pub(super) fn bind_agent_run(
        &mut self,
        host_id: HostId,
        workspace_id: WorkspaceId,
        run_id: AgentRunId,
        pane_id: PaneId,
        provider: Provider,
        binding: AgentRunBinding,
    ) -> Result<(), String> {
        match connection::registry_operation(
            self,
            host_id,
            RequestOperation::BindAgentRun {
                workspace_id,
                run_id,
                pane_id,
                provider,
                binding,
            },
        )? {
            ResponsePayload::Acknowledged => Ok(()),
            _ => Err("bp-host returned an unexpected agent binding response.".to_owned()),
        }
    }

    fn abort_agent_run(
        &mut self,
        host_id: HostId,
        workspace_id: WorkspaceId,
        run_id: AgentRunId,
        pane_id: PaneId,
        provider: Provider,
    ) -> Result<(), String> {
        match connection::registry_operation(
            self,
            host_id,
            RequestOperation::AbortAgentRun {
                workspace_id,
                run_id,
                pane_id,
                provider,
            },
        )? {
            ResponsePayload::Acknowledged => Ok(()),
            _ => Err("bp-host returned an unexpected agent abort response.".to_owned()),
        }
    }

    pub(super) fn abort_note(
        &mut self,
        host_id: HostId,
        workspace_id: WorkspaceId,
        run_id: AgentRunId,
        pane_id: PaneId,
        provider: Provider,
    ) -> String {
        match self.abort_agent_run(host_id, workspace_id, run_id, pane_id, provider) {
            Ok(()) => " The failed status run was deactivated.".to_owned(),
            Err(error) => format!(
                " The failed status run could not be deactivated and will be discarded as stale on a later registry scan: {error}."
            ),
        }
    }
}
