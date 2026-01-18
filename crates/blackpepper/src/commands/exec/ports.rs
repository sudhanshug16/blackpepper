//! List workspace port allocations and their status.

use std::process::Command;

use crate::state::{get_workspace_ports, PORT_BLOCK_SIZE};

use super::{CommandContext, CommandResult};

/// Maximum length for process name display.
const MAX_PROCESS_NAME_LEN: usize = 20;

/// Show allocated ports and their status (free/occupied).
pub(crate) fn ports_list(ctx: &CommandContext) -> CommandResult {
    let Some(workspace_path) = ctx.workspace_path.as_ref() else {
        return CommandResult {
            ok: false,
            message: "No active workspace. Switch to a workspace first.".to_string(),
            data: None,
        };
    };

    let base_port = match get_workspace_ports(workspace_path) {
        Some(port) => port,
        None => {
            return CommandResult {
                ok: false,
                message: "No ports allocated for this workspace.".to_string(),
                data: None,
            };
        }
    };

    let mut lines = Vec::new();
    lines.push("PORT   STATUS      PROCESS".to_string());
    lines.push("─────  ──────────  ────────────────────".to_string());

    for i in 0..PORT_BLOCK_SIZE {
        let port = base_port + i;
        let env_name = format!("WORKSPACE_PORT_{i}");
        let status = check_port_status(port);
        let status_display = match &status {
            PortStatus::Free => "free        -".to_string(),
            PortStatus::Occupied { pid, name } => {
                let trimmed_name = truncate_name(name, MAX_PROCESS_NAME_LEN);
                format!("occupied    [{pid}] {trimmed_name}")
            }
        };
        lines.push(format!("{port}   {status_display}  ({env_name})"));
    }

    CommandResult {
        ok: true,
        message: lines.join("\n"),
        data: None,
    }
}

enum PortStatus {
    Free,
    Occupied { pid: u32, name: String },
}

/// Check if a port is in use and return the process info if occupied.
fn check_port_status(port: u16) -> PortStatus {
    // Use lsof to check if port is in use
    let output = Command::new("lsof")
        .args(["-i", &format!(":{port}"), "-t", "-sTCP:LISTEN"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let pid_str = stdout.lines().next().unwrap_or("").trim();
            if let Ok(pid) = pid_str.parse::<u32>() {
                let name = get_process_name(pid).unwrap_or_else(|| "unknown".to_string());
                PortStatus::Occupied { pid, name }
            } else {
                PortStatus::Free
            }
        }
        _ => PortStatus::Free,
    }
}

/// Get the process name for a given PID.
fn get_process_name(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;

    if output.status.success() {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    } else {
        None
    }
}

/// Truncate a name to a maximum length, adding ellipsis if needed.
fn truncate_name(name: &str, max_len: usize) -> String {
    if name.len() <= max_len {
        name.to_string()
    } else if max_len > 3 {
        format!("{}...", &name[..max_len - 3])
    } else {
        name[..max_len].to_string()
    }
}
