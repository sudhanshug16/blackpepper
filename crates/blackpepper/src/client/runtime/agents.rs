use super::{connection, ClientRuntime};
use crate::agent_status::Provider;
use crate::client::ClientEvent;
use crate::core::{AgentRunBinding, AgentRunId, PaneId, WorkspaceId};
use crate::providers::runtime::{build_launch, ProviderKind, AGENT_RUN_ID_ENV};
use std::path::Path;
use std::sync::mpsc::Sender;

mod command;
mod integration;

#[cfg(test)]
use command::apply_agent_environment_with;
use command::{apply_agent_environment, initial_agent_command};
use integration::VerifiedAgentTab;

#[derive(Debug, Clone)]
pub(crate) struct SpawnedAgent {
    pub run_id: AgentRunId,
    pub pane_id: PaneId,
    pub tab_id: u64,
    pub zellij_pane_id: String,
    pub capability: &'static str,
}

impl ClientRuntime {
    pub(crate) fn spawn_agent(
        &mut self,
        workspace_id: WorkspaceId,
        provider: Provider,
        sender: Sender<ClientEvent>,
    ) -> Result<SpawnedAgent, String> {
        let workspace = self
            .registry
            .workspace(workspace_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "The selected workspace no longer exists.".to_string())?;
        let provider_kind = match provider {
            Provider::Codex => ProviderKind::Codex,
            Provider::Claude => ProviderKind::Claude,
            Provider::OpenCode => ProviderKind::OpenCode,
        };
        let provider_binary = self.command_path(workspace.host_id, provider_kind.command())?;
        if provider_kind == ProviderKind::OpenCode {
            self.reserve_opencode_inline_config(workspace.host_id)?;
        }
        let helper = self.helper_path(workspace.host_id)?;
        let integration_dir = self.integration_dir(workspace.host_id)?;
        let run_id = AgentRunId::new();
        let pane_id = PaneId::new();
        let mut launch = build_launch(
            provider_kind,
            workspace.id,
            run_id,
            pane_id,
            Path::new(&helper),
            &integration_dir,
        )?;
        launch.program = provider_binary;
        if let Err(error) = self.install_assets(workspace.host_id, &launch.assets) {
            let cleanup = self.cleanup_assets_note(workspace.host_id, &launch.assets);
            return Err(format!("{error}{cleanup}"));
        }

        // Hold the same host lifecycle gate used by service creation and
        // termination from the authoritative workspace refresh through the
        // exact pane binding. A second laptop cannot kill this session between
        // the tab creation and its durable status registration.
        let (lease, workspace) = match self.acquire_workspace_session_lease(workspace.id) {
            Ok(value) => value,
            Err(error) => {
                let cleanup = self.cleanup_assets_note(workspace.host_id, &launch.assets);
                return Err(format!("{error}{cleanup}"));
            }
        };
        // Project environment applies to agents and their preflight just like
        // configured services. Launch-scoped integration IDs/config always
        // win so project values cannot redirect status events across runs.
        let workspace_env = match self.workspace_config(&workspace) {
            Ok(config) => config.workspace_env,
            Err(error) => {
                let cleanup = self.cleanup_assets_note(workspace.host_id, &launch.assets);
                return Err(format!("{error}{cleanup}"));
            }
        };
        // Stock Zellij drops the notification protocol selected by terminal
        // identity, so expose it only when this workspace's recorded runtime
        // has the matching transport patch. This also keeps old sessions safe
        // after Blackpepper activates a newer runtime for new sessions.
        let session_generation = match self.current_or_new_session(&workspace) {
            Ok(session) => session.backend_version,
            Err(error) => {
                let cleanup = self.cleanup_assets_note(workspace.host_id, &launch.assets);
                return Err(format!("{error}{cleanup}"));
            }
        };
        let terminal_identity_supported =
            crate::transport::is_blackpepper_zellij_version(&session_generation);
        apply_agent_environment(&mut launch, workspace_env, terminal_identity_supported);
        if let Err(error) =
            self.preflight_integration(workspace.host_id, &workspace.root_path, &launch)
        {
            let cleanup = self.cleanup_assets_note(workspace.host_id, &launch.assets);
            return Err(format!("{error}{cleanup}"));
        }

        let (zellij, session, _) = match self.ensure_workspace_session_under_lease(&workspace) {
            Ok(value) => value,
            Err(error) => {
                let cleanup = self.cleanup_assets_note(workspace.host_id, &launch.assets);
                return Err(format!("{error}{cleanup}"));
            }
        };
        let registration = connection::registry_operation(
            self,
            workspace.host_id,
            crate::core::RequestOperation::RegisterAgentRun {
                workspace_id: workspace.id,
                run_id,
                pane_id: Some(pane_id),
                provider,
            },
        );
        if !matches!(registration, Ok(crate::core::ResponsePayload::Acknowledged)) {
            let message = match registration {
                Err(error) => error,
                Ok(_) => "bp-host returned an unexpected registration response.".to_owned(),
            };
            let cleanup = self.cleanup_assets_note(workspace.host_id, &launch.assets);
            return Err(format!(
                "bp-host could not initialize the agent status run: {message}{cleanup}"
            ));
        }

        let initial = initial_agent_command(&launch);
        let name = format!("agent-{run_id}");
        let tab_result = self.transport_mut(workspace.host_id).and_then(|transport| {
            zellij
                .ensure_tab(
                    transport,
                    &session.backend_session_id,
                    &name,
                    Path::new(&workspace.root_path),
                    Some(&initial),
                )
                .map_err(|error| error.to_string())
        });
        let (tab_id, _) = match tab_result {
            Ok(value) => value,
            Err(error) => {
                let abort =
                    self.abort_note(workspace.host_id, workspace.id, run_id, pane_id, provider);
                let assets = self.cleanup_assets_note(workspace.host_id, &launch.assets);
                return Err(format!(
                    "Could not create the agent tab: {error}.{abort}{assets}"
                ));
            }
        };
        let pane_result = self.transport_mut(workspace.host_id).and_then(|transport| {
            zellij
                .terminal_pane_for_tab(transport, &session.backend_session_id, tab_id)
                .map_err(|error| error.to_string())
        });
        let pane = match pane_result {
            Ok(pane) => pane,
            Err(error) => {
                let cleanup = self.cleanup_agent_tab(
                    &zellij,
                    workspace.host_id,
                    &session.backend_session_id,
                    None,
                );
                let abort =
                    self.abort_note(workspace.host_id, workspace.id, run_id, pane_id, provider);
                let assets = self.cleanup_assets_note(workspace.host_id, &launch.assets);
                return Err(format!(
                    "Could not identify the new agent pane: {error}.{cleanup}{abort}{assets}"
                ));
            }
        };
        let launch_marker = format!("{AGENT_RUN_ID_ENV}={run_id}");
        if !pane.has_command_argument(&launch_marker) {
            let cleanup = self.cleanup_agent_tab(
                &zellij,
                workspace.host_id,
                &session.backend_session_id,
                None,
            );
            let abort = self.abort_note(workspace.host_id, workspace.id, run_id, pane_id, provider);
            let assets = self.cleanup_assets_note(workspace.host_id, &launch.assets);
            return Err(format!(
                "Zellij did not preserve the launch-scoped agent identity marker; status recovery was disabled.{cleanup}{abort}{assets}"
            ));
        }
        // The immutable run marker is stronger ownership evidence than
        // Zellij's sometimes-missing new-tab response. From this point on,
        // downstream cleanup may close a reconciled launch tab safely.
        let zellij_pane_id = pane.selector();
        let owned_tab = VerifiedAgentTab {
            tab_id,
            tab_name: &name,
            pane_selector: &zellij_pane_id,
            launch_marker: &launch_marker,
        };
        let binding = AgentRunBinding {
            session_id: session.id,
            session_name: session.backend_session_id.clone(),
            zellij_version: session.backend_version.clone(),
            tab_id,
            tab_name: name.clone(),
            zellij_pane_id: zellij_pane_id.clone(),
        };
        if let Err(error) = self.bind_agent_run(
            workspace.host_id,
            workspace.id,
            run_id,
            pane_id,
            provider,
            binding,
        ) {
            let tab = self.cleanup_agent_tab(
                &zellij,
                workspace.host_id,
                &session.backend_session_id,
                Some(&owned_tab),
            );
            let abort = self.abort_note(workspace.host_id, workspace.id, run_id, pane_id, provider);
            let assets = self.cleanup_assets_note(workspace.host_id, &launch.assets);
            return Err(format!(
                "Could not persist the exact Zellij agent binding: {error}.{tab}{abort}{assets}"
            ));
        }
        if let Err(error) =
            self.wait_for_integration_health(workspace.host_id, run_id, tab_id, &launch)
        {
            // Codex hook trust can only be reviewed in the running TUI. Keep
            // that tab available for `/hooks`, but do not rediscover an
            // unhealthy status descriptor. The user closes it and retries
            // after trusting the hook.
            if provider_kind == ProviderKind::Codex {
                let abort =
                    self.abort_note(workspace.host_id, workspace.id, run_id, pane_id, provider);
                return Err(format!("{error}{abort}"));
            }
            let cleanup = self.cleanup_agent_tab(
                &zellij,
                workspace.host_id,
                &session.backend_session_id,
                Some(&owned_tab),
            );
            let abort = self.abort_note(workspace.host_id, workspace.id, run_id, pane_id, provider);
            let assets = self.cleanup_assets_note(workspace.host_id, &launch.assets);
            return Err(format!("{error}{cleanup}{abort}{assets}"));
        }
        if let Err(error) = self.start_blocker_watcher(
            workspace.host_id,
            workspace.id,
            run_id,
            pane_id,
            provider,
            &session.backend_session_id,
            &session.backend_version,
            &zellij_pane_id,
            0,
            sender,
        ) {
            let cleanup = self.cleanup_agent_tab(
                &zellij,
                workspace.host_id,
                &session.backend_session_id,
                Some(&owned_tab),
            );
            let abort = self.abort_note(workspace.host_id, workspace.id, run_id, pane_id, provider);
            let assets = self.cleanup_assets_note(workspace.host_id, &launch.assets);
            return Err(format!(
                "The provider integration is healthy, but its blocker monitor could not start: {error}.{cleanup}{abort}{assets}"
            ));
        }
        let spawned = SpawnedAgent {
            run_id,
            pane_id,
            tab_id,
            zellij_pane_id,
            capability: provider_kind.needs_input_capability(),
        };
        lease.release()?;
        Ok(spawned)
    }
}

#[cfg(test)]
#[path = "agents/tests.rs"]
mod tests;
