use super::ClientRuntime;
use crate::core::{AgentRunBinding, AgentRunId, HostId};
use crate::providers::runtime::AGENT_RUN_ID_ENV;
use crate::zellij::ZellijRuntime;

pub(crate) use crate::zellij::PaneProcessState;

impl ClientRuntime {
    /// Query one persisted agent pane through its session's exact Zellij
    /// version. This does not attach, focus, or otherwise mutate the session.
    pub(crate) fn observe_zellij_pane(
        &mut self,
        host_id: HostId,
        binding: &AgentRunBinding,
        run_id: AgentRunId,
    ) -> Result<PaneProcessState, String> {
        let binary = self.exact_binary(host_id, "zellij", &binding.zellij_version)?;
        let zellij = ZellijRuntime::for_version(binary, &binding.zellij_version)
            .map_err(|error| error.to_string())?;
        let (zellij, session_exists) = zellij
            .resolve_session_namespace(self.transport_mut(host_id)?, &binding.session_name)
            .map_err(|error| error.to_string())?;
        if !session_exists {
            return Ok(PaneProcessState::Missing);
        }
        let launch_marker = format!("{AGENT_RUN_ID_ENV}={run_id}");
        zellij
            .pane_process_state(
                self.transport_mut(host_id)?,
                &binding.session_name,
                binding.tab_id,
                &binding.tab_name,
                &binding.zellij_pane_id,
                &launch_marker,
            )
            .map_err(|error| error.to_string())
    }
}
