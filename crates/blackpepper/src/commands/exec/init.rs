use std::path::Path;
use std::{fs, io};

use crate::git::resolve_repo_root;

use super::{CommandContext, CommandResult};

/// Initialize project with config and gitignore.
pub(super) fn init_project(args: &[String], ctx: &CommandContext) -> CommandResult {
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
    match ensure_gitignore_entries(
        &gitignore_path,
        &[
            ".blackpepper/workspaces/",
            ".config/blackpepper/config.local.toml",
        ],
    ) {
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

    let config_path = repo_root
        .join(".config")
        .join("blackpepper")
        .join("config.toml");
    match ensure_project_config(&config_path) {
        Ok(true) => actions.push("created .config/blackpepper/config.toml"),
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

pub(super) fn ensure_gitignore_entries(path: &Path, entries: &[&str]) -> io::Result<bool> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    let mut known: std::collections::HashSet<String> = existing
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

pub(super) fn ensure_project_config(path: &Path) -> io::Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, "")?;
    Ok(true)
}
