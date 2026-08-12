//! Host-local repository identity detection used by both the client and
//! helper. Ambiguous remotes intentionally fall back to host-scoped identity.

use crate::core::{HostId, RepositoryIdentity};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedRepository {
    pub identity: RepositoryIdentity,
    pub git_common_dir: PathBuf,
    pub primary_remote: Option<String>,
}

pub fn detect_local(path: &Path, host_id: HostId) -> Result<Option<DetectedRepository>, String> {
    let common = run_git(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    if !common.status.success() {
        return Ok(None);
    }
    let common_dir = first_line(&common.stdout)
        .map(PathBuf::from)
        .ok_or_else(|| "Git returned an empty common directory.".to_string())?;
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        path.join(common_dir)
    };
    let common_dir = std::fs::canonicalize(&common_dir).unwrap_or(common_dir);
    let remote = primary_remote(path)?;
    let identity = match remote
        .as_deref()
        .and_then(|url| RepositoryIdentity::remote(url).ok())
    {
        Some(identity) => identity,
        None => RepositoryIdentity::local(host_id, common_dir.to_string_lossy().into_owned())
            .map_err(|err| err.to_string())?,
    };
    Ok(Some(DetectedRepository {
        identity,
        git_common_dir: common_dir,
        primary_remote: remote,
    }))
}

fn primary_remote(path: &Path) -> Result<Option<String>, String> {
    let output = run_git(path, &["remote"])?;
    if !output.status.success() {
        return Ok(None);
    }
    let remotes = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let selected = if remotes.iter().any(|remote| remote == "origin") {
        Some("origin")
    } else if remotes.len() == 1 {
        remotes.first().map(String::as_str)
    } else {
        None
    };
    let Some(selected) = selected else {
        return Ok(None);
    };
    let output = run_git(path, &["remote", "get-url", selected])?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(first_line(&output.stdout).map(ToOwned::to_owned))
}

fn run_git(path: &Path, args: &[&str]) -> Result<Output, String> {
    Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .map_err(|err| format!("Could not run git in {}: {err}", path.display()))
}

fn first_line(bytes: &[u8]) -> Option<&str> {
    std::str::from_utf8(bytes)
        .ok()?
        .lines()
        .next()
        .map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn git(path: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(path)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn non_git_folder_has_no_repository_identity() {
        let temp = TempDir::new().unwrap();
        assert!(detect_local(temp.path(), HostId::new()).unwrap().is_none());
    }

    #[test]
    fn remote_groups_across_hosts_and_schemes() {
        let temp = TempDir::new().unwrap();
        git(temp.path(), &["init", "-b", "main"]);
        git(
            temp.path(),
            &["remote", "add", "origin", "git@github.com:acme/app.git"],
        );
        let first = detect_local(temp.path(), HostId::new()).unwrap().unwrap();
        git(
            temp.path(),
            &["remote", "set-url", "origin", "https://github.com/acme/app"],
        );
        let second = detect_local(temp.path(), HostId::new()).unwrap().unwrap();
        assert_eq!(
            first.identity.repository_id(),
            second.identity.repository_id()
        );
    }

    #[test]
    fn ambiguous_non_origin_remotes_stay_host_local() {
        let temp = TempDir::new().unwrap();
        git(temp.path(), &["init", "-b", "main"]);
        git(
            temp.path(),
            &["remote", "add", "upstream", "https://example.com/a/r"],
        );
        git(
            temp.path(),
            &["remote", "add", "fork", "https://example.com/b/r"],
        );
        let host = HostId::new();
        let detected = detect_local(temp.path(), host).unwrap().unwrap();
        assert_eq!(detected.primary_remote, None);
        assert!(matches!(
            detected.identity,
            RepositoryIdentity::Local { .. }
        ));
    }

    #[test]
    fn worktrees_share_git_common_dir() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        fs::write(repo.join("README.md"), "x").unwrap();
        git(&repo, &["add", "."]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "init",
            ],
        );
        let worktree = temp.path().join("feature-worktree");
        git(
            &repo,
            &[
                "worktree",
                "add",
                worktree.to_str().unwrap(),
                "-b",
                "feature",
            ],
        );
        let host = HostId::new();
        let main = detect_local(&repo, host).unwrap().unwrap();
        let feature = detect_local(&worktree, host).unwrap().unwrap();
        assert_eq!(
            main.identity.repository_id(),
            feature.identity.repository_id()
        );
    }
}
