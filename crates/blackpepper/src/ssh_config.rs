//! Explicit OpenSSH config import. Only literal positive aliases are offered;
//! imported records retain the alias so OpenSSH remains the source of truth.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshAliasPreview {
    pub alias: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub proxy_jump: Option<String>,
    pub identity_files: Vec<PathBuf>,
}

pub fn discover_literal_aliases(path: &Path) -> Result<Vec<String>, String> {
    let mut aliases = BTreeSet::new();
    let mut visited = HashSet::new();
    visit_config(path, &mut aliases, &mut visited, 0)?;
    Ok(aliases.into_iter().collect())
}

pub fn preview_alias(ssh_command: &Path, alias: &str) -> Result<SshAliasPreview, String> {
    validate_literal_alias(alias)?;
    let output = crate::transport::RunningCommand::spawn(
        &crate::transport::ProcessSpec::new(ssh_command).args(["-G", "--", alias]),
        false,
    )
    .and_then(crate::transport::RunningCommand::wait_with_output)
    .map_err(|err| format!("Could not run {} -G: {err}", ssh_command.display()))?;
    if !output.success {
        return Err(format!(
            "OpenSSH could not resolve '{alias}': {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_preview(alias, &String::from_utf8_lossy(&output.stdout))
}

fn parse_preview(alias: &str, output: &str) -> Result<SshAliasPreview, String> {
    let mut values: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for line in output.lines() {
        let Some((key, value)) = line.trim().split_once(char::is_whitespace) else {
            continue;
        };
        values.entry(key).or_default().push(value.trim());
    }
    Ok(SshAliasPreview {
        alias: alias.to_string(),
        hostname: first(&values, "hostname").map(ToOwned::to_owned),
        user: first(&values, "user").map(ToOwned::to_owned),
        port: first(&values, "port").and_then(|value| value.parse().ok()),
        proxy_jump: first(&values, "proxyjump")
            .filter(|value| *value != "none")
            .map(ToOwned::to_owned),
        identity_files: values
            .get("identityfile")
            .into_iter()
            .flatten()
            .map(PathBuf::from)
            .collect(),
    })
}

fn first<'a>(values: &'a BTreeMap<&str, Vec<&'a str>>, key: &str) -> Option<&'a str> {
    values.get(key)?.first().copied()
}

fn visit_config(
    path: &Path,
    aliases: &mut BTreeSet<String>,
    visited: &mut HashSet<PathBuf>,
    depth: usize,
) -> Result<(), String> {
    if depth > 16 {
        return Err("SSH config Include nesting exceeds 16 levels.".to_string());
    }
    let path = expand_tilde(path);
    let canonical = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    if !visited.insert(canonical) {
        return Ok(());
    }
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("Could not read {}: {err}", path.display())),
    };
    for raw in contents.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let Some((keyword, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if keyword.eq_ignore_ascii_case("host") {
            for value in rest.split_whitespace() {
                if validate_literal_alias(value).is_ok() {
                    aliases.insert(value.to_string());
                }
            }
        } else if keyword.eq_ignore_ascii_case("include") {
            for pattern in rest.split_whitespace() {
                for included in expand_include(&path, pattern)? {
                    visit_config(&included, aliases, visited, depth + 1)?;
                }
            }
        }
    }
    Ok(())
}

fn validate_literal_alias(alias: &str) -> Result<(), String> {
    if alias.is_empty()
        || alias.starts_with('!')
        || alias.chars().any(|ch| matches!(ch, '*' | '?' | '[' | ']'))
    {
        return Err("SSH import only accepts literal positive Host aliases.".to_string());
    }
    Ok(())
}

fn strip_comment(line: &str) -> &str {
    line.split('#').next().unwrap_or(line)
}

fn expand_include(config_path: &Path, pattern: &str) -> Result<Vec<PathBuf>, String> {
    let raw = PathBuf::from(pattern);
    let pattern = if pattern.starts_with('~') {
        expand_tilde(&raw)
    } else if raw.is_absolute() {
        raw
    } else {
        config_path.parent().unwrap_or(Path::new(".")).join(raw)
    };
    let Some(file_pattern) = pattern.file_name().and_then(|name| name.to_str()) else {
        return Ok(Vec::new());
    };
    if !file_pattern.contains('*') {
        return Ok(vec![pattern]);
    }
    if file_pattern.matches('*').count() != 1 {
        return Err(format!(
            "Unsupported SSH Include pattern: {}",
            pattern.display()
        ));
    }
    let (prefix, suffix) = file_pattern.split_once('*').unwrap();
    let parent = pattern.parent().unwrap_or(Path::new("."));
    let mut matches = fs::read_dir(parent)
        .map_err(|err| {
            format!(
                "Could not read SSH Include directory {}: {err}",
                parent.display()
            )
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix) && name.ends_with(suffix))
        })
        .collect::<Vec<_>>();
    matches.sort();
    Ok(matches)
}

fn expand_tilde(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        return dirs::home_dir().unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = text.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn imports_only_literal_positive_aliases_and_includes() {
        let temp = TempDir::new().unwrap();
        let include_dir = temp.path().join("config.d");
        fs::create_dir_all(&include_dir).unwrap();
        fs::write(
            temp.path().join("config"),
            "Host dev *.internal !blocked\n  User me\nInclude config.d/*.conf\n",
        )
        .unwrap();
        fs::write(include_dir.join("lab.conf"), "Host homelab\n").unwrap();
        assert_eq!(
            discover_literal_aliases(&temp.path().join("config")).unwrap(),
            vec!["dev".to_string(), "homelab".to_string()]
        );
    }

    #[test]
    fn preview_parser_keeps_only_connection_metadata() {
        let preview = parse_preview(
            "lab",
            "host lab\nhostname 10.0.0.8\nuser dev\nport 2222\nproxyjump bastion\nidentityfile ~/.ssh/id_ed25519\n",
        )
        .unwrap();
        assert_eq!(preview.alias, "lab");
        assert_eq!(preview.hostname.as_deref(), Some("10.0.0.8"));
        assert_eq!(preview.port, Some(2222));
        assert_eq!(preview.identity_files.len(), 1);
    }
}
