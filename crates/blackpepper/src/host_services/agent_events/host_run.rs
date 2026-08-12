use super::StoredAgentRunContext;
use crate::agent_status::Provider;
use crate::core::HostAgentRun;
use std::path::Path;

pub(super) fn host_run_from(
    record: StoredAgentRunContext,
    snapshot: crate::agent_status::AgentSnapshot,
) -> Result<HostAgentRun, String> {
    let pane_id = record
        .context
        .pane_id
        .ok_or_else(|| "Bound agent run has no stable pane ID.".to_owned())?;
    let binding = record
        .binding
        .ok_or_else(|| "Agent run has no Zellij binding.".to_owned())?;
    Ok(HostAgentRun {
        host_id: record.context.host_id,
        workspace_id: record.context.workspace_id,
        run_id: record.context.run_id,
        pane_id,
        provider: record.context.provider,
        binding,
        snapshot,
    })
}

pub(super) fn cleanup_managed_asset(
    database_path: &Path,
    record: &StoredAgentRunContext,
) -> Result<(), String> {
    let name = match record.context.provider {
        Provider::Codex => return Ok(()),
        Provider::Claude => format!("claude-{}.json", record.context.run_id),
        Provider::OpenCode => format!("opencode-{}.js", record.context.run_id),
    };
    let path = database_path
        .parent()
        .ok_or_else(|| "Agent event database has no state directory.".to_owned())?
        .join("integrations")
        .join(name);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Could not clean exited agent integration {}: {error}",
            path.display()
        )),
    }
}
