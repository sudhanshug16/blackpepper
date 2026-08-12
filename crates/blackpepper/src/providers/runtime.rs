//! Launch-scoped provider integrations. These builders never replace a
//! provider's user/project configuration and never weaken its permissions.

mod assets;
mod integrations;

use crate::core::{AgentRunId, PaneId, WorkspaceId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) use assets::write_private_atomic;
use integrations::{claude_settings, codex_args, hook_command, opencode_plugin};

pub const INTEGRATION_HEALTH_TIMEOUT_SECS: u64 = 5;
pub const OPENCODE_HEARTBEAT_INTERVAL_MS: u64 = 2_000;
/// Allow several missed pulses before demoting plugin authority. This avoids
/// flapping during short host load spikes while still failing closed quickly.
pub const OPENCODE_HEALTH_STALE_AFTER_MS: u64 = 10_000;
pub const AGENT_RUN_ID_ENV: &str = "BLACKPEPPER_AGENT_RUN_ID";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Codex,
    Claude,
    OpenCode,
}

impl ProviderKind {
    pub fn command(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::OpenCode => "opencode",
        }
    }

    pub fn needs_input_capability(self) -> &'static str {
        match self {
            Self::OpenCode => "full",
            Self::Codex | Self::Claude => "partial",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
            Self::OpenCode => "OpenCode",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedAsset {
    pub path: PathBuf,
    pub contents: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderLaunch {
    pub provider: ProviderKind,
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub assets: Vec<ManagedAsset>,
    pub health_event: &'static str,
}

impl ProviderLaunch {
    pub fn install_assets(&self) -> Result<(), String> {
        for asset in &self.assets {
            write_private_atomic(&asset.path, &asset.contents)?;
        }
        Ok(())
    }

    /// Return a non-interactive command that makes the provider parse the
    /// launch-scoped integration without starting an agent session. The real
    /// health handshake still has to arrive after the interactive process
    /// starts; this only catches unsupported or malformed configuration early.
    pub fn preflight_args(&self) -> Option<Vec<String>> {
        let mut args = self.args.clone();
        match self.provider {
            ProviderKind::Codex => args.extend(["features".to_string(), "list".to_string()]),
            ProviderKind::Claude => args.push("doctor".to_string()),
            // OpenCode does not expose a stable config-validation command. Its
            // managed plugin emits an explicit fail-closed runtime handshake.
            ProviderKind::OpenCode => return None,
        }
        Some(args)
    }

    pub fn handshake_error(&self, tab_id: u64) -> String {
        match self.provider {
            ProviderKind::Codex => format!(
                "Codex did not run Blackpepper's SessionStart hook within {INTEGRATION_HEALTH_TIMEOUT_SECS}s. Open Zellij tab {tab_id}, make sure the hooks feature is enabled, then run /hooks to review and trust the Blackpepper hook. Close the tab and retry. Blackpepper never bypasses hook trust."
            ),
            ProviderKind::Claude => format!(
                "Claude Code did not run Blackpepper's SessionStart hook within {INTEGRATION_HEALTH_TIMEOUT_SECS}s in Zellij tab {tab_id}. Make sure hooks are enabled and Claude Code is authenticated on this host, then close the tab and retry."
            ),
            ProviderKind::OpenCode => format!(
                "OpenCode did not complete Blackpepper's managed-plugin handshake within {INTEGRATION_HEALTH_TIMEOUT_SECS}s in Zellij tab {tab_id}. Authenticate OpenCode or inspect its plugin logs on this host, then retry."
            ),
        }
    }
}

pub fn build_launch(
    provider: ProviderKind,
    workspace_id: WorkspaceId,
    run_id: AgentRunId,
    pane_id: PaneId,
    helper: &Path,
    integration_dir: &Path,
) -> Result<ProviderLaunch, String> {
    if !helper.is_absolute() || !integration_dir.is_absolute() {
        return Err(
            "Provider helper and integration directory must be absolute paths.".to_string(),
        );
    }
    // Keep the reviewed hook definition stable between runs. IDs remain
    // launch-scoped environment values while the trusted command pins the
    // exact managed helper path.
    let hook_command = hook_command(provider, helper);
    let mut env = BTreeMap::from([
        (
            "BLACKPEPPER_WORKSPACE_ID".to_string(),
            workspace_id.to_string(),
        ),
        (AGENT_RUN_ID_ENV.to_string(), run_id.to_string()),
        ("BLACKPEPPER_PANE_ID".to_string(), pane_id.to_string()),
    ]);
    let (args, assets) = match provider {
        ProviderKind::Codex => (codex_args(&hook_command), Vec::new()),
        ProviderKind::Claude => {
            let settings = integration_dir.join(format!("claude-{run_id}.json"));
            let contents = claude_settings(&hook_command)?;
            (
                vec![
                    "--settings".to_string(),
                    settings.to_string_lossy().into_owned(),
                ],
                vec![ManagedAsset {
                    path: settings,
                    contents,
                }],
            )
        }
        ProviderKind::OpenCode => {
            let plugin = integration_dir.join(format!("opencode-{run_id}.js"));
            let contents = opencode_plugin(helper, workspace_id, run_id, pane_id).into_bytes();
            let inline = serde_json::json!({
                "plugin": [plugin.to_string_lossy()]
            });
            env.insert("OPENCODE_CONFIG_CONTENT".to_string(), inline.to_string());
            (
                Vec::new(),
                vec![ManagedAsset {
                    path: plugin,
                    contents,
                }],
            )
        }
    };
    Ok(ProviderLaunch {
        provider,
        program: provider.command().to_string(),
        args,
        env,
        assets,
        health_event: match provider {
            ProviderKind::Codex | ProviderKind::Claude => "SessionStart",
            ProviderKind::OpenCode => "blackpepper.integration.ready",
        },
    })
}

#[cfg(test)]
mod tests;
