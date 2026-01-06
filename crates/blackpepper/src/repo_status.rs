use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Deserialize;

use crate::events::AppEvent;
use crate::git::{git_common_dir, run_git};

const STATUS_DEBOUNCE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Default)]
pub struct RepoStatus {
    pub head: Option<String>,
    pub divergence: Option<Divergence>,
    pub pr: PrStatus,
}

#[derive(Debug, Clone)]
pub struct Divergence {
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone)]
pub enum PrStatus {
    None,
    Info(PrInfo),
    Error(PrError),
}

impl Default for PrStatus {
    fn default() -> Self {
        PrStatus::None
    }
}

#[derive(Debug, Clone)]
pub struct PrInfo {
    pub number: u32,
    pub title: String,
    pub state: PrState,
}

#[derive(Debug, Clone)]
pub enum PrState {
    Open,
    Closed,
    Merged,
    Draft,
}

#[derive(Debug, Clone)]
pub struct PrError {
    pub kind: PrErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrErrorKind {
    MissingCli,
    Other,
}

#[derive(Debug)]
pub(crate) enum RepoStatusSignal {
    Request(PathBuf),
    Notify,
}

pub(crate) fn spawn_repo_status_worker(event_tx: Sender<AppEvent>) -> Sender<RepoStatusSignal> {
    let (signal_tx, signal_rx) = mpsc::channel();
    let worker_tx = signal_tx.clone();
    std::thread::spawn(move || run_repo_status_worker(signal_rx, worker_tx, event_tx));
    signal_tx
}

fn run_repo_status_worker(
    signal_rx: Receiver<RepoStatusSignal>,
    signal_tx: Sender<RepoStatusSignal>,
    event_tx: Sender<AppEvent>,
) {
    let mut current_cwd: Option<PathBuf> = None;
    let mut watcher: Option<RecommendedWatcher> = None;
    let mut watched_dir: Option<PathBuf> = None;

    loop {
        let signal = match signal_rx.recv() {
            Ok(signal) => signal,
            Err(_) => break,
        };

        match signal {
            RepoStatusSignal::Request(cwd) => {
                current_cwd = Some(cwd);
            }
            RepoStatusSignal::Notify => {}
        }

        if let Some(cwd) = current_cwd.as_ref() {
            let next_git_dir = git_common_dir(cwd);
            if next_git_dir != watched_dir {
                watcher = None;
                watched_dir = None;
                if let Some(path) = next_git_dir.clone() {
                    if let Ok(mut next_watcher) = new_repo_watcher(signal_tx.clone()) {
                        if next_watcher.watch(&path, RecursiveMode::Recursive).is_ok() {
                            watcher = Some(next_watcher);
                            watched_dir = Some(path);
                        }
                    }
                }
            }
        }

        let _ = watcher.as_ref();
        // Coalesce bursts of git changes so we only compute once.
        std::thread::sleep(STATUS_DEBOUNCE);
        while let Ok(extra) = signal_rx.try_recv() {
            if let RepoStatusSignal::Request(cwd) = extra {
                current_cwd = Some(cwd);
            }
        }

        if let Some(cwd) = current_cwd.as_ref() {
            let status = compute_repo_status(cwd);
            let _ = event_tx.send(AppEvent::RepoStatusUpdated {
                cwd: cwd.clone(),
                status,
            });
        }
    }
}

fn new_repo_watcher(signal_tx: Sender<RepoStatusSignal>) -> notify::Result<RecommendedWatcher> {
    notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = signal_tx.send(RepoStatusSignal::Notify);
        }
    })
}

fn compute_repo_status(cwd: &Path) -> RepoStatus {
    let status = run_git(["status", "--porcelain=2", "-b"].as_ref(), cwd);
    if !status.ok {
        return RepoStatus::default();
    }

    let head = parse_branch_head(&status.stdout);
    let divergence = parse_divergence(&status.stdout);
    let pr = fetch_pr_status(cwd);
    RepoStatus {
        head,
        divergence,
        pr,
    }
}

fn parse_branch_head(output: &str) -> Option<String> {
    for line in output.lines() {
        let line = line.trim();
        if !line.starts_with("# branch.head ") {
            continue;
        }
        let head = line.trim_start_matches("# branch.head ").trim();
        if head.is_empty() || head == "(unknown)" {
            return None;
        }
        if head == "(detached)" {
            return Some("detached".to_string());
        }
        return Some(head.to_string());
    }
    None
}

fn parse_divergence(output: &str) -> Option<Divergence> {
    for line in output.lines() {
        let line = line.trim();
        if !line.starts_with("# branch.ab ") {
            continue;
        }
        let mut ahead: Option<u32> = None;
        let mut behind: Option<u32> = None;
        for part in line.split_whitespace() {
            if let Some(value) = part.strip_prefix('+') {
                ahead = value.parse::<u32>().ok();
            } else if let Some(value) = part.strip_prefix('-') {
                behind = value.parse::<u32>().ok();
            }
        }
        let ahead = ahead.unwrap_or(0);
        let behind = behind.unwrap_or(0);
        if ahead == 0 && behind == 0 {
            return None;
        }
        return Some(Divergence { ahead, behind });
    }
    None
}

