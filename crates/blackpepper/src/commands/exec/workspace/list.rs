use crate::git::resolve_repo_root;
use crate::workspaces::list_workspace_names;

use super::super::{CommandContext, CommandResult, CommandSource};

pub(crate) fn workspace_list(ctx: &CommandContext) -> CommandResult {
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
