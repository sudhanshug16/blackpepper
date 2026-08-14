use super::ClientRuntime;
use crate::agent_status::IntegrationHealth;
use crate::core::AgentRunId;
use crate::providers::runtime::{ProviderLaunch, INTEGRATION_HEALTH_TIMEOUT_SECS};
use crate::transport::HostCommand;
use crate::zellij::ZellijRuntime;
use std::time::{Duration, Instant};

pub(super) struct VerifiedAgentTab<'a> {
    pub tab_id: u64,
    pub tab_name: &'a str,
    pub pane_selector: &'a str,
    pub launch_marker: &'a str,
}

impl ClientRuntime {
    pub(super) fn cleanup_agent_tab(
        &mut self,
        zellij: &ZellijRuntime,
        host_id: crate::core::HostId,
        session: &str,
        tab: Option<&VerifiedAgentTab<'_>>,
    ) -> String {
        let Some(tab) = tab else {
            return " The tab was left untouched because its launch identity was not verified."
                .to_string();
        };
        match self.transport_mut(host_id).and_then(|transport| {
            zellij
                .close_tab_if_pane_matches(
                    transport,
                    session,
                    tab.tab_id,
                    tab.tab_name,
                    tab.pane_selector,
                    tab.launch_marker,
                )
                .map_err(|error| error.to_string())
        }) {
            Ok(true) => " The failed background tab was closed.".to_string(),
            Ok(false) => {
                " The background tab was left open because its launch identity changed.".to_string()
            }
            Err(error) => format!(
                " The background tab was left open because safe cleanup was unavailable: {error}."
            ),
        }
    }

    pub(super) fn preflight_integration(
        &mut self,
        host_id: crate::core::HostId,
        workspace_root: &str,
        launch: &ProviderLaunch,
    ) -> Result<(), String> {
        let Some(args) = launch.preflight_args() else {
            return Ok(());
        };
        let mut command = HostCommand::new(&launch.program)
            .args(args)
            .cwd(workspace_root);
        for (key, value) in &launch.env {
            command = command.env(key, value);
        }
        let output = self
            .transport_mut(host_id)?
            .exec(&command)
            .map_err(|error| error.to_string())?;
        if output.success {
            Ok(())
        } else {
            Err(format!(
                "{} could not validate Blackpepper's launch-scoped integration. Upgrade or repair the provider installation and retry; no agent was started.",
                launch.provider.display_name()
            ))
        }
    }

    pub(super) fn wait_for_integration_health(
        &mut self,
        host_id: crate::core::HostId,
        run_id: AgentRunId,
        tab_id: u64,
        launch: &ProviderLaunch,
    ) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(INTEGRATION_HEALTH_TIMEOUT_SECS);
        loop {
            let snapshot = self.agent_snapshot(host_id, run_id).map_err(|error| {
                format!(
                    "Could not verify {} integration health for Zellij tab {tab_id}: {error}",
                    launch.provider.display_name()
                )
            })?;
            if let Some(snapshot) = snapshot {
                match snapshot.snapshot.integration_health {
                    IntegrationHealth::Healthy { .. } => return Ok(()),
                    IntegrationHealth::Degraded { issue } => {
                        return Err(format!(
                            "{} integration reported {issue:?} in Zellij tab {tab_id}; inspect the provider's setup message, close the tab, and retry.",
                            launch.provider.display_name()
                        ));
                    }
                    IntegrationHealth::Unknown
                    | IntegrationHealth::NotInstalled
                    | IntegrationHealth::Starting
                    | IntegrationHealth::Stale => {}
                }
            }
            if Instant::now() >= deadline {
                return Err(launch.handshake_error(tab_id));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
