//! Command registry and metadata.
//!
//! Defines all available commands with their names and descriptions.
//! Used for parsing validation, autocompletion, and help display.

/// Specification for a single command.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub cli_exposed: bool,
}

/// Top-level command names (first word after `:`).
pub const TOP_LEVEL_COMMANDS: &[&str] = &[
    "workspace",
    "tab",
    "pr",
    "export",
    "init",
    "update",
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
        cli_exposed: true,
    },
    CommandSpec {
        name: "workspace switch",
        description: "Switch the active workspace",
        cli_exposed: true,
    },
    CommandSpec {
        name: "workspace create",
        description: "Create a workspace worktree (auto-pick name if omitted)",
        cli_exposed: true,
    },
    CommandSpec {
        name: "workspace destroy",
        description: "Destroy a workspace worktree and delete its branch (defaults to active)",
        cli_exposed: true,
    },
    CommandSpec {
        name: "export",
        description: "Export current tab scrollback into a vi/vim buffer in a new tab",
        cli_exposed: false,
    },
    CommandSpec {
        name: "init",
        description: "Initialize project config and gitignore",
        cli_exposed: true,
    },
    CommandSpec {
        name: "update",
        description: "Update to the latest release (applies on next restart)",
        cli_exposed: true,
    },
    CommandSpec {
        name: "tab new",
        description: "Open a new tab in the active workspace",
        cli_exposed: false,
    },
    CommandSpec {
        name: "tab rename",
        description: "Rename the active tab",
        cli_exposed: false,
    },
    CommandSpec {
        name: "tab close",
        description: "Close the active tab",
        cli_exposed: false,
    },
    CommandSpec {
        name: "tab next",
        description: "Switch to the next tab",
        cli_exposed: false,
    },
    CommandSpec {
        name: "tab prev",
        description: "Switch to the previous tab",
        cli_exposed: false,
    },
    CommandSpec {
        name: "tab switch",
        description: "Switch tabs by index or name",
        cli_exposed: false,
    },
    CommandSpec {
        name: "pr create",
        description: "Create a pull request",
        cli_exposed: false,
    },
    CommandSpec {
        name: "pr open",
        description: "Open the current pull request",
        cli_exposed: false,
    },
    CommandSpec {
        name: "pr merge",
        description: "Merge the current pull request",
        cli_exposed: false,
    },
    CommandSpec {
        name: "debug mouse",
        description: "Toggle mouse debug overlay",
        cli_exposed: false,
    },
    CommandSpec {
        name: "help",
        description: "Show available commands",
        cli_exposed: true,
    },
    CommandSpec {
        name: "version",
        description: "Show version information",
        cli_exposed: true,
    },
    CommandSpec {
        name: "quit",
        description: "Exit Blackpepper",
        cli_exposed: false,
    },
    CommandSpec {
        name: "q",
        description: "Alias for :quit",
        cli_exposed: false,
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

/// Generate help lines for CLI-exposed commands.
pub fn command_help_lines_cli() -> Vec<String> {
    let mut matches: Vec<&CommandSpec> = COMMANDS.iter().filter(|cmd| cmd.cli_exposed).collect();
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
        .map(|command| {
            format!(
                "{:<width$} {}",
                command.name,
                command.description,
                width = longest
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{command_help_lines, command_help_lines_cli, command_hint_lines, COMMANDS};

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

    #[test]
    fn help_lines_cli_only_include_cli_exposed() {
        let lines = command_help_lines_cli();
        assert!(!lines.is_empty());
        assert!(lines.iter().any(|line| line.starts_with("workspace list")));
        assert!(lines.iter().any(|line| line.starts_with("init")));
        assert!(lines.iter().any(|line| line.starts_with("update")));
        assert!(lines.iter().any(|line| line.starts_with("version")));
        assert!(!lines.iter().any(|line| line.starts_with("tab new")));
    }
}
