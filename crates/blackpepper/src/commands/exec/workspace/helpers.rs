use std::collections::HashSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::animals::ANIMAL_NAMES;
use crate::git::{run_git, ExecResult};
use crate::workspaces::is_valid_workspace_name;

pub(super) fn format_exec_output(result: &ExecResult) -> String {
    let stdout = result.stdout.trim();
    let stderr = result.stderr.trim();
    [stdout, stderr]
        .iter()
        .filter(|text| !text.is_empty())
        .map(|text| text.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn branch_exists(repo_root: &Path, name: &str) -> bool {
    let result = run_git(
        [
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ]
        .as_ref(),
        repo_root,
    );
    result.ok
}

pub(super) fn normalize_workspace_name(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in raw.chars() {
        let ch = ch.to_ascii_lowercase();
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            out.push(ch);
            last_dash = false;
        } else if ch == '-' {
            if !out.is_empty() && !last_dash {
                out.push('-');
                last_dash = true;
            }
        } else if !out.is_empty() && !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

pub(crate) fn unique_animal_names() -> Vec<String> {
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for name in ANIMAL_NAMES {
        if !is_valid_workspace_name(name) {
            continue;
        }
        if seen.insert(*name) {
            names.push((*name).to_string());
        }
    }
    names
}

pub(crate) fn pick_unused_animal_name(used: &HashSet<String>) -> Option<String> {
    let unused: Vec<String> = unique_animal_names()
        .into_iter()
        .filter(|name| !used.contains(name))
        .collect();
    if unused.is_empty() {
        return None;
    }
    // Simple pseudo-random selection based on time
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let index = (nanos % unused.len() as u128) as usize;
    unused.get(index).cloned()
}
