//! Pull request helpers (prompt templates, provider defaults, parsing).

pub const PR_CREATE: &str = include_str!("../../assets/prompts/PR_CREATE.md");
#[allow(dead_code)]
pub const COMMIT_CHANGES: &str = include_str!("../../assets/prompts/COMMIT_CHANGES.md");

#[derive(Debug, Clone)]
pub struct PrMessage {
    pub title: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct PrProvider {
    pub name: &'static str,
    pub command: &'static str,
}

const CODEX_COMMAND: &str = r#"mkdir -p /tmp/blackpepper/codex && cp -r "${CODEX_HOME:-$HOME/.codex}"/* /tmp/blackpepper/codex/ 2>/dev/null && rm -f /tmp/blackpepper/codex/config.toml && CODEX_HOME=/tmp/blackpepper/codex codex exec --skip-git-repo-check {{PROMPT}}"#;

const CLAUDE_COMMAND: &str = r#"tmpdir=$(mktemp -d) && cd "$tmpdir" && claude -p {{PROMPT}} \
    --tools "" \
    --strict-mcp-config --mcp-config '{"mcpServers":{}}' \
    --disable-slash-commands \
    --no-session-persistence \
    --setting-sources user"#;

const OPENCODE_COMMAND: &str = r#"tmpdir=$(mktemp -d) && tmpcfg=$(mktemp -d) && cd "$tmpdir" && XDG_CONFIG_HOME="$tmpcfg" opencode run {{PROMPT}}"#;

const PROVIDERS: &[PrProvider] = &[
    PrProvider {
        name: "codex",
        command: CODEX_COMMAND,
    },
    PrProvider {
        name: "claude",
        command: CLAUDE_COMMAND,
    },
    PrProvider {
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

pub fn build_prompt_script(command_template: &str, prompt: &str) -> String {
    let delimiter = prompt_heredoc_delimiter(prompt);
    let mut script = String::new();
    script.push_str("PROMPT_FILE=$(mktemp)\n");
    script.push_str("cat <<'");
    script.push_str(&delimiter);
    script.push_str("' > \"$PROMPT_FILE\"\n");
    script.push_str(prompt);
    if !prompt.ends_with('\n') {
        script.push('\n');
    }
    script.push_str(&delimiter);
    script.push_str("\nPROMPT=$(cat \"$PROMPT_FILE\")\n");
    script.push_str("rm -f \"$PROMPT_FILE\"\n");

    let command = if command_template.contains("{{PROMPT}}") {
        command_template.replace("{{PROMPT}}", "\"$PROMPT\"")
    } else {
        format!("{command_template} \"$PROMPT\"")
    };
    script.push_str(&command);
    script
}

pub fn parse_pr_output(output: &str) -> Result<PrMessage, String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Err("PR generator returned empty output.".to_string());
    }

    if let Some(error_block) = extract_block(trimmed, "error") {
        let reason = extract_tag(&error_block, "reason").unwrap_or_else(|| "Unknown error".into());
        let action = extract_tag(&error_block, "action").unwrap_or_default();
        if action.trim().is_empty() {
            return Err(format!("PR generator error: {reason}"));
        }
        return Err(format!("PR generator error: {reason} ({action})"));
    }

    let pr_block = extract_block(trimmed, "pr")
        .ok_or_else(|| "PR generator output missing <pr> block.".to_string())?;
    let title = extract_tag(&pr_block, "title")
        .ok_or_else(|| "PR generator output missing <title>.".to_string())?
        .trim()
        .to_string();
    let description = extract_tag(&pr_block, "description")
        .ok_or_else(|| "PR generator output missing <description>.".to_string())?
        .trim()
        .to_string();

    if title.is_empty() {
        return Err("PR generator returned an empty title.".to_string());
    }
    if description.is_empty() {
        return Err("PR generator returned an empty description.".to_string());
    }

    Ok(PrMessage { title, description })
}

fn prompt_heredoc_delimiter(prompt: &str) -> String {
    let base = "BLACKPEPPER_PROMPT";
    let mut suffix = 0usize;
    loop {
        let delimiter = if suffix == 0 {
            base.to_string()
        } else {
            format!("{base}_{suffix}")
        };
        if !prompt.lines().any(|line| line == delimiter) {
            return delimiter;
        }
        suffix += 1;
    }
}

fn extract_block(content: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = content.find(&open)? + open.len();
    let end = content[start..].find(&close)? + start;
    Some(content[start..end].to_string())
}

fn extract_tag(content: &str, tag: &str) -> Option<String> {
    extract_block(content, tag)
}

#[cfg(test)]
mod tests {
    use super::{build_prompt_script, parse_pr_output};

    #[test]
    fn build_prompt_script_avoids_prompt_delimiter_collisions() {
        let prompt = "Line one\nBLACKPEPPER_PROMPT\nLine two";
        let script = build_prompt_script("echo {{PROMPT}}", prompt);
        assert!(script.contains("BLACKPEPPER_PROMPT_1"));
    }

    #[test]
    fn parse_pr_output_success() {
        let output = r#"
<pr>
  <title>feat(ui): add PR creation flow</title>
  <description>
## Summary
Added a PR flow.
  </description>
</pr>
"#;
        let parsed = parse_pr_output(output).expect("parse ok");
        assert_eq!(parsed.title, "feat(ui): add PR creation flow");
        assert!(parsed.description.contains("## Summary"));
    }

    #[test]
    fn parse_pr_output_error() {
        let output = r#"
<error>
  <reason>No changes</reason>
  <action>Commit first</action>
</error>
"#;
        let err = parse_pr_output(output).expect_err("parse err");
        assert!(err.contains("No changes"));
        assert!(err.contains("Commit first"));
    }
}
