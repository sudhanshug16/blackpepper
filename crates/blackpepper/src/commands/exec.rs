//! Command execution handlers.
//!
//! Contains the actual logic for each command. Commands receive
//! a context with current directory and repo information, and
//! return a result with success/failure and a message.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fs, io};

use crate::animals::ANIMAL_NAMES;
use crate::git::{resolve_repo_root, run_git, ExecResult};
use crate::updater::UpdateOutcome;
use crate::workspaces::{
    ensure_workspace_root, is_valid_workspace_name, list_workspace_names, workspace_path,
};

use super::registry::{command_help_lines, command_help_lines_cli};

/// Result of command execution.
#[derive(Debug, Clone)]
pub struct CommandResult {
    pub ok: bool,
    pub message: String,
    /// Optional data returned by command (e.g., created workspace name).
    pub data: Option<String>,
}

/// Context provided to command handlers.
#[derive(Debug, Clone)]
pub struct CommandContext {
    pub cwd: PathBuf,
    pub repo_root: Option<PathBuf>,
    pub workspace_root: PathBuf,
    pub source: CommandSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSource {
    Tui,
    Cli,
}

/// Dispatch and execute a command by name.
pub fn run_command(name: &str, args: &[String], ctx: &CommandContext) -> CommandResult {
    match name {
        "init" => init_project(args, ctx),
        "update" => CommandResult {
            ok: true,
            message: update_message(crate::updater::force_update()),
            data: None,
        },
        "version" => CommandResult {
            ok: true,
            message: format!("blackpepper v{}", env!("CARGO_PKG_VERSION")),
            data: None,
        },
        "help" => CommandResult {
            ok: true,
            message: if ctx.source == CommandSource::Cli {
                command_help_lines_cli().join("\n")
            } else {
                command_help_lines().join("\n")
            },
            data: None,
        },
        "workspace" => {
            let Some(subcommand) = args.first() else {
                return CommandResult {
                    ok: false,
                    message: "Usage: :workspace <list|switch|create|destroy>".to_string(),
                    data: None,
                };
            };
            match subcommand.as_str() {
                "create" => workspace_create(&args[1..], ctx),
                "destroy" => workspace_destroy(&args[1..], ctx),
                "list" => workspace_list(ctx),
                "switch" => CommandResult {
                    ok: true,
                    message: "Use :workspace switch <name> to change.".to_string(),
                    data: None,
                },
                _ => CommandResult {
                    ok: false,
                    message: "Usage: :workspace <list|switch|create|destroy>".to_string(),
                    data: None,
                },
            }
        }
        "pr" => {
            let Some(subcommand) = args.first() else {
                return CommandResult {
                    ok: false,
                    message: "Usage: :pr <create|open|merge>".to_string(),
                    data: None,
                };
            };
            // PR commands are stubs for now
            match subcommand.as_str() {
                "create" => CommandResult {
                    ok: true,
                    message: "PR creation is not implemented yet.".to_string(),
                    data: None,
                },
                "open" => CommandResult {
                    ok: true,
                    message: "PR opening is not implemented yet.".to_string(),
                    data: None,
                },
                "merge" => CommandResult {
                    ok: true,
                    message: "PR merge is not implemented yet.".to_string(),
                    data: None,
                },
                _ => CommandResult {
                    ok: false,
                    message: "Usage: :pr <create|open|merge>".to_string(),
                    data: None,
                },
            }
        }
        "quit" | "q" => CommandResult {
            ok: true,
            message: "Exiting.".to_string(),
            data: None,
        },
        _ => CommandResult {
            ok: false,
            message: format!("Unhandled command: {name}"),
            data: None,
        },
    }
}

/// Initialize project with config and gitignore.
fn init_project(args: &[String], ctx: &CommandContext) -> CommandResult {
    if !args.is_empty() {
        return CommandResult {
            ok: false,
            message: "Usage: :init".to_string(),
            data: None,
        };
    }
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

    let mut actions = Vec::new();
    let gitignore_path = repo_root.join(".gitignore");
    match ensure_gitignore_entries(&gitignore_path, &[".blackpepper/workspaces/"]) {
        Ok(true) => actions.push("updated .gitignore"),
        Ok(false) => actions.push(".gitignore already up to date"),
        Err(err) => {
            return CommandResult {
                ok: false,
                message: format!("Failed to update .gitignore: {err}"),
                data: None,
            }
        }
    }

    let config_path = repo_root.join(".blackpepper").join("config.toml");
    match ensure_project_config(&config_path) {
        Ok(true) => actions.push("created .blackpepper/config.toml"),
        Ok(false) => actions.push("project config already exists"),
        Err(err) => {
            return CommandResult {
                ok: false,
                message: format!("Failed to create project config: {err}"),
                data: None,
            }
        }
    }

    CommandResult {
        ok: true,
        message: format!("Initialized Blackpepper project: {}.", actions.join(", ")),
        data: None,
    }
}

fn workspace_list(ctx: &CommandContext) -> CommandResult {
    if ctx.source == CommandSource::Tui {
        return CommandResult {
            ok: true,
            message: "Use :workspace list or Ctrl+P to switch.".to_string(),
            data: None,
        };
    }

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

    let names = list_workspace_names(&repo_root, &ctx.workspace_root);
    if names.is_empty() {
        CommandResult {
            ok: true,
            message: "No workspaces yet.".to_string(),
            data: None,
        }
    } else {
        CommandResult {
            ok: true,
            message: names.join("\n"),
            data: None,
        }
    }
}

/// Create a new workspace worktree.
fn workspace_create(args: &[String], ctx: &CommandContext) -> CommandResult {
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

    if let Err(error) = ensure_workspace_root(&repo_root, &ctx.workspace_root) {
        return CommandResult {
            ok: false,
            message: format!("Failed to create workspace root: {error}"),
            data: None,
        };
    }

    let used_names: HashSet<String> = list_workspace_names(&repo_root, &ctx.workspace_root)
        .into_iter()
        .collect();
    let mut workspace_name = args.first().cloned();
    if workspace_name.is_none() {
        workspace_name = pick_unused_animal_name(&used_names);
    }
    let Some(workspace_name) = workspace_name else {
        return CommandResult {
            ok: false,
            message: "No unused animal names available. Use :workspace create <unique-name>."
                .to_string(),
            data: None,
        };
    };

    if !is_valid_workspace_name(&workspace_name) {
        return CommandResult {
            ok: false,
            message: "Workspace name must be lowercase letters or dashes.".to_string(),
            data: None,
        };
    }
    if used_names.contains(&workspace_name) {
        return CommandResult {
            ok: false,
            message: format!(
                "Workspace name '{workspace_name}' is already in use. Choose another."
            ),
            data: None,
        };
    }
    if branch_exists(&repo_root, &workspace_name) {
        return CommandResult {
            ok: false,
            message: format!(
                "Branch '{workspace_name}' already exists. Choose another workspace name."
            ),
            data: None,
        };
    }

    let worktree_path = workspace_path(&ctx.workspace_root, &workspace_name);
    let worktree_path_str = worktree_path.to_string_lossy().to_string();
    let git_args = [
        "worktree",
        "add",
        worktree_path_str.as_str(),
        "-b",
        workspace_name.as_str(),
        "HEAD",
    ];
    let result = run_git(git_args.as_ref(), &repo_root);
    if !result.ok {
        let output = format_exec_output(&result);
        let details = if output.is_empty() {
            "".to_string()
        } else {
            format!("\n{output}")
        };
        return CommandResult {
            ok: false,
            message: format!("Failed to create workspace '{workspace_name}'.{details}"),
            data: None,
        };
    }

    let output = format_exec_output(&result);
    let details = if output.is_empty() {
        "".to_string()
    } else {
        format!("\n{output}")
    };
    CommandResult {
        ok: true,
        message: format!(
            "Created workspace '{workspace_name}' at {}.{details}",
            worktree_path.to_string_lossy()
        ),
        data: Some(workspace_name),
    }
}

/// Destroy a workspace worktree and its branch.
fn workspace_destroy(args: &[String], ctx: &CommandContext) -> CommandResult {
    let Some(name) = args.first() else {
        return CommandResult {
            ok: false,
            message: "Usage: :workspace destroy <animal>".to_string(),
            data: None,
        };
    };
    if !is_valid_workspace_name(name) {
        return CommandResult {
            ok: false,
            message: "Workspace name must be lowercase letters or dashes.".to_string(),
            data: None,
        };
    }
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

    let worktree_path_str = workspace_path(&ctx.workspace_root, name)
        .to_string_lossy()
        .to_string();
    let git_args = ["worktree", "remove", worktree_path_str.as_str()];
    let result = run_git(git_args.as_ref(), &repo_root);
    if !result.ok {
        let output = format_exec_output(&result);
        let details = if output.is_empty() {
            "".to_string()
        } else {
            format!("\n{output}")
        };
        return CommandResult {
            ok: false,
            message: format!("Failed to remove workspace '{name}'.{details}"),
            data: None,
        };
    }

    let mut deleted_branch = false;
    if branch_exists(&repo_root, name) {
        let git_args = ["branch", "-D", name.as_str()];
        let branch_result = run_git(git_args.as_ref(), &repo_root);
        if !branch_result.ok {
            let output = format_exec_output(&branch_result);
            let details = if output.is_empty() {
                "".to_string()
            } else {
                format!("\n{output}")
            };
            return CommandResult {
                ok: false,
                message: format!(
                    "Removed workspace '{name}', but failed to delete branch '{name}'.{details}"
                ),
                data: None,
            };
        }
        deleted_branch = true;
    }

    let output = format_exec_output(&result);
    let details = if output.is_empty() {
        "".to_string()
    } else {
        format!("\n{output}")
    };
    CommandResult {
        ok: true,
        message: if deleted_branch {
            format!(
                "Removed workspace '{name}' from {} and deleted its branch.{details}",
                workspace_path(&ctx.workspace_root, name).to_string_lossy()
            )
        } else {
            format!(
                "Removed workspace '{name}' from {}.{details}",
                workspace_path(&ctx.workspace_root, name).to_string_lossy()
            )
        },
        data: None,
    }
}

// --- Helper functions ---

fn format_exec_output(result: &ExecResult) -> String {
    let stdout = result.stdout.trim();
    let stderr = result.stderr.trim();
    [stdout, stderr]
        .iter()
        .filter(|text| !text.is_empty())
        .map(|text| text.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn branch_exists(repo_root: &Path, name: &str) -> bool {
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

fn unique_animal_names() -> Vec<String> {
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

fn pick_unused_animal_name(used: &HashSet<String>) -> Option<String> {
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

fn ensure_gitignore_entries(path: &Path, entries: &[&str]) -> io::Result<bool> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    let mut known: HashSet<String> = existing
        .lines()
        .map(|line| line.trim().to_string())
        .collect();
    let mut output = existing;
    let mut changed = false;

    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
        changed = true;
    }

    for entry in entries {
        if !known.contains(*entry) {
            output.push_str(entry);
            output.push('\n');
            known.insert((*entry).to_string());
            changed = true;
        }
    }

    if changed {
        fs::write(path, output)?;
    }
    Ok(changed)
}

fn ensure_project_config(path: &Path) -> io::Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, "")?;
    Ok(true)
}

fn update_message(outcome: UpdateOutcome) -> String {
    match outcome {
        UpdateOutcome::Started => {
            "Update started. Restart Blackpepper to use the new version.".to_string()
        }
        UpdateOutcome::SkippedDev => {
            "Update skipped for dev builds. Use the installer for releases.".to_string()
        }
        UpdateOutcome::SkippedDisabled => {
            "Update disabled via BLACKPEPPER_DISABLE_UPDATE.".to_string()
        }
        UpdateOutcome::SkippedCooldown => {
            "Update skipped due to cooldown. Try again later.".to_string()
        }
        UpdateOutcome::FailedSpawn => "Failed to start updater.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_gitignore_entries, ensure_project_config, pick_unused_animal_name,
        unique_animal_names, workspace_create, workspace_destroy, CommandContext, CommandSource,
    };
    use crate::git::run_git;
    use std::collections::HashSet;
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    fn run_git_cmd(args: &[&str], cwd: &Path) {
        let status = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "Test User")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test User")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .expect("run git");
        assert!(status.success(), "git {:?} failed", args);
    }

    fn init_repo() -> TempDir {
        let repo = TempDir::new().expect("temp repo");
        run_git_cmd(&["init"], repo.path());
        fs::write(repo.path().join("README.md"), "hello").expect("write file");
        run_git_cmd(&["add", "."], repo.path());
        run_git_cmd(&["commit", "-m", "init"], repo.path());
        repo
    }

    #[test]
    fn gitignore_entries_are_appended_once() {
        let dir = TempDir::new().expect("temp dir");
        let gitignore = dir.path().join(".gitignore");
        fs::write(&gitignore, "target/\n").expect("write gitignore");

        let changed = ensure_gitignore_entries(&gitignore, &[".blackpepper/workspaces/"])
            .expect("update gitignore");
        assert!(changed);

        let contents = fs::read_to_string(&gitignore).expect("read gitignore");
        assert!(contents.contains("target/"));
        assert!(contents.contains(".blackpepper/workspaces/"));

        let changed_again = ensure_gitignore_entries(&gitignore, &[".blackpepper/workspaces/"])
            .expect("update gitignore");
        assert!(!changed_again);
    }

    #[test]
    fn project_config_is_created_once() {
        let dir = TempDir::new().expect("temp dir");
        let config_path = dir.path().join(".blackpepper").join("config.toml");

        let created = ensure_project_config(&config_path).expect("create config");
        assert!(created);
        assert!(config_path.exists());

        let created_again = ensure_project_config(&config_path).expect("create config");
        assert!(!created_again);
    }

    #[test]
    fn unique_animal_names_are_valid_and_unique() {
        let names = unique_animal_names();
        let set: HashSet<_> = names.iter().collect();
        assert_eq!(set.len(), names.len());
        assert!(!names.is_empty());
    }

    #[test]
    fn pick_unused_returns_none_when_exhausted() {
        let names = unique_animal_names();
        let used: HashSet<String> = names.into_iter().collect();
        let picked = pick_unused_animal_name(&used);
        assert!(picked.is_none());
    }

    #[test]
    fn workspace_create_and_destroy_workflow() {
        let repo = init_repo();
        let workspace_root = Path::new(".blackpepper/workspaces");
        let ctx = CommandContext {
            cwd: repo.path().to_path_buf(),
            repo_root: Some(repo.path().to_path_buf()),
            workspace_root: workspace_root.to_path_buf(),
            source: CommandSource::Cli,
        };

        let name = "otter";
        let create = workspace_create(&[name.to_string()], &ctx);
        assert!(create.ok, "create failed: {}", create.message);
        assert_eq!(create.data.as_deref(), Some(name));

        let workspace_path = repo.path().join(workspace_root).join(name);
        assert!(workspace_path.exists());

        let destroy = workspace_destroy(&[name.to_string()], &ctx);
        assert!(destroy.ok, "destroy failed: {}", destroy.message);
        assert!(!workspace_path.exists());

        let result = run_git(
            [
                "show-ref",
                "--verify",
                "--quiet",
                "refs/heads/otter",
            ]
            .as_ref(),
            repo.path(),
        );
        assert!(!result.ok);
    }
}
