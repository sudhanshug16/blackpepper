use std::path::Path;
use std::process::Command;

use crate::git::ExecResult;

pub const DEFAULT_PROVIDER: &str = "github";

pub fn create_pr(
    provider: &str,
    cwd: &Path,
    title: &str,
    body: &str,
) -> Result<ExecResult, String> {
    if provider.eq_ignore_ascii_case(DEFAULT_PROVIDER) {
        Ok(run_github_create(cwd, title, body))
    } else {
        Err(format!("Unknown upstream provider: {provider}."))
    }
}

fn run_github_create(cwd: &Path, title: &str, body: &str) -> ExecResult {
    let output = Command::new("gh")
        .args(["pr", "create", "--title"])
        .arg(title)
        .args(["--body"])
        .arg(body)
        .current_dir(cwd)
        .output();
    match output {
        Ok(out) => ExecResult {
            ok: out.status.success(),
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        },
        Err(err) => ExecResult {
            ok: false,
            exit_code: -1,
            stdout: String::new(),
            stderr: err.to_string(),
        },
    }
}
