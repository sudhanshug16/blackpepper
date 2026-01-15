//! Workspace rename helpers (prompt templates, parsing).

pub const WORKSPACE_RENAME: &str = include_str!("../../assets/prompts/WORKSPACE_RENAME.md");

#[derive(Debug, Clone)]
pub struct RenameMessage {
    pub name: String,
}

pub fn parse_rename_output(output: &str) -> Result<RenameMessage, String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Err("Rename generator returned empty output.".to_string());
    }

    if let Some(error_block) = extract_block(trimmed, "error") {
        let reason = extract_tag(&error_block, "reason").unwrap_or_else(|| "Unknown error".into());
        let action = extract_tag(&error_block, "action").unwrap_or_default();
        if action.trim().is_empty() {
            return Err(format!("Rename generator error: {reason}"));
        }
        return Err(format!("Rename generator error: {reason} ({action})"));
    }

    let rename_block = extract_block(trimmed, "rename")
        .ok_or_else(|| "Rename generator output missing <rename> block.".to_string())?;
    let name = extract_tag(&rename_block, "name")
        .ok_or_else(|| "Rename generator output missing <name>.".to_string())?
        .trim()
        .to_string();

    if name.is_empty() {
        return Err("Rename generator returned an empty name.".to_string());
    }

    Ok(RenameMessage { name })
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
    use super::parse_rename_output;

    #[test]
    fn parse_rename_output_success() {
        let output = r#"
<rename>
  <name>workspace-slug</name>
</rename>
"#;
        let parsed = parse_rename_output(output).expect("parse ok");
        assert_eq!(parsed.name, "workspace-slug");
    }

    #[test]
    fn parse_rename_output_error() {
        let output = r#"
<error>
  <reason>Needs input</reason>
  <action>Provide a name</action>
</error>
"#;
        let err = parse_rename_output(output).expect_err("parse err");
        assert!(err.contains("Needs input"));
        assert!(err.contains("Provide a name"));
    }
}
