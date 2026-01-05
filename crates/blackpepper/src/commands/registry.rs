//! Command registry and metadata.
//!
//! Defines all available commands with their names and descriptions.
//! Used for parsing validation, autocompletion, and help display.

/// Specification for a single command.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub name: &'static str,
    pub description: &'static str,
}

/// Top-level command names (first word after `:`).
pub const TOP_LEVEL_COMMANDS: &[&str] = &[
    "workspace",
    "tab",
    "pr",
    "export",
    "init",
    "debug",
    "help",
    "quit",
    "q",
];

/// Full command specifications with descriptions.
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
        description: "Destroy a workspace worktree and delete its branch (defaults to active)",
    },
    CommandSpec {
        name: "export",
        description: "Export current tab scrollback into a vi/vim buffer in a new tab",
    },
    CommandSpec {
        name: "init",
        description: "Initialize project config and gitignore",
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

/// Generate help lines for all commands.
pub fn command_help_lines() -> Vec<String> {
    let longest = COMMANDS
        .iter()
        .map(|command| command.name.len())
        .max()
        .unwrap_or(0);
    COMMANDS
        .iter()
        .map(|command| {
            format!(
                ":{:<width$} {}",
                command.name,
                command.description,
                width = longest
            )
        })
        .collect()
}

/// Generate hint lines matching the current input prefix.
///
/// Used for command-mode autocompletion display.
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
        .map(|command| {
            format!(
                ":{:<width$} {}",
                command.name,
                command.description,
                width = longest
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{command_help_lines, command_hint_lines, COMMANDS};

    #[test]
    fn help_lines_include_all_commands() {
        let lines = command_help_lines();
        assert_eq!(lines.len(), COMMANDS.len());
        assert!(lines.iter().all(|line| line.starts_with(':')));
        assert!(lines
            .iter()
            .any(|line| line.contains(":workspace list") && line.contains("List/switch")));
    }

    #[test]
    fn hint_lines_require_colon_prefix() {
        let lines = command_hint_lines("workspace", 10);
        assert!(lines.is_empty());
    }

    #[test]
    fn hint_lines_match_prefix_and_limit() {
        let lines = command_hint_lines(":tab", 3);
        assert!(!lines.is_empty());
        assert!(lines.len() <= 3);
        assert!(lines.iter().all(|line| line.starts_with(":tab")));
    }
}
