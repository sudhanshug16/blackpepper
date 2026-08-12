use crate::agent_status::Provider;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientCommand {
    HostAdd {
        name: String,
        destination: String,
    },
    HostImport,
    HostConnect {
        name: String,
    },
    HostDisconnect {
        name: String,
    },
    WorkspaceRegister {
        path: PathBuf,
    },
    WorkspaceSwitch {
        selector: String,
    },
    WorkspaceUngroup,
    WorkspaceTerminate,
    WorktreeList,
    WorktreeCreate {
        branch: String,
        base: Option<String>,
    },
    WorktreeOpen {
        selector: String,
    },
    WorktreeRemove,
    AgentSpawn {
        provider: Provider,
    },
    ServiceStart {
        name: String,
    },
    Ports {
        all_host: bool,
    },
    Forward {
        remote_port: u16,
        bind_address: Option<String>,
    },
    ForwardCancel {
        remote_port: u16,
        bind_address: Option<String>,
    },
    /// `None` lists the palettes; `Some` switches to one.
    Theme {
        name: Option<String>,
    },
    StatusExplain,
    Approve,
    Refresh,
    Help,
    Quit,
}

pub fn parse(input: &str) -> Result<ClientCommand, String> {
    let input = input
        .trim()
        .strip_prefix(':')
        .ok_or_else(|| "Blackpepper commands begin with ':'.".to_string())?;
    let words =
        shell_words::split(input).map_err(|err| format!("Invalid command quoting: {err}"))?;
    let values = words.iter().map(String::as_str).collect::<Vec<_>>();
    match values.as_slice() {
        ["host", "add", name, destination] => Ok(ClientCommand::HostAdd {
            name: validate_name(name)?,
            destination: validate_destination(destination)?,
        }),
        ["host", "import"] => Ok(ClientCommand::HostImport),
        ["host", "connect", name] => Ok(ClientCommand::HostConnect {
            name: validate_name(name)?,
        }),
        ["host", "disconnect", name] => Ok(ClientCommand::HostDisconnect {
            name: validate_name(name)?,
        }),
        ["workspace", "add", path] => Ok(ClientCommand::WorkspaceRegister {
            path: PathBuf::from(path),
        }),
        ["workspace", "switch", selector] => Ok(ClientCommand::WorkspaceSwitch {
            selector: selector.to_string(),
        }),
        ["workspace", "ungroup"] => Ok(ClientCommand::WorkspaceUngroup),
        ["workspace", "terminate"] => Ok(ClientCommand::WorkspaceTerminate),
        ["worktree", "list"] => Ok(ClientCommand::WorktreeList),
        ["worktree", "create", branch] => Ok(ClientCommand::WorktreeCreate {
            branch: selector(branch)?,
            base: None,
        }),
        ["worktree", "create", branch, "--base", base] => Ok(ClientCommand::WorktreeCreate {
            branch: selector(branch)?,
            base: Some(selector(base)?),
        }),
        ["worktree", "open", target] => Ok(ClientCommand::WorktreeOpen {
            selector: selector(target)?,
        }),
        ["worktree", "remove"] => Ok(ClientCommand::WorktreeRemove),
        ["agent", "spawn", provider] => Ok(ClientCommand::AgentSpawn {
            provider: provider
                .parse::<Provider>()
                .map_err(|err| err.to_string())?,
        }),
        ["service", "start", name] => Ok(ClientCommand::ServiceStart {
            name: validate_service_name(name)?,
        }),
        ["ports"] => Ok(ClientCommand::Ports { all_host: false }),
        ["ports", "--all-host"] => Ok(ClientCommand::Ports { all_host: true }),
        ["forward", selector] => {
            let (bind_address, remote_port) = parse_forward_selector(selector)?;
            Ok(ClientCommand::Forward {
                remote_port,
                bind_address,
            })
        }
        ["forward", "cancel", selector] => {
            let (bind_address, remote_port) = parse_forward_selector(selector)?;
            Ok(ClientCommand::ForwardCancel {
                remote_port,
                bind_address,
            })
        }
        ["theme"] => Ok(ClientCommand::Theme { name: None }),
        ["theme", name] => Ok(ClientCommand::Theme {
            name: Some(name.to_string()),
        }),
        ["status", "explain"] => Ok(ClientCommand::StatusExplain),
        ["approve"] => Ok(ClientCommand::Approve),
        ["refresh"] => Ok(ClientCommand::Refresh),
        ["help"] => Ok(ClientCommand::Help),
        ["quit" | "q"] => Ok(ClientCommand::Quit),
        [] => Err("Choose a command; type to filter the list below.".to_string()),
        _ => Err(usage_error(input, &values)),
    }
}

