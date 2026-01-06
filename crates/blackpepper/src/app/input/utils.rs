use std::path::PathBuf;

pub(super) fn format_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn simplify_title(title: &str) -> String {
    let mut cleaned = title.trim();
    if let Some((head, _)) = cleaned.split_once(" - ") {
        cleaned = head.trim();
    }
    if let Some(idx) = cleaned.rfind(&['/', '\\'][..]) {
        let tail = cleaned[idx + 1..].trim();
        if !tail.is_empty() {
            return tail.to_string();
        }
    }
    cleaned.to_string()
}

pub(super) fn truncate_label(label: &str, max_len: usize) -> String {
    let len = label.chars().count();
    if len <= max_len {
        return label.to_string();
    }
    if max_len <= 3 {
        return label.chars().take(max_len).collect();
    }
    let keep = max_len - 3;
    let mut out: String = label.chars().take(keep).collect();
    out.push_str("...");
    out
}

pub(super) fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

pub(super) fn find_editor_binary() -> Option<String> {
    for name in ["vim", "vi"] {
        if find_executable(name).is_some() {
            return Some(name.to_string());
        }
    }
    None
}

pub(super) fn shell_escape(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' || ch == '/')
    {
        return value.to_string();
    }
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            escaped.push_str("'\\''");
        } else {
            escaped.push(ch);
        }
    }
    escaped.push('\'');
    escaped
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() && is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|meta| meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}
