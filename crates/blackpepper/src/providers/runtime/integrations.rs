use super::{ProviderKind, OPENCODE_HEARTBEAT_INTERVAL_MS};
use crate::core::{AgentRunId, PaneId, WorkspaceId};
use std::path::Path;

pub(super) fn hook_command(provider: ProviderKind, helper: &Path) -> String {
    format!(
        "{} agent-event --provider {} --workspace-id \"$BLACKPEPPER_WORKSPACE_ID\" --run-id \"$BLACKPEPPER_AGENT_RUN_ID\" --pane-id \"$BLACKPEPPER_PANE_ID\"",
        quote_posix(&helper.to_string_lossy()),
        match provider {
            ProviderKind::Codex => "codex",
            ProviderKind::Claude => "claude",
            ProviderKind::OpenCode => "opencode",
        }
    )
}

pub(super) fn codex_args(command: &str) -> Vec<String> {
    [
        "SessionStart",
        "UserPromptSubmit",
        "PermissionRequest",
        "PostToolUse",
        "Stop",
        "SessionEnd",
    ]
    .into_iter()
    .flat_map(|event| {
        let command = toml::Value::String(command.to_string()).to_string();
        [
            "-c".to_string(),
            format!("hooks.{event}=[{{hooks=[{{type=\"command\",command={command},timeout=3}}]}}]"),
        ]
    })
    .collect()
}

pub(super) fn claude_settings(command: &str) -> Result<Vec<u8>, String> {
    let handler = || {
        serde_json::json!({
            "hooks": [{"type": "command", "command": command, "timeout": 3}]
        })
    };
    let settings = serde_json::json!({
        "hooks": {
            "SessionStart": [handler()],
            "UserPromptSubmit": [handler()],
            "PermissionRequest": [handler()],
            "PostToolUse": [handler()],
            "Stop": [handler()],
            "SessionEnd": [handler()]
        }
    });
    serde_json::to_vec_pretty(&settings).map_err(|err| err.to_string())
}

pub(super) fn opencode_plugin(
    helper: &Path,
    workspace_id: WorkspaceId,
    run_id: AgentRunId,
    pane_id: PaneId,
) -> String {
    let helper = serde_json::to_string(&helper.to_string_lossy()).unwrap_or_default();
    let workspace = serde_json::to_string(&workspace_id.to_string()).unwrap_or_default();
    let run = serde_json::to_string(&run_id.to_string()).unwrap_or_default();
    let pane = serde_json::to_string(&pane_id.to_string()).unwrap_or_default();
    format!(
        r#"const HELPER = {helper};
const WORKSPACE = {workspace};
const RUN = {run};
const PANE = {pane};
const HEARTBEAT_MS = {heartbeat_ms};
const DELIVERY_TIMEOUT_MS = 3000;
const MAX_PENDING = 256;
let queue = Promise.resolve();
let pending = 0;
let semanticSequence = 0;
const SEMANTIC_TYPES = new Set([
  "session.created", "permission.asked", "question.asked",
  "permission.replied", "question.replied", "question.rejected",
  "tool.execute.before", "tool.execute.after", "message.updated",
  "session.idle", "session.deleted", "session.error", "session.status"
]);

function compact(event) {{
  const type = event?.type || "unknown";
  if (!SEMANTIC_TYPES.has(type)) return null;
  const sessionID = event?.properties?.sessionID || event?.properties?.info?.id || null;
  const status = event?.properties?.status?.type || null;
  semanticSequence += 1;
  return {{ type, session_id: sessionID, status, semantic_sequence: semanticSequence }};
}}

async function deliver(body) {{
  const child = Bun.spawn([
    HELPER, "agent-event", "--provider", "opencode",
    "--workspace-id", WORKSPACE, "--run-id", RUN, "--pane-id", PANE
  ], {{ stdin: new Blob([body]), stdout: "ignore", stderr: "ignore" }});
  const timeout = setTimeout(() => child.kill(), DELIVERY_TIMEOUT_MS);
  try {{
    const exitCode = await child.exited;
    if (exitCode !== 0) throw new Error("status helper unavailable");
  }} finally {{
    clearTimeout(timeout);
  }}
}}

function enqueue(body, heartbeat = false) {{
  // One later heartbeat is enough to prove recovery. Dropping pulses while a
  // delivery is pending prevents an unavailable helper from growing memory.
  // Dropped semantic events still advance the cursor and therefore fail the
  // host authority closed on the next successful heartbeat.
  if ((heartbeat && pending > 0) || pending >= MAX_PENDING) return Promise.resolve();
  pending += 1;
  queue = queue
    .then(() => deliver(body))
    .catch(() => undefined)
    .finally(() => {{ pending -= 1; }});
  return queue;
}}

export const BlackpepperStatus = async () => {{
  // A native server event can occur before external plugins are registered.
  // This explicit event proves that this exact launch-scoped plugin loaded and
  // could reach the helper; it carries no provider payload.
  await enqueue(JSON.stringify({{
    type: "blackpepper.integration.ready", semantic_sequence: semanticSequence
  }}));
  const heartbeat = setInterval(() => {{
    void enqueue(JSON.stringify({{
      type: "blackpepper.integration.heartbeat", semantic_sequence: semanticSequence
    }}), true);
  }}, HEARTBEAT_MS);
  heartbeat.unref?.();
  return {{
    event: async ({{ event }}) => {{
      // Provider work never waits for status delivery. The serialized queue
      // still preserves semantic ordering, and heartbeat expiry reports a
      // wedged or unavailable helper through Blackpepper itself.
      const body = compact(event);
      if (body !== null) void enqueue(JSON.stringify(body));
    }}
  }};
}};
"#,
        heartbeat_ms = OPENCODE_HEARTBEAT_INTERVAL_MS,
    )
}

fn quote_posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
