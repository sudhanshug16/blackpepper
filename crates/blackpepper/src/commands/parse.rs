//! Command parsing and completion.
//!
//! Handles tokenizing command input, validating command names,
//! and providing tab-completion suggestions.

use super::registry::{COMMANDS, TOP_LEVEL_COMMANDS};

/// Successfully parsed command.
#[derive(Debug, Clone)]
pub struct CommandMatch {
    pub name: String,
    pub args: Vec<String>,
    #[allow(dead_code)]
    pub raw: String,
}

/// Parse error with context.
#[derive(Debug, Clone)]
pub struct CommandError {
    pub error: String,
    #[allow(dead_code)]
    pub raw: String,
}

pub type CommandParseResult = Result<CommandMatch, CommandError>;

/// Parse a command string into name and arguments.
///
/// Commands must start with `:` and have a valid top-level name.
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
    if !TOP_LEVEL_COMMANDS.contains(name) {
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

/// Attempt to complete the current command input.
///
/// Returns the completed string if a unique completion exists,
/// or the longest common prefix if multiple matches exist.
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
        // Complete top-level commands
        for cmd in TOP_LEVEL_COMMANDS {
            if current.is_empty() || cmd.starts_with(current) {
                candidates.push(cmd.to_string());
            }
        }
    } else {
        // Complete subcommands based on registry
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

    // Add trailing space if unique completion
    if candidates.len() == 1 && common_prefix == candidates[0] {
        new_input.push(' ');
    }
    Some(new_input)
}

/// Find the longest common prefix among strings.
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

#[cfg(test)]
mod tests {
    use super::{complete_command_input, parse_command};

    #[test]
    fn parse_rejects_empty_input() {
        let err = parse_command("").unwrap_err();
        assert_eq!(err.error, "Empty command");
    }

    #[test]
    fn parse_rejects_missing_colon() {
        let err = parse_command("help").unwrap_err();
        assert_eq!(err.error, "Commands must start with ':'");
    }

    #[test]
    fn parse_rejects_missing_name() {
        let err = parse_command(":").unwrap_err();
        assert_eq!(err.error, "Missing command name");
    }

    #[test]
    fn parse_rejects_unknown_command() {
        let err = parse_command(":wut").unwrap_err();
        assert_eq!(err.error, "Unknown command: wut");
    }

    #[test]
    fn parse_accepts_command_with_args() {
        let cmd = parse_command(":workspace create otter").expect("parse ok");
        assert_eq!(cmd.name, "workspace");
        assert_eq!(cmd.args, vec!["create".to_string(), "otter".to_string()]);
    }

    #[test]
    fn parse_trims_whitespace() {
        let cmd = parse_command("  :help   ").expect("parse ok");
        assert_eq!(cmd.name, "help");
        assert!(cmd.args.is_empty());
    }

    #[test]
    fn complete_top_level_unique() {
        let completed = complete_command_input(":wo").expect("completion");
        assert_eq!(completed, ":workspace ");
    }

    #[test]
    fn complete_subcommand_common_prefix() {
        let completed = complete_command_input(":tab n").expect("completion");
        assert_eq!(completed, ":tab ne");
    }

    #[test]
    fn complete_returns_none_without_colon() {
        assert!(complete_command_input("tab n").is_none());
    }

    #[test]
    fn complete_returns_none_when_no_progress() {
        assert!(complete_command_input(":tab ").is_none());
    }
}
