use super::agent_events::{healthy_event, AgentRunContext, HostAgentEvents};
use crate::agent_status::{AgentEventKind, Provider};
use crate::core::{AgentRunId, CorePaths, HostRegistry, PaneId, WorkspaceId};
use serde_json::Value;
use std::io::Read;

const MAX_HOOK_INPUT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderHookArgs {
    pub provider: Provider,
    pub workspace_id: WorkspaceId,
    pub run_id: AgentRunId,
    pub pane_id: Option<PaneId>,
}

impl ProviderHookArgs {
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Option<Self> {
        let mut provider = None;
        let mut workspace_id = None;
        let mut run_id = None;
        let mut pane_id = None;
        let mut arguments = arguments.into_iter();
        while let Some(flag) = arguments.next() {
            let value = arguments.next()?;
            match flag.as_str() {
                "--provider" if provider.is_none() => provider = value.parse().ok(),
                "--workspace-id" if workspace_id.is_none() => workspace_id = value.parse().ok(),
                "--run-id" if run_id.is_none() => run_id = value.parse().ok(),
                "--pane-id" if pane_id.is_none() => pane_id = Some(value.parse().ok()?),
                _ => return None,
            }
        }
        Some(Self {
            provider: provider?,
            workspace_id: workspace_id?,
            run_id: run_id?,
            pane_id,
        })
    }
}

/// Reduces one bounded hook payload to semantic events and discards the raw
/// JSON before persistence. Callers deliberately ignore the result so hooks
/// always remain fail-silent to the provider.
pub fn record_provider_hook(
    paths: &CorePaths,
    registry: &HostRegistry,
    arguments: ProviderHookArgs,
    reader: impl Read,
) -> bool {
    try_record_provider_hook(paths, registry, arguments, reader).is_some()
}

fn try_record_provider_hook(
    paths: &CorePaths,
    registry: &HostRegistry,
    arguments: ProviderHookArgs,
    reader: impl Read,
) -> Option<()> {
    let payload = read_bounded(reader)?;
    let value: Value = serde_json::from_slice(&payload).ok()?;
    let kinds = reduce_provider_event(arguments.provider, &value)?;
    let semantic_sequence = if arguments.provider == Provider::OpenCode {
        value.get("semantic_sequence")?.as_u64()?
    } else {
        0
    };
    drop(value);
    drop(payload);

    let host_id = registry.local_host_id().ok()?;
    let context = AgentRunContext {
        host_id,
        workspace_id: arguments.workspace_id,
        run_id: arguments.run_id,
        pane_id: arguments.pane_id,
        provider: arguments.provider,
    };
    let mut events = HostAgentEvents::open(paths).ok()?;
    if kinds.contains(&healthy_event()) {
        events.register_run(registry, context).ok()?;
    }
    if arguments.provider == Provider::OpenCode {
        events
            .record_opencode_delivery(context, &kinds, semantic_sequence)
            .ok()
    } else {
        events.append_many(context, &kinds).ok().map(|_| ())
    }
}

fn read_bounded(mut reader: impl Read) -> Option<Vec<u8>> {
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = reader.read(&mut buffer).ok()?;
        if read == 0 {
            return (!retained.is_empty()).then_some(retained);
        }
        if retained.len() + read > MAX_HOOK_INPUT_BYTES {
            return None;
        }
        retained.extend_from_slice(&buffer[..read]);
    }
}

fn reduce_provider_event(provider: Provider, value: &Value) -> Option<Vec<AgentEventKind>> {
    let event_name = event_name(value)?;
    let event = normalize(event_name);
    match provider {
        Provider::Codex | Provider::Claude => match event.as_str() {
            "sessionstart" => Some(vec![healthy_event(), AgentEventKind::Ready]),
            "userpromptsubmit" | "pretooluse" | "posttooluse" => {
                Some(vec![AgentEventKind::Working])
            }
            "permissionrequest" => Some(vec![AgentEventKind::NeedsInput]),
            "stop" | "taskcompleted" => Some(vec![AgentEventKind::TurnCompleted]),
            "sessionend" => Some(vec![AgentEventKind::StateUnknown]),
            _ => None,
        },
        Provider::OpenCode => reduce_opencode(&event, value),
    }
}

fn reduce_opencode(event: &str, value: &Value) -> Option<Vec<AgentEventKind>> {
    match event {
        // This private event is emitted only after the managed plugin has
        // successfully invoked bp-host. Native lifecycle events do not prove
        // that the external plugin was loaded.
        "blackpepperintegrationready" => Some(vec![healthy_event()]),
        // A heartbeat proves only that this launch-scoped plugin can still
        // reach bp-host. Host storage collapses repeated pulses into one
        // freshness row and records only health edges.
        "blackpepperintegrationheartbeat" => Some(Vec::new()),
        "sessioncreated" => Some(vec![AgentEventKind::Ready]),
        "permissionasked" | "questionasked" => Some(vec![AgentEventKind::NeedsInput]),
        "permissionreplied" | "questionreplied" | "questionrejected" => {
            Some(vec![AgentEventKind::Working])
        }
        "toolexecutebefore" | "toolexecuteafter" | "messageupdated" => {
            Some(vec![AgentEventKind::Working])
        }
        "sessionidle" => Some(vec![AgentEventKind::TurnCompleted]),
        "sessiondeleted" | "sessionerror" => Some(vec![AgentEventKind::StateUnknown]),
        "sessionstatus" => match value.get("status").and_then(Value::as_str).map(normalize) {
            Some(status) if status == "busy" || status == "retry" => {
                Some(vec![AgentEventKind::Working])
            }
            Some(status) if status == "idle" => Some(vec![AgentEventKind::TurnCompleted]),
            _ => None,
        },
        _ => None,
    }
}

fn event_name(value: &Value) -> Option<&str> {
    value
        .get("hook_event_name")
        .or_else(|| value.get("type"))
        .or_else(|| value.get("event_name"))
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("event")
                .and_then(|event| event.as_str().or_else(|| event.get("type")?.as_str()))
        })
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
#[path = "provider_hook_tests.rs"]
mod tests;
