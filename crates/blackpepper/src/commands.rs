use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::animals::ANIMAL_NAMES;
use crate::git::{run_git, ExecResult, resolve_repo_root};
use crate::workspaces::{ensure_workspace_root, is_valid_workspace_name, list_workspace_names, workspace_path};

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub name: &'static str,
    pub description: &'static str,
}

pub const TOP_LEVEL_COMMANDS: &[&str] = &["workspace", "tab", "pr", "debug", "help", "quit", "q"];

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "workspace list",
        description: "List/switch workspaces",
    },
    CommandSpec {
        name: "workspace switch",
        description: "Switch the active workspace",
    },
    CommandSpec {
        name: "workspace create",
        description: "Create a workspace worktree (auto-pick name if omitted)",
    },
    CommandSpec {
        name: "workspace destroy",
        description: "Destroy a workspace worktree (defaults to active)",
    },
    CommandSpec {
        name: "tab new",
        description: "Open a new tab in the active workspace",
    },
    CommandSpec {
        name: "tab rename",
        description: "Rename the active tab",
    },
    CommandSpec {
        name: "tab close",
        description: "Close the active tab",
    },
    CommandSpec {
        name: "tab next",
        description: "Switch to the next tab",
    },
    CommandSpec {
        name: "tab prev",
        description: "Switch to the previous tab",
    },
    CommandSpec {
        name: "tab switch",
        description: "Switch tabs by index or name",
    },
    CommandSpec {
        name: "pr create",
        description: "Create a pull request",
    },
    CommandSpec {
        name: "pr open",
        description: "Open the current pull request",
    },
    CommandSpec {
        name: "pr merge",
        description: "Merge the current pull request",
    },
    CommandSpec {
        name: "debug mouse",
        description: "Toggle mouse debug overlay",
    },
    CommandSpec {
        name: "help",
        description: "Show available commands",
    },
    CommandSpec {
        name: "quit",
        description: "Exit Blackpepper",
    },
    CommandSpec {
        name: "q",
        description: "Alias for :quit",
    },
];

#[derive(Debug, Clone)]
pub struct CommandMatch {
    pub name: String,
    pub args: Vec<String>,
    #[allow(dead_code)]
    pub raw: String,
}

#[derive(Debug, Clone)]
pub struct CommandError {
    pub error: String,
    #[allow(dead_code)]
    pub raw: String,
}

pub type CommandParseResult = Result<CommandMatch, CommandError>;

#[derive(Debug, Clone)]
pub struct CommandResult {
    pub ok: bool,
    pub message: String,
    pub data: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CommandContext {
    pub cwd: PathBuf,
}

pub fn parse_command(input: &str) -> CommandParseResult {
    let raw = input.to_string();
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(CommandError {
            error: "Empty command".to_string(),
            raw,
        });
    }
    if !trimmed.starts_with(':') {
        return Err(CommandError {
            error: "Commands must start with ':'".to_string(),
            raw,
        });
    }
    let tokens: Vec<&str> = trimmed[1..].split_whitespace().collect();
    let Some((name, args)) = tokens.split_first() else {
        return Err(CommandError {
            error: "Missing command name".to_string(),
            raw,
        });
    };
    if !TOP_LEVEL_COMMANDS.iter().any(|command| *command == *name) {
        return Err(CommandError {
            error: format!("Unknown command: {name}"),
            raw,
        });
    }

    Ok(CommandMatch {
        name: name.to_string(),
        args: args.iter().map(|arg| arg.to_string()).collect(),
        raw,
    })
}

pub fn command_help_lines() -> Vec<String> {
    let longest = COMMANDS
        .iter()
        .map(|command| command.name.len())
        .max()
        .unwrap_or(0);
    COMMANDS
        .iter()
        .map(|command| format!(":{:<width$} {}", command.name, command.description, width = longest))
        .collect()
}

pub fn command_hint_lines(input: &str, max: usize) -> Vec<String> {
    let trimmed = input.trim();
    if !trimmed.starts_with(':') {
        return Vec::new();
    }

    let mut parts = trimmed[1..].split_whitespace();
    let first = parts.next().unwrap_or("");
    let second = parts.next();
    let query = if let Some(second) = second {
        format!("{first} {second}").to_lowercase()
    } else {
        first.to_lowercase()
    };
    let mut matches: Vec<&CommandSpec> = COMMANDS
        .iter()
        .filter(|command| query.is_empty() || command.name.starts_with(&query))
        .collect();

    if matches.is_empty() {
        return Vec::new();
    }

    matches.sort_by(|a, b| a.name.cmp(b.name));
    let longest = matches
        .iter()
        .map(|command| command.name.len())
        .max()
        .unwrap_or(0);

    matches
        .into_iter()
        .take(max)
        .map(|command| format!(":{:<width$} {}", command.name, command.description, width = longest))
        .collect()
}

pub fn complete_command_input(input: &str) -> Option<String> {
    let trimmed = input.trim_end();
    if !trimmed.starts_with(':') {
        return None;
    }
    let ends_with_space = trimmed.ends_with(' ');
    let without_colon = &trimmed[1..];
    let mut parts: Vec<&str> = without_colon.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    let current = if ends_with_space {
        ""
    } else {
        parts.pop().unwrap_or("")
    };
    let prefix_len = parts.len();

    let mut candidates: Vec<String> = Vec::new();
    if prefix_len == 0 {
        for cmd in TOP_LEVEL_COMMANDS {
            if current.is_empty() || cmd.starts_with(current) {
                candidates.push(cmd.to_string());
            }
        }
    } else {
        for spec in COMMANDS {
            let tokens: Vec<&str> = spec.name.split_whitespace().collect();
            if tokens.len() <= prefix_len {
                continue;
            }
            if tokens[..prefix_len] != parts[..] {
                continue;
            }
            let next = tokens[prefix_len];
            if current.is_empty() || next.starts_with(current) {
                candidates.push(next.to_string());
            }
        }
    }

    candidates.sort();
    candidates.dedup();
    if candidates.is_empty() {
        return None;
    }

    let common_prefix = longest_common_prefix(&candidates);
    if common_prefix.is_empty() || common_prefix == current {
        return None;
    }

    let mut new_parts: Vec<String> = parts.iter().map(|part| part.to_string()).collect();
    new_parts.push(common_prefix.clone());
    let mut new_input = format!(":{}", new_parts.join(" "));

    if candidates.len() == 1 && common_prefix == candidates[0] {
        new_input.push(' ');
    }
    Some(new_input)
}

