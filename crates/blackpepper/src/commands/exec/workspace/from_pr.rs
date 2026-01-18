//! Create workspace from an existing pull request.

use std::process::Command;

use serde::Deserialize;

use crate::git::resolve_repo_root;

use super::super::{CommandContext, CommandResult};
use super::from_branch::workspace_from_branch;

/// Create a workspace from an existing pull request.
///
/// Fetches PR info via `gh` CLI, extracts the branch name, and delegates
/// to `workspace_from_branch`. Supports PR numbers (e.g., `123`) or full
/// URLs (e.g., `https://github.com/org/repo/pull/123`).
pub(crate) fn workspace_from_pr(args: &[String], ctx: &CommandContext) -> CommandResult {
    let Some(pr_ref) = args.first() else {
        return CommandResult {
            ok: false,
            message: "Usage: :workspace from-pr <number-or-url>".to_string(),
            data: None,
        };
    };

    let repo_root = ctx
        .repo_root
        .clone()
        .or_else(|| resolve_repo_root(&ctx.cwd))
        .ok_or_else(|| CommandResult {
            ok: false,
            message: "Not inside a git repository.".to_string(),
            data: None,
        });
    let repo_root = match repo_root {
        Ok(root) => root,
        Err(result) => return result,
    };

    // Parse PR reference (extract number from URL if needed)
    let pr_identifier = parse_pr_ref(pr_ref);

    // Fetch PR info via gh CLI
    let pr_info = match fetch_pr_info(&repo_root, &pr_identifier) {
        Ok(info) => info,
        Err(err) => {
            return CommandResult {
                ok: false,
                message: err,
                data: None,
            }
        }
    };

    // Check for forked PRs
    if pr_info.is_fork {
        return CommandResult {
            ok: false,
            message: format!(
                "PR #{} is from a fork ({}). Forked PRs are not supported yet.",
                pr_info.number, pr_info.head_repository_owner
            ),
            data: None,
        };
    }

    // Delegate to from_branch with the PR's branch name
    let branch_args = vec![pr_info.head_ref_name];
    workspace_from_branch(&branch_args, ctx)
}

/// Parse a PR reference, extracting the number from a URL if needed.
///
/// Supports:
/// - Plain numbers: `123`
/// - GitHub URLs: `https://github.com/org/repo/pull/123`
fn parse_pr_ref(pr_ref: &str) -> String {
    let trimmed = pr_ref.trim();

    // Try to extract PR number from URL
    if trimmed.contains("github.com") && trimmed.contains("/pull/") {
        if let Some(number) = trimmed.rsplit("/pull/").next() {
            // Handle trailing paths like /files, /commits, etc.
            let number = number.split('/').next().unwrap_or(number);
            let number = number.split('?').next().unwrap_or(number);
            let number = number.split('#').next().unwrap_or(number);
            if !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()) {
                return number.to_string();
            }
        }
    }

    // Return as-is (assume it's a PR number)
    trimmed.to_string()
}

#[derive(Debug)]
struct PrInfo {
    number: u32,
    head_ref_name: String,
    head_repository_owner: String,
    is_fork: bool,
}

#[derive(Debug, Deserialize)]
struct GhPrView {
    number: u32,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "headRepositoryOwner")]
    head_repository_owner: Option<GhOwner>,
    #[serde(rename = "baseRepository")]
    base_repository: Option<GhRepository>,
}

#[derive(Debug, Deserialize)]
struct GhOwner {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GhRepository {
    owner: GhOwner,
}

fn fetch_pr_info(cwd: &std::path::Path, pr_ref: &str) -> Result<PrInfo, String> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            pr_ref,
            "--json",
            "number,headRefName,headRepositoryOwner,baseRepository",
        ])
        .current_dir(cwd)
        .output();

    match output {
        Ok(out) => {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                parse_pr_info(&stdout)
            } else {
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let message = first_non_empty_line(&stderr)
                    .or_else(|| first_non_empty_line(&stdout))
                    .unwrap_or_else(|| format!("Failed to fetch PR {pr_ref}."));
                Err(message)
            }
        }
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                Err("gh CLI not found. Install it from https://cli.github.com/".to_string())
            } else {
                Err(format!("Failed to run gh: {err}"))
            }
        }
    }
}

