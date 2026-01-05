use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct ExecResult {
    pub ok: bool,
    #[allow(dead_code)]
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn run_git(args: &[&str], cwd: &Path) -> ExecResult {
    let output = Command::new("git").args(args).current_dir(cwd).output();
    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let exit_code = out.status.code().unwrap_or(-1);
            ExecResult {
                ok: out.status.success(),
                exit_code,
                stdout,
                stderr,
            }
        }
        Err(err) => ExecResult {
            ok: false,
            exit_code: -1,
            stdout: String::new(),
            stderr: err.to_string(),
        },
    }
}

pub fn resolve_repo_root(cwd: &Path) -> Option<PathBuf> {
    let result = run_git(["rev-parse", "--git-common-dir"].as_ref(), cwd);
    if !result.ok {
        return None;
    }

    let git_common = result.stdout.trim();
    if git_common.is_empty() {
        return None;
    }

    let git_path = Path::new(git_common);
    let resolved = if git_path.is_absolute() {
        git_path.to_path_buf()
    } else {
        cwd.join(git_path)
    };

    let canonical = std::fs::canonicalize(&resolved).unwrap_or(resolved);
    canonical.parent().map(|path| path.to_path_buf())
}
