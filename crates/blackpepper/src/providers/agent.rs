/// Agent provider defaults for PR generation.

#[derive(Debug, Clone)]
pub struct AgentProvider {
    pub name: &'static str,
    pub command: &'static str,
}

const CODEX_COMMAND: &str = r#"repo="$(pwd)" && \
main_git="$(git rev-parse --git-common-dir)" && \
PATH="/usr/bin:/bin:/usr/sbin:/sbin:$PATH" \
mkdir -p /tmp/blackpepper/codex && \
cp -r "${CODEX_HOME:-$HOME/.codex}"/* /tmp/blackpepper/codex/ 2>/dev/null && \
rm -f /tmp/blackpepper/codex/config.toml && \
CODEX_HOME=/tmp/blackpepper/codex \
codex exec -C "$repo" -s workspace-write --add-dir "$main_git" \
{{PROMPT}}"#;

const CLAUDE_COMMAND: &str = r#"repo="$(pwd)" && \
PATH="/usr/bin:/bin:/usr/sbin:/sbin:$PATH" \
cd "$repo" && \
claude -p {{PROMPT}} \
    --tools "Bash" \
    --allowed-tools "Bash" \
    --permission-mode bypassPermissions \
    --strict-mcp-config --mcp-config '{"mcpServers":{}}' \
    --disable-slash-commands \
    --no-session-persistence \
    --setting-sources user"#;

const OPENCODE_COMMAND: &str = r#"repo="$(pwd)" && tmpcfg=$(mktemp -d) && \
PATH="/usr/bin:/bin:/usr/sbin:/sbin:$PATH" \
cd "$repo" && \
XDG_CONFIG_HOME="$tmpcfg" opencode run {{PROMPT}}"#;

const PROVIDERS: &[AgentProvider] = &[
    AgentProvider {
        name: "codex",
        command: CODEX_COMMAND,
    },
    AgentProvider {
        name: "claude",
        command: CLAUDE_COMMAND,
    },
    AgentProvider {
        name: "opencode",
        command: OPENCODE_COMMAND,
    },
];

pub fn provider_names() -> Vec<String> {
    PROVIDERS
        .iter()
        .map(|provider| provider.name.to_string())
        .collect()
}

pub fn provider_command(provider: &str) -> Option<&'static str> {
    PROVIDERS
        .iter()
        .find(|candidate| candidate.name.eq_ignore_ascii_case(provider))
        .map(|provider| provider.command)
}
