use std::collections::HashSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::animals::ANIMAL_NAMES;
use crate::git::{resolve_repo_root, run_git, ExecResult};
use crate::workspaces::{
    ensure_workspace_root, is_valid_workspace_name, list_workspace_names, workspace_path,
};

use super::{CommandContext, CommandResult, CommandSource};

pub(super) fn workspace_list(ctx: &CommandContext) -> CommandResult {
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
pub(super) fn workspace_create(args: &[String], ctx: &CommandContext) -> CommandResult {
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
pub(super) fn workspace_destroy(args: &[String], ctx: &CommandContext) -> CommandResult {
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

pub(super) fn unique_animal_names() -> Vec<String> {
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

pub(super) fn pick_unused_animal_name(used: &HashSet<String>) -> Option<String> {
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
