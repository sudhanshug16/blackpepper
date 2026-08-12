use serde::{Deserialize, Serialize};
use std::path::PathBuf;

mod approval;
pub(crate) use approval::WorktrunkApprovalPlan;
pub use approval::{WorktrunkApprovalToken, WorktrunkProjectCommand};

pub const LIST_SCHEMA_VERSION: u64 = 2;
pub const APPROVAL_TOKEN_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeList {
    pub schema: u64,
    pub repo: Repository,
    #[serde(default)]
    pub collected: Collected,
    pub items: Vec<WorktreeItem>,
}

impl WorktreeList {
    pub fn parse(json: &str) -> Result<Self, String> {
        let list: Self = serde_json::from_str(json)
            .map_err(|err| format!("Worktrunk returned invalid JSON: {err}"))?;
        if list.schema != LIST_SCHEMA_VERSION {
            return Err(format!(
                "Unsupported Worktrunk list schema {} (expected {}).",
                list.schema, LIST_SCHEMA_VERSION
            ));
        }
        Ok(list)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repository {
    pub default_branch: String,
    #[serde(default)]
    pub forge: Option<Forge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forge {
    pub url: String,
    pub provider: String,
    pub host: String,
    pub owner: String,
    pub name: String,
    #[serde(default)]
    pub remote: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collected {
    #[serde(default)]
    pub ci: bool,
    #[serde(default)]
    pub summary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeItem {
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(default)]
    pub head: Option<Head>,
    #[serde(default)]
    pub worktree: Option<Worktree>,
    #[serde(default)]
    pub display: Option<Display>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Head {
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
    pub committed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worktree {
    pub path: PathBuf,
    #[serde(default)]
    pub main: bool,
    #[serde(default)]
    pub current: bool,
    #[serde(default)]
    pub previous: bool,
    #[serde(default)]
    pub detached: bool,
    #[serde(default)]
    pub branch_mismatch: bool,
    #[serde(default)]
    pub duplicate_branch: bool,
    #[serde(default)]
    pub changes: Option<Changes>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changes {
    #[serde(default)]
    pub staged: bool,
    #[serde(default)]
    pub modified: bool,
    #[serde(default)]
    pub untracked: bool,
    #[serde(default)]
    pub renamed: bool,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub conflicted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Display {
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub symbols: Option<String>,
    #[serde(default)]
    pub statusline: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwitchResult {
    pub branch: Option<String>,
    pub path: PathBuf,
}

impl SwitchResult {
    pub fn parse(json: &str) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|err| format!("Worktrunk returned invalid switch JSON: {err}"))?;
        let path = value
            .get("path")
            .or_else(|| value.get("worktree_path"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "Worktrunk switch JSON did not include a worktree path.".to_string())?;
        let branch = value
            .get("branch")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        Ok(Self {
            branch,
            path: PathBuf::from(path),
        })
    }
}
