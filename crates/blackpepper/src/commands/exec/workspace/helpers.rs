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

/// Normalize a raw name to a valid workspace name.
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

#[cfg(test)]
mod tests {
    use super::{normalize_workspace_name, pick_unused_animal_name, unique_animal_names};
    use std::collections::HashSet;

    #[test]
    fn normalize_simple_name() {
        assert_eq!(normalize_workspace_name("feature"), "feature");
    }

    #[test]
    fn normalize_with_slashes() {
        assert_eq!(normalize_workspace_name("feature/auth"), "feature-auth");
        assert_eq!(normalize_workspace_name("fix/bug/123"), "fix-bug-123");
    }

    #[test]
    fn normalize_with_underscores() {
        assert_eq!(normalize_workspace_name("my_feature"), "my-feature");
    }

    #[test]
    fn normalize_uppercase() {
        assert_eq!(normalize_workspace_name("FEATURE"), "feature");
        assert_eq!(normalize_workspace_name("MyFeature"), "myfeature");
    }

    #[test]
    fn normalize_mixed_special_chars() {
        assert_eq!(
            normalize_workspace_name("feature/auth_v2@test"),
            "feature-auth-v2-test"
        );
    }

    #[test]
    fn normalize_leading_trailing_special() {
        assert_eq!(normalize_workspace_name("-feature-"), "feature");
        assert_eq!(normalize_workspace_name("/feature/"), "feature");
        assert_eq!(normalize_workspace_name("--feature--"), "feature");
    }

    #[test]
    fn normalize_consecutive_dashes() {
        assert_eq!(normalize_workspace_name("a--b"), "a-b");
        assert_eq!(normalize_workspace_name("a///b"), "a-b");
    }

    #[test]
    fn normalize_empty_string() {
        assert_eq!(normalize_workspace_name(""), "");
        assert_eq!(normalize_workspace_name("---"), "");
        assert_eq!(normalize_workspace_name("///"), "");
    }

    #[test]
    fn normalize_numbers() {
        assert_eq!(normalize_workspace_name("123"), "123");
        assert_eq!(normalize_workspace_name("v2.0"), "v2-0");
    }

    #[test]
    fn pick_unused_returns_name() {
        let used = HashSet::new();
        let name = pick_unused_animal_name(&used).expect("should pick a name");
        assert!(!name.is_empty());
    }

    #[test]
    fn unique_animal_names_are_valid() {
        for name in unique_animal_names() {
            assert!(
                crate::workspaces::is_valid_workspace_name(&name),
                "animal name '{}' should be valid",
                name
            );
        }
    }

    #[test]
    fn pick_unused_excludes_used() {
        // Mark all but one as used
        let all_names: Vec<String> = unique_animal_names()
            .into_iter()
            .collect();
        let mut used: HashSet<String> = all_names.iter().cloned().collect();
        let keep = used.iter().next().cloned().unwrap();
        used.remove(&keep);

        let picked = pick_unused_animal_name(&used).expect("should pick a name");
        assert_eq!(picked, keep);
    }
}
