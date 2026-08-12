use super::agent_events::HostAgentEvents;
use super::tool_runtime::{discover_exact_binary, validate_exact_binary};
use crate::agent_status::{IntegrationHealth, Provider};
use crate::core::{AgentRunId, CorePaths, HostRegistry, PaneId, WorkspaceId};
use crate::status_monitor::{
    run_host_local_subscription_cancellable_with_health, run_host_local_subscription_fallible,
    HostSubscriptionError, MonitorContext, ViewportBlockerMonitor,
};
use crate::zellij::ZellijRuntime;
use std::io::{self, Write};
use std::path::Path;
use std::sync::mpsc::Receiver;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockerWatchArgs {
    pub workspace_id: WorkspaceId,
    pub run_id: AgentRunId,
    pub pane_id: PaneId,
    pub provider: Provider,
    pub session: String,
    pub zellij_version: String,
    pub zellij_pane_id: String,
    pub after_sequence: u64,
}

impl BlockerWatchArgs {
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Option<Self> {
        let mut workspace_id = None;
        let mut run_id = None;
        let mut pane_id = None;
        let mut provider = None;
        let mut session = None;
        let mut zellij_version = None;
        let mut zellij_pane_id = None;
        let mut after_sequence = None;
        let mut arguments = arguments.into_iter();
        while let Some(flag) = arguments.next() {
            let value = arguments.next()?;
            match flag.as_str() {
                "--workspace-id" if workspace_id.is_none() => workspace_id = value.parse().ok(),
                "--run-id" if run_id.is_none() => run_id = value.parse().ok(),
                "--pane-id" if pane_id.is_none() => pane_id = value.parse().ok(),
                "--provider" if provider.is_none() => provider = value.parse().ok(),
                "--session" if session.is_none() => session = Some(value),
                "--zellij-version" if zellij_version.is_none() => {
                    if !valid_version(&value) {
                        return None;
                    }
                    zellij_version = Some(value);
                }
                "--zellij-pane-id" if zellij_pane_id.is_none() => zellij_pane_id = Some(value),
                "--after-sequence" if after_sequence.is_none() => {
                    after_sequence = value.parse().ok()
                }
                _ => return None,
            }
        }
        Some(Self {
            workspace_id: workspace_id?,
            run_id: run_id?,
            pane_id: pane_id?,
            provider: provider?,
            session: session?,
            zellij_version: zellij_version?,
            zellij_pane_id: zellij_pane_id?,
            after_sequence: after_sequence.unwrap_or(0),
        })
    }
}

fn valid_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes().first().is_some_and(u8::is_ascii_digit)
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '+' | '_')
        })
}

/// Runs Zellij and evaluates viewport rules entirely on the workspace host.
/// Only redacted `BlockerTransition` values cross the supplied writer.
pub fn watch_blockers(
    paths: &CorePaths,
    registry: &HostRegistry,
    arguments: &BlockerWatchArgs,
    writer: impl Write,
) -> Result<(), String> {
    let binary = discover_exact_binary("Zellij", "zellij", "zellij", &arguments.zellij_version)?;
    watch_blockers_with_binary(paths, registry, arguments, writer, &binary)
}

/// Cancellable form used by `bp-host`: EOF on the helper's stdin triggers the
/// receiver and reaps the host-local Zellij subscription immediately.
pub fn watch_blockers_cancellable(
    paths: &CorePaths,
    registry: &HostRegistry,
    arguments: &BlockerWatchArgs,
    writer: impl Write + Send + 'static,
    cancelled: Receiver<()>,
) -> Result<(), String> {
    let binary = discover_exact_binary("Zellij", "zellij", "zellij", &arguments.zellij_version)?;
    let (monitor, runtime) = prepare_monitor(paths, registry, arguments, &binary)?;
    let mut events = HostAgentEvents::open(paths)?;
    run_host_local_subscription_cancellable_with_health(
        &runtime,
        &arguments.session,
        monitor,
        now_millis,
        transition_writer(writer),
        cancelled,
        || {
            events
                .integration_health(arguments.run_id)
                // If the freshness store itself cannot be read, fail closed:
                // the provider must not retain full-authority status.
                .unwrap_or(IntegrationHealth::Stale)
        },
        HEALTH_POLL_INTERVAL,
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn watch_blockers_with_binary(
    paths: &CorePaths,
    registry: &HostRegistry,
    arguments: &BlockerWatchArgs,
    mut writer: impl Write,
    binary: &Path,
) -> Result<(), String> {
    let (mut monitor, runtime) = prepare_monitor(paths, registry, arguments, binary)?;
    let result = run_host_local_subscription_fallible(
        &runtime,
        &arguments.session,
        &mut monitor,
        now_millis,
        transition_writer(&mut writer),
    );
    match result {
        Ok(_) => Ok(()),
        Err(HostSubscriptionError::Stream(error)) if error.kind() == io::ErrorKind::BrokenPipe => {
            Ok(())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn prepare_monitor(
    paths: &CorePaths,
    registry: &HostRegistry,
    arguments: &BlockerWatchArgs,
    binary: &Path,
) -> Result<(ViewportBlockerMonitor, ZellijRuntime), String> {
    let host_id = registry
        .local_host_id()
        .map_err(|error| error.to_string())?;
    let mut events = HostAgentEvents::open(paths)?;
    let context = events
        .context(arguments.run_id)?
        .ok_or_else(|| "Agent run is not registered or is stale.".to_owned())?;
    if context.host_id != host_id
        || context.workspace_id != arguments.workspace_id
        || context.pane_id != Some(arguments.pane_id)
        || context.provider != arguments.provider
    {
        return Err("Blocker watch context does not match the active agent run.".to_owned());
    }
    let integration_health = events
        .snapshot(arguments.run_id)?
        .map_or(IntegrationHealth::Unknown, |snapshot| {
            snapshot.snapshot.integration_health
        });
    let monitor_context = MonitorContext {
        host_id,
        workspace_id: arguments.workspace_id,
        run_id: arguments.run_id,
        pane_id: arguments.pane_id,
        provider: arguments.provider,
        integration_health,
    };
    let monitor = ViewportBlockerMonitor::bundled_after(
        monitor_context,
        &arguments.zellij_pane_id,
        arguments.after_sequence,
    )
    .map_err(|error| error.to_string())?;
    let binary = validate_exact_binary(binary.to_path_buf(), "Zellij", &arguments.zellij_version)?;
    let binary = binary
        .to_str()
        .ok_or_else(|| "Zellij binary path must be valid UTF-8.".to_owned())?;
    let runtime = ZellijRuntime::for_version(binary, &arguments.zellij_version)
        .map_err(|error| error.to_string())?;
    let mut transport = crate::transport::LocalTransport;
    let (runtime, _) = runtime
        .resolve_session_namespace(&mut transport, &arguments.session)
        .map_err(|error| error.to_string())?;
    Ok((monitor, runtime))
}

fn transition_writer(
    mut writer: impl Write,
) -> impl FnMut(crate::status_monitor::BlockerTransition) -> io::Result<()> {
    move |transition| {
        serde_json::to_writer(&mut writer, &transition).map_err(io::Error::other)?;
        writer.write_all(b"\n")?;
        writer.flush()
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "blocker_watch_tests.rs"]
mod tests;
