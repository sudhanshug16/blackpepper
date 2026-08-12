//! Repository state for the workspaces a client is currently attached to.
//!
//! This runs on the host because that is where the checkout lives — a client
//! attached over SSH cannot stat the working tree. Git is asked on every
//! refresh (cheap, local); `gh` is asked at most once per repository per
//! `PR_REFRESH`, because it is a network round trip and the answer changes on
//! human timescales.

use crate::core::{PullRequestState, PullRequestSummary, WorkspaceOverview};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const PR_REFRESH: Duration = Duration::from_secs(300);

type PullRequestCache = HashMap<String, (Instant, Option<PullRequestSummary>)>;

fn cache() -> &'static Mutex<PullRequestCache> {
    static CACHE: OnceLock<Mutex<PullRequestCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Git and pull-request state for one checkout. Returns the default overview
/// when the path is not a repository, which renders as no repository segment
/// at all rather than as an error.
pub(super) fn overview(root_path: &str) -> WorkspaceOverview {
    let path = Path::new(root_path);
    let Some(status) = run_git(&["status", "--porcelain=2", "-b"], path) else {
        return WorkspaceOverview::default();
    };
    let (ahead, behind) = parse_divergence(&status);
    WorkspaceOverview {
        head: parse_head(&status),
        dirty: parse_dirty(&status),
        ahead,
        behind,
        pull_request: pull_request(root_path, path),
        active_tab: None,
        tab_count: None,
    }
}

fn run_git(args: &[&str], cwd: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_head(output: &str) -> Option<String> {
    let head = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("# branch.head "))?
        .trim();
    match head {
        "" | "(unknown)" => None,
        "(detached)" => Some("detached".to_owned()),
        head => Some(head.to_owned()),
    }
}

/// Any non-header line in porcelain v2 is a changed, untracked, or unmerged
/// path.
fn parse_dirty(output: &str) -> bool {
    output
        .lines()
        .map(str::trim)
        .any(|line| !line.is_empty() && !line.starts_with('#'))
}

fn parse_divergence(output: &str) -> (u32, u32) {
    let Some(line) = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("# branch.ab "))
    else {
        return (0, 0);
    };
    let mut ahead = 0;
    let mut behind = 0;
    for part in line.split_whitespace() {
        if let Some(value) = part.strip_prefix('+') {
            ahead = value.parse().unwrap_or(0);
        } else if let Some(value) = part.strip_prefix('-') {
            behind = value.parse().unwrap_or(0);
        }
    }
    (ahead, behind)
}

/// Cached `gh pr view`. A missing `gh`, an unauthenticated one, or a branch
/// with no PR all cache as "no pull request" so the failure costs one lookup
/// per interval rather than one per refresh.
fn pull_request(key: &str, cwd: &Path) -> Option<PullRequestSummary> {
    let mut cache = cache().lock().ok()?;
    if let Some((fetched_at, summary)) = cache.get(key) {
        if fetched_at.elapsed() < PR_REFRESH {
            return summary.clone();
        }
    }
    let summary = fetch_pull_request(cwd);
    cache.insert(key.to_owned(), (Instant::now(), summary.clone()));
    summary
}

#[derive(Deserialize)]
struct GhPullRequest {
    number: u32,
    state: String,
    #[serde(rename = "mergedAt")]
    merged_at: Option<String>,
    #[serde(rename = "isDraft")]
    is_draft: Option<bool>,
}

fn fetch_pull_request(cwd: &Path) -> Option<PullRequestSummary> {
    let output = Command::new("gh")
        .args(["pr", "view", "--json", "number,state,mergedAt,isDraft"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let parsed: GhPullRequest = serde_json::from_slice(&output.stdout).ok()?;
    let state = if parsed.merged_at.is_some() {
        PullRequestState::Merged
    } else if parsed.is_draft.unwrap_or(false) {
        PullRequestState::Draft
    } else {
        match parsed.state.trim().to_ascii_lowercase().as_str() {
            "open" => PullRequestState::Open,
            "merged" => PullRequestState::Merged,
            _ => PullRequestState::Closed,
        }
    };
    Some(PullRequestSummary {
        number: parsed.number,
        state,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_dirty, parse_divergence, parse_head};

    const CLEAN: &str =
        "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -0\n";

    #[test]
    fn porcelain_header_yields_branch_and_divergence() {
        assert_eq!(parse_head(CLEAN).as_deref(), Some("main"));
        assert_eq!(parse_divergence(CLEAN), (2, 0));
        assert!(!parse_dirty(CLEAN));
    }

    #[test]
    fn any_entry_line_marks_the_tree_dirty() {
        let output = format!("{CLEAN}1 .M N... 100644 100644 100644 abc def src/main.rs\n");
        assert!(parse_dirty(&output));
    }

    #[test]
    fn detached_head_is_named_rather_than_dropped() {
        let output = "# branch.oid abc\n# branch.head (detached)\n";
        assert_eq!(parse_head(output).as_deref(), Some("detached"));
        assert_eq!(parse_divergence(output), (0, 0));
    }
}
