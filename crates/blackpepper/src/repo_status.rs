use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Deserialize;

use crate::events::AppEvent;
use crate::git::{git_common_dir, run_git};

// Notify events are debounced; explicit requests should compute immediately.
const NOTIFY_DEBOUNCE: Duration = Duration::from_secs(5);
// Rate-limit gh PR lookups globally; reuse the last known status when limited.
const PR_STATUS_RATE_LIMIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Default)]
pub struct RepoStatus {
    pub head: Option<String>,
    pub dirty: bool,
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
    let mut notify_deadline: Option<Instant> = None;
    let mut last_pr_fetch: Option<Instant> = None;
    let mut last_pr_status: Option<PrStatus> = None;

    loop {
        let signal = match notify_deadline {
            Some(deadline) => {
                let timeout = deadline.saturating_duration_since(Instant::now());
                match signal_rx.recv_timeout(timeout) {
                    Ok(signal) => Some(signal),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            None => match signal_rx.recv() {
                Ok(signal) => Some(signal),
                Err(_) => break,
            },
        };

        let mut compute_now = false;
        let mut deadline_reached = false;

        match signal {
            Some(RepoStatusSignal::Request(cwd)) => {
                current_cwd = Some(cwd);
                notify_deadline = None;
                compute_now = true;
            }
            Some(RepoStatusSignal::Notify) => {
                notify_deadline = Some(Instant::now() + NOTIFY_DEBOUNCE);
            }
            None => {
                deadline_reached = notify_deadline
                    .map(|deadline| Instant::now() >= deadline)
                    .unwrap_or(false);
                if deadline_reached {
                    notify_deadline = None;
                }
            }
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
        if compute_now || deadline_reached {
            if let Some(cwd) = current_cwd.as_ref() {
                let status = compute_repo_status(cwd, &mut last_pr_fetch, &mut last_pr_status);
                let _ = event_tx.send(AppEvent::RepoStatusUpdated {
                    cwd: cwd.clone(),
                    status,
                });
            }
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

fn compute_repo_status(
    cwd: &Path,
    last_pr_fetch: &mut Option<Instant>,
    last_pr_status: &mut Option<PrStatus>,
) -> RepoStatus {
    let status = run_git(["status", "--porcelain=2", "-b"].as_ref(), cwd);
    if !status.ok {
        return RepoStatus::default();
    }

    let head = parse_branch_head(&status.stdout);
    let dirty = parse_dirty(&status.stdout);
    let divergence = parse_divergence(&status.stdout);
    let now = Instant::now();
    let pr = if last_pr_fetch
        .map(|last| now.duration_since(last) >= PR_STATUS_RATE_LIMIT)
        .unwrap_or(true)
    {
        let pr = fetch_pr_status(cwd);
        *last_pr_fetch = Some(now);
        *last_pr_status = Some(pr.clone());
        pr
    } else {
        last_pr_status.clone().unwrap_or_default()
    };
    RepoStatus {
        head,
        dirty,
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

fn parse_dirty(output: &str) -> bool {
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !line.starts_with('#') {
            return true;
        }
    }
    false
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
        .args([
            "pr",
            "view",
            "--json",
            "number,title,state,mergedAt,isDraft",
        ])
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
mod tests;