fn parse_pr_info(raw: &str) -> Result<PrInfo, String> {
    let parsed: GhPrView = serde_json::from_str(raw.trim())
        .map_err(|err| format!("Invalid gh pr view output: {err}"))?;

    let head_owner = parsed
        .head_repository_owner
        .map(|o| o.login)
        .unwrap_or_default();
    let base_owner = parsed
        .base_repository
        .map(|r| r.owner.login)
        .unwrap_or_default();

    // It's a fork if the head owner differs from the base owner
    let is_fork = !head_owner.is_empty()
        && !base_owner.is_empty()
        && head_owner.to_lowercase() != base_owner.to_lowercase();

    Ok(PrInfo {
        number: parsed.number,
        head_ref_name: parsed.head_ref_name,
        head_repository_owner: head_owner,
        is_fork,
    })
}

fn first_non_empty_line(message: &str) -> Option<String> {
    message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
}

#[cfg(test)]
mod tests {
    use super::{parse_pr_info, parse_pr_ref};

    #[test]
    fn parse_pr_ref_number() {
        assert_eq!(parse_pr_ref("123"), "123");
        assert_eq!(parse_pr_ref("  456  "), "456");
    }

    #[test]
    fn parse_pr_ref_github_url() {
        assert_eq!(parse_pr_ref("https://github.com/org/repo/pull/789"), "789");
        assert_eq!(
            parse_pr_ref("https://github.com/org/repo/pull/123/files"),
            "123"
        );
        assert_eq!(
            parse_pr_ref("https://github.com/org/repo/pull/456?tab=commits"),
            "456"
        );
        assert_eq!(
            parse_pr_ref("https://github.com/org/repo/pull/789#discussion"),
            "789"
        );
    }

    #[test]
    fn parse_pr_info_same_repo() {
        let json = r#"{
            "number": 42,
            "headRefName": "feature-branch",
            "headRepositoryOwner": {"login": "myorg"},
            "baseRepository": {"owner": {"login": "myorg"}}
        }"#;
        let info = parse_pr_info(json).expect("should parse");
        assert_eq!(info.number, 42);
        assert_eq!(info.head_ref_name, "feature-branch");
        assert_eq!(info.head_repository_owner, "myorg");
        assert!(!info.is_fork, "same owner should not be a fork");
    }

    #[test]
    fn parse_pr_info_fork_detected() {
        let json = r#"{
            "number": 99,
            "headRefName": "contributor-fix",
            "headRepositoryOwner": {"login": "contributor"},
            "baseRepository": {"owner": {"login": "mainorg"}}
        }"#;
        let info = parse_pr_info(json).expect("should parse");
        assert_eq!(info.number, 99);
        assert_eq!(info.head_ref_name, "contributor-fix");
        assert_eq!(info.head_repository_owner, "contributor");
        assert!(info.is_fork, "different owner should be a fork");
    }

    #[test]
    fn parse_pr_info_case_insensitive_owner() {
        let json = r#"{
            "number": 1,
            "headRefName": "branch",
            "headRepositoryOwner": {"login": "MyOrg"},
            "baseRepository": {"owner": {"login": "myorg"}}
        }"#;
        let info = parse_pr_info(json).expect("should parse");
        assert!(
            !info.is_fork,
            "case-insensitive owner match should not be a fork"
        );
    }

    #[test]
    fn parse_pr_info_missing_head_owner() {
        let json = r#"{
            "number": 1,
            "headRefName": "branch",
            "headRepositoryOwner": null,
            "baseRepository": {"owner": {"login": "myorg"}}
        }"#;
        let info = parse_pr_info(json).expect("should parse");
        assert!(
            !info.is_fork,
            "missing head owner should not be detected as fork"
        );
    }

    #[test]
    fn parse_pr_info_missing_base_owner() {
        let json = r#"{
            "number": 1,
            "headRefName": "branch",
            "headRepositoryOwner": {"login": "myorg"},
            "baseRepository": null
        }"#;
        let info = parse_pr_info(json).expect("should parse");
        assert!(
            !info.is_fork,
            "missing base owner should not be detected as fork"
        );
    }
}