fn longest_common_prefix(items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut prefix = items[0].clone();
    for item in &items[1..] {
        let mut next = String::new();
        for (a, b) in prefix.chars().zip(item.chars()) {
            if a == b {
                next.push(a);
            } else {
                break;
            }
        }
        prefix = next;
        if prefix.is_empty() {
            break;
        }
    }
    prefix
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
        ["show-ref", "--verify", "--quiet", &format!("refs/heads/{name}")].as_ref(),
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
    for name in unique_animal_names() {
        if !used.contains(&name) {
            return Some(name);
        }
    }
    None
}

pub fn run_command(name: &str, args: &[String], ctx: &CommandContext) -> CommandResult {
    match name {
        "help" => CommandResult {
            ok: true,
            message: command_help_lines().join("\n"),
            data: None,
        },
        "workspace" => {
            let Some(subcommand) = args.get(0) else {
                return CommandResult {
                    ok: false,
                    message: "Usage: :workspace <list|switch|create|destroy>".to_string(),
                    data: None,
                };
            };
            match subcommand.as_str() {
                "create" => workspace_create(&args[1..], ctx),
                "destroy" => workspace_destroy(&args[1..], ctx),
                "list" => CommandResult {
                    ok: true,
                    message: "Use :workspace list or Ctrl+P to switch.".to_string(),
                    data: None,
                },
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
            let Some(subcommand) = args.get(0) else {
                return CommandResult {
                    ok: false,
                    message: "Usage: :pr <create|open|merge>".to_string(),
                    data: None,
                };
            };
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

fn workspace_create(args: &[String], ctx: &CommandContext) -> CommandResult {
    let repo_root = match resolve_repo_root(&ctx.cwd) {
        Some(root) => root,
        None => {
            return CommandResult {
                ok: false,
                message: "Not inside a git repository.".to_string(),
                data: None,
            }
        }
    };

    if let Err(error) = ensure_workspace_root(&repo_root) {
        return CommandResult {
            ok: false,
            message: format!("Failed to create workspace root: {error}"),
            data: None,
        };
    }

    let used_names: HashSet<String> = list_workspace_names(&repo_root).into_iter().collect();
    let mut workspace_name = args.get(0).cloned();
    if workspace_name.is_none() {
        workspace_name = pick_unused_animal_name(&used_names);
    }
    let Some(workspace_name) = workspace_name else {
        return CommandResult {
            ok: false,
            message: "No unused animal names available. Use :workspace create <unique-name>.".to_string(),
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
            message: format!("Workspace name '{workspace_name}' is already in use. Choose another."),
            data: None,
        };
    }
    if branch_exists(&repo_root, &workspace_name) {
        return CommandResult {
            ok: false,
            message: format!("Branch '{workspace_name}' already exists. Choose another workspace name."),
            data: None,
        };
    }

    let worktree_path = workspace_path(&workspace_name);
    let worktree_path_str = worktree_path.to_string_lossy().to_string();
    let args = [
        "worktree",
        "add",
        worktree_path_str.as_str(),
        "-b",
        workspace_name.as_str(),
        "HEAD",
    ];
    let result = run_git(args.as_ref(), &repo_root);
    if !result.ok {
        let output = format_exec_output(&result);
        let details = if output.is_empty() { "".to_string() } else { format!("\n{output}") };
        return CommandResult {
            ok: false,
            message: format!("Failed to create workspace '{workspace_name}'.{details}"),
            data: None,
        };
    }

    let output = format_exec_output(&result);
    let details = if output.is_empty() { "".to_string() } else { format!("\n{output}") };
    CommandResult {
        ok: true,
        message: format!(
            "Created workspace '{workspace_name}' at {}.{details}",
            workspace_path(&workspace_name).to_string_lossy()
        ),
        data: Some(workspace_name),
    }
}

fn workspace_destroy(args: &[String], ctx: &CommandContext) -> CommandResult {
    let Some(name) = args.get(0) else {
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
    let repo_root = match resolve_repo_root(&ctx.cwd) {
        Some(root) => root,
        None => {
            return CommandResult {
                ok: false,
                message: "Not inside a git repository.".to_string(),
                data: None,
            }
        }
    };

    let worktree_path_str = workspace_path(name).to_string_lossy().to_string();
    let args = ["worktree", "remove", worktree_path_str.as_str()];
    let result = run_git(args.as_ref(), &repo_root);
    if !result.ok {
        let output = format_exec_output(&result);
        let details = if output.is_empty() { "".to_string() } else { format!("\n{output}") };
        return CommandResult {
            ok: false,
            message: format!("Failed to remove workspace '{name}'.{details}"),
            data: None,
        };
    }

    let output = format_exec_output(&result);
    let details = if output.is_empty() { "".to_string() } else { format!("\n{output}") };
    CommandResult {
        ok: true,
        message: format!(
            "Removed workspace '{name}' from {}.{details}",
            workspace_path(name).to_string_lossy()
        ),
        data: None,
    }
}
