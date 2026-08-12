//! Safe, non-shelling Worktrunk command construction and JSON parsing.

mod model;

pub(crate) use model::WorktrunkApprovalPlan;
pub use model::{
    SwitchResult, WorktreeItem, WorktreeList, WorktrunkApprovalToken, WorktrunkProjectCommand,
    APPROVAL_TOKEN_SCHEMA_VERSION, LIST_SCHEMA_VERSION,
};

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

pub const REQUIRED_VERSION: &str = "0.72.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Read,
    Mutation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub kind: OperationKind,
    /// Mutations are deliberately not passed `--yes`; callers must provide a
    /// visible interactive approval path if Worktrunk requests one.
    pub interactive: bool,
}

impl CommandSpec {
    pub fn display(&self) -> String {
        std::iter::once(self.program.as_os_str())
            .chain(self.args.iter().map(OsString::as_os_str))
            .map(display_arg)
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn contains_forbidden_force_flag(&self) -> bool {
        self.args.iter().any(|arg| {
            matches!(
                arg.to_str(),
                Some("--force" | "-f" | "--force-delete" | "-D" | "--clobber" | "--reap")
            )
        })
    }
}

#[derive(Debug, Clone)]
pub struct Worktrunk {
    command: PathBuf,
}

impl Worktrunk {
    pub fn new(command: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
        }
    }

    pub fn version(&self) -> CommandSpec {
        self.read([OsString::from("--version")])
    }

    pub fn list(&self, repo: &Path) -> CommandSpec {
        self.read([
            OsString::from("-C"),
            repo.as_os_str().to_owned(),
            OsString::from("--config-set"),
            OsString::from("list.json-schema=2"),
            OsString::from("list"),
            OsString::from("--branches"),
            OsString::from("--remotes"),
            OsString::from("--format=json"),
        ])
    }

    pub fn create(
        &self,
        repo: &Path,
        branch: &str,
        base: Option<&str>,
    ) -> Result<CommandSpec, String> {
        validate_selector(branch)?;
        if let Some(base) = base {
            validate_selector(base)?;
        }
        let mut args = vec![
            OsString::from("-C"),
            repo.as_os_str().to_owned(),
            OsString::from("switch"),
            OsString::from("--create"),
            OsString::from(branch),
        ];
        if let Some(base) = base {
            args.extend([OsString::from("--base"), OsString::from(base)]);
        }
        args.extend([OsString::from("--no-cd"), OsString::from("--format=json")]);
        Ok(self.mutation(args))
    }

    pub fn switch(&self, repo: &Path, selector: &str) -> Result<CommandSpec, String> {
        validate_selector(selector)?;
        Ok(self.mutation([
            OsString::from("-C"),
            repo.as_os_str().to_owned(),
            OsString::from("switch"),
            OsString::from(selector),
            OsString::from("--no-cd"),
            OsString::from("--format=json"),
        ]))
    }

    pub fn remove(&self, surviving_worktree: &Path, target: &Path) -> Result<CommandSpec, String> {
        if !target.is_absolute() {
            return Err("Worktrunk removal requires an absolute target path.".to_string());
        }
        Ok(self.mutation([
            OsString::from("-C"),
            surviving_worktree.as_os_str().to_owned(),
            OsString::from("remove"),
            target.as_os_str().to_owned(),
            OsString::from("--foreground"),
            OsString::from("--format=json"),
        ]))
    }

    pub fn approval_list(&self, repo: &Path) -> CommandSpec {
        self.read([
            OsString::from("-C"),
            repo.as_os_str().to_owned(),
            OsString::from("config"),
            OsString::from("approvals"),
            OsString::from("list"),
            OsString::from("--format=json"),
        ])
    }

    pub fn approval_add(&self, repo: &Path) -> CommandSpec {
        self.mutation([
            OsString::from("-C"),
            repo.as_os_str().to_owned(),
            OsString::from("config"),
            OsString::from("approvals"),
            OsString::from("add"),
        ])
    }

    fn read(&self, args: impl IntoIterator<Item = OsString>) -> CommandSpec {
        CommandSpec {
            program: self.command.clone(),
            args: args.into_iter().collect(),
            kind: OperationKind::Read,
            interactive: false,
        }
    }

    fn mutation(&self, args: impl IntoIterator<Item = OsString>) -> CommandSpec {
        let spec = CommandSpec {
            program: self.command.clone(),
            args: args.into_iter().collect(),
            kind: OperationKind::Mutation,
            interactive: true,
        };
        debug_assert!(!spec.contains_forbidden_force_flag());
        spec
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationResult<T> {
    Success(T),
    /// The directory exists even though a pre-start hook failed.
    SetupFailed {
        path: PathBuf,
        message: String,
    },
    /// Transport loss leaves the repository result indeterminate. Never retry.
    UnknownAfterDisconnect,
}

fn validate_selector(selector: &str) -> Result<(), String> {
    if selector.trim().is_empty() {
        return Err("Worktrunk branch or PR selector cannot be empty.".to_string());
    }
    if selector.contains('\0') {
        return Err("Worktrunk selector contains a NUL byte.".to_string());
    }
    if selector.starts_with('-') {
        return Err("Worktrunk selectors cannot begin with an option marker.".to_string());
    }
    Ok(())
}

fn display_arg(value: &OsStr) -> String {
    let value = value.to_string_lossy();
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "-._/:=@+".contains(ch))
    {
        value.into_owned()
    } else {
        format!("{:?}", value)
    }
}

#[cfg(test)]
mod tests;