fn fetch_pr_status(cwd: &Path) -> PrStatus {
    let output = Command::new("gh")
        .args(["pr", "view", "--json", "number,title,state,mergedAt,isDraft"])
        .current_dir(cwd)
        .output();

    match output {
        Ok(out) => {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                match parse_pr_view(&stdout) {
                    Ok(info) => PrStatus::Info(info),
                    Err(err) => PrStatus::Error(PrError {
                        kind: PrErrorKind::Other,
                        message: err,
                    }),
                }
            } else {
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                if is_no_pr_error(&stderr) || is_no_pr_error(&stdout) {
                    return PrStatus::None;
                }
                let message =
                    first_non_empty_line(&stderr).or_else(|| first_non_empty_line(&stdout));
                let message = message.unwrap_or_else(|| "gh pr view failed.".to_string());
                PrStatus::Error(PrError {
                    kind: PrErrorKind::Other,
                    message,
                })
            }
        }
        Err(err) => {
            let kind = if err.kind() == ErrorKind::NotFound {
                PrErrorKind::MissingCli
            } else {
                PrErrorKind::Other
            };
            let message = if kind == PrErrorKind::MissingCli {
                "gh cli not available".to_string()
            } else {
                err.to_string()
            };
            PrStatus::Error(PrError { kind, message })
        }
    }
}

fn is_no_pr_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("no pull requests found")
}

fn first_non_empty_line(message: &str) -> Option<String> {
    message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
}

#[derive(Debug, Deserialize)]
struct GhPrView {
    number: u32,
    title: String,
    state: String,
    #[serde(rename = "mergedAt")]
    merged_at: Option<String>,
    #[serde(rename = "isDraft")]
    is_draft: Option<bool>,
}

fn parse_pr_view(raw: &str) -> Result<PrInfo, String> {
    let parsed: GhPrView = serde_json::from_str(raw.trim())
        .map_err(|err| format!("Invalid gh pr view output: {err}"))?;
    let state = if parsed.is_draft.unwrap_or(false) {
        PrState::Draft
    } else if parsed.merged_at.is_some() {
        PrState::Merged
    } else {
        match parsed.state.trim().to_ascii_lowercase().as_str() {
            "open" => PrState::Open,
            "closed" => PrState::Closed,
            _ => PrState::Closed,
        }
    };
    let title = parsed.title.lines().next().unwrap_or("").trim().to_string();
    Ok(PrInfo {
        number: parsed.number,
        title,
        state,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_branch_head, parse_divergence, parse_pr_view, Divergence, PrState};

    #[test]
    fn parse_divergence_extracts_counts() {
        let output = "# branch.oid 0123\n# branch.ab +2 -1\n";
        let result = parse_divergence(output).expect("divergence");
        assert_eq!(result.ahead, 2);
        assert_eq!(result.behind, 1);
    }

    #[test]
    fn parse_divergence_ignores_zero() {
        let output = "# branch.ab +0 -0\n";
        assert!(parse_divergence(output).is_none());
    }

    #[test]
    fn parse_branch_head_prefers_name() {
        let output = "# branch.head main\n";
        assert_eq!(parse_branch_head(output), Some("main".to_string()));
    }

    #[test]
    fn parse_branch_head_handles_detached() {
        let output = "# branch.head (detached)\n";
        assert_eq!(parse_branch_head(output), Some("detached".to_string()));
    }

    #[test]
    fn parse_pr_view_merges_state() {
        let raw =
            r#"{"number":12,"title":"Ship it","state":"CLOSED","mergedAt":"2024-01-01T00:00:00Z"}"#;
        let info = parse_pr_view(raw).expect("parse ok");
        assert_eq!(info.number, 12);
        assert_eq!(info.title, "Ship it");
        assert!(matches!(info.state, PrState::Merged));
    }

    #[test]
    fn parse_pr_view_closed_state() {
        let raw = r#"{"number":12,"title":"Nope","state":"CLOSED","mergedAt":null}"#;
        let info = parse_pr_view(raw).expect("parse ok");
        assert!(matches!(info.state, PrState::Closed));
    }

    #[test]
    fn parse_pr_view_open_state() {
        let raw = r#"{"number":12,"title":"Yep","state":"OPEN","mergedAt":null}"#;
        let info = parse_pr_view(raw).expect("parse ok");
        assert!(matches!(info.state, PrState::Open));
    }

    #[test]
    fn parse_pr_view_draft_state() {
        let raw =
            r#"{"number":12,"title":"Draft","state":"OPEN","mergedAt":null,"isDraft":true}"#;
        let info = parse_pr_view(raw).expect("parse ok");
        assert!(matches!(info.state, PrState::Draft));
    }

    #[test]
    fn divergence_struct_stays_simple() {
        let divergence = Divergence {
            ahead: 1,
            behind: 0,
        };
        assert_eq!(divergence.ahead, 1);
        assert_eq!(divergence.behind, 0);
    }
}