fn usage_error(input: &str, values: &[&str]) -> String {
    let usage = match values.first().copied() {
        Some("host") => {
            ":host add <name> <ssh-alias> | import | connect <name> | disconnect <name>"
        }
        Some("workspace") => ":workspace add <path> | switch <name|id> | ungroup | terminate",
        Some("worktree") => {
            ":worktree list | create <branch> [--base <ref>] | open <branch|pr:123|url> | remove"
        }
        Some("agent") => ":agent spawn <codex|claude|opencode>",
        Some("service") => ":service start <name>",
        Some("ports") => ":ports [--all-host]",
        Some("forward") => ":forward [cancel] <port|address:port>",
        Some("status") => ":status explain",
        _ => return format!("Unknown command: :{input}. Type :help for the full list."),
    };
    format!("Usage: {usage}")
}

pub const HELP: &[(&str, &str)] = &[
    (":host add <name> <ssh-alias>", "Add an SSH host"),
    (":host import", "Preview literal aliases from ~/.ssh/config"),
    (":host connect <name>", "Connect to a host"),
    (
        ":host disconnect <name>",
        "Disconnect without stopping sessions",
    ),
    (":workspace add <path>", "Register a folder as a workspace"),
    (":workspace switch <name|id>", "Attach to a workspace"),
    (
        ":workspace ungroup",
        "Keep this workspace outside repository grouping",
    ),
    (
        ":workspace terminate",
        "Terminate its Zellij session, keep its folder",
    ),
    (":worktree list", "List Worktrunk branches/worktrees"),
    (
        ":worktree create <branch> [--base <ref>]",
        "Create and register a worktree",
    ),
    (
        ":worktree open <branch|pr:123|url>",
        "Open an existing branch or PR worktree",
    ),
    (
        ":worktree remove",
        "Remove through Worktrunk without force flags",
    ),
    (
        ":agent spawn <codex|claude|opencode>",
        "Start an integrated agent tab",
    ),
    (":service start <name>", "Start a configured service tab"),
    (":ports [--all-host]", "Discover listening ports"),
    (
        ":forward <port|address:port>",
        "Forward one discovered listener to client loopback",
    ),
    (
        ":forward cancel <port|address:port>",
        "Cancel this client's exact forward",
    ),
    (":theme [<name>]", "List palettes, or switch to one"),
    (":status explain", "Show redacted status evidence"),
    (":approve", "Approve the displayed Worktrunk command"),
    (":refresh", "Refresh hosts, workspaces, agents, and ports"),
    (":help", "Show this command reference"),
    (":quit", "Detach and exit"),
];

fn parse_port(value: &str) -> Result<u16, String> {
    value
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| format!("Invalid TCP port: {value}"))
}

fn parse_forward_selector(value: &str) -> Result<(Option<String>, u16), String> {
    if let Ok(port) = parse_port(value) {
        return Ok((None, port));
    }
    let (address, port) = if let Some(value) = value.strip_prefix('[') {
        let (address, port) = value
            .split_once("]:")
            .ok_or_else(|| "IPv6 forward targets use [address]:port.".to_string())?;
        (address, port)
    } else {
        let (address, port) = value
            .rsplit_once(':')
            .ok_or_else(|| "Use :forward <port> or :forward <address>:<port>.".to_string())?;
        if address.contains(':') {
            return Err("IPv6 forward targets use [address]:port.".to_string());
        }
        (address, port)
    };
    let port = parse_port(port)?;
    let target = crate::ports::RemotePortTarget::from_bind_address(address, port)?;
    Ok((Some(target.remote_host), port))
}

fn validate_name(value: &str) -> Result<String, String> {
    if value.is_empty()
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(format!("Invalid host name: {value}"));
    }
    Ok(value.to_string())
}

fn validate_service_name(value: &str) -> Result<String, String> {
    if value.trim().is_empty() || value.len() > 48 || value.chars().any(char::is_control) {
        return Err("Service names must contain 1-48 printable characters.".to_string());
    }
    Ok(value.to_string())
}

fn validate_destination(value: &str) -> Result<String, String> {
    if value.is_empty() || value.starts_with('-') || value.chars().any(char::is_whitespace) {
        return Err("SSH destination must be a host alias and cannot start with '-'.".to_string());
    }
    Ok(value.to_string())
}

fn selector(value: &str) -> Result<String, String> {
    if value.is_empty() || value.contains('\0') {
        return Err("Worktree selector cannot be empty or contain NUL.".to_string());
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_remote_and_local_workspace_commands() {
        assert_eq!(
            parse(":host add lab homelab").unwrap(),
            ClientCommand::HostAdd {
                name: "lab".to_string(),
                destination: "homelab".to_string()
            }
        );
        assert_eq!(
            parse(":workspace add '/srv/work folder'").unwrap(),
            ClientCommand::WorkspaceRegister {
                path: PathBuf::from("/srv/work folder")
            }
        );
    }

    #[test]
    fn worktree_scope_has_no_pr_create_merge_or_force_surface() {
        assert!(parse(":pr create").is_err());
        assert!(parse(":pr merge").is_err());
        assert!(parse(":worktree remove --force").is_err());
        assert!(parse(":workspace rename new-name").is_err());
    }

    #[test]
    fn parses_worktrunk_pr_url_as_one_argv() {
        let command = parse(":worktree open https://github.com/acme/app/pull/7").unwrap();
        assert!(
            matches!(command, ClientCommand::WorktreeOpen { selector } if selector.ends_with("/7"))
        );
    }

    #[test]
    fn quoted_configured_service_labels_remain_addressable() {
        assert_eq!(
            parse(":service start 'api worker / β'").unwrap(),
            ClientCommand::ServiceStart {
                name: "api worker / β".to_string(),
            }
        );
        assert!(parse(":service start 'line\nbreak'").is_err());
    }

    #[test]
    fn forward_selectors_support_exact_ipv4_and_ipv6_targets() {
        assert_eq!(
            parse(":forward 3000").unwrap(),
            ClientCommand::Forward {
                remote_port: 3000,
                bind_address: None,
            }
        );
        assert_eq!(
            parse(":forward 192.0.2.10:3000").unwrap(),
            ClientCommand::Forward {
                remote_port: 3000,
                bind_address: Some("192.0.2.10".to_string()),
            }
        );
        assert_eq!(
            parse(":forward cancel [::1]:3000").unwrap(),
            ClientCommand::ForwardCancel {
                remote_port: 3000,
                bind_address: Some("::1".to_string()),
            }
        );
        assert!(parse(":forward ::1:3000").unwrap_err().contains("IPv6"));
    }

    #[test]
    fn command_reference_includes_every_zero_argument_command() {
        let commands = HELP.iter().map(|(command, _)| *command).collect::<Vec<_>>();
        for command in [
            ":host import",
            ":workspace ungroup",
            ":worktree list",
            ":status explain",
            ":approve",
            ":refresh",
            ":help",
            ":quit",
        ] {
            assert!(
                commands.contains(&command),
                "missing help entry for {command}"
            );
        }
    }
}
