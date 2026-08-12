use super::{AttributionConfidence, PortListener, PortSnapshot, ProbeCompleteness};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeCommand {
    pub program: &'static str,
    pub args: &'static [&'static str],
}

pub fn platform_probe() -> Result<ProbeCommand, String> {
    if cfg!(target_os = "linux") {
        Ok(ProbeCommand {
            program: "ss",
            args: &["-H", "-ltnp"],
        })
    } else if cfg!(target_os = "macos") {
        Ok(ProbeCommand {
            program: "lsof",
            args: &["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpcn"],
        })
    } else {
        Err("Port discovery is supported on Linux and macOS only.".to_string())
    }
}

pub fn parse_linux_ss(stdout: &str, stderr: &str) -> PortSnapshot {
    let mut listeners = Vec::new();
    let mut rejected = 0_usize;
    let mut rows = 0_usize;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        rows += 1;
        let parsed = parse_ss_line(line);
        if parsed.is_empty() {
            rejected += 1;
        } else {
            listeners.extend(parsed);
        }
    }
    listeners.sort_by_key(|listener| listener.port);
    let missing_processes = listeners.iter().any(|listener| listener.pid.is_none());
    let mut warnings = Vec::new();
    if !stderr.trim().is_empty() {
        warnings.push(format!(
            "Port process details are incomplete: {}",
            stderr.trim()
        ));
    }
    if missing_processes {
        warnings.push("Some listening processes are hidden by host permissions.".to_string());
    }
    if rejected != 0 {
        warnings.push(format!(
            "Could not parse {rejected} non-empty ss listener row(s)."
        ));
    }
    let warning = (!warnings.is_empty()).then(|| warnings.join(" "));
    PortSnapshot {
        listeners,
        completeness: if rows != 0 && rejected == rows {
            ProbeCompleteness::Failed
        } else if warning.is_some() {
            ProbeCompleteness::Partial
        } else {
            ProbeCompleteness::Full
        },
        warning,
    }
}

fn parse_ss_line(line: &str) -> Vec<PortListener> {
    let fields = line.split_whitespace().collect::<Vec<_>>();
    if fields.len() < 4 || fields[0] != "LISTEN" {
        return Vec::new();
    }
    let Some((bind_address, port)) = split_socket_address(fields[3]) else {
        return Vec::new();
    };
    let process_blob = fields.get(5..).unwrap_or_default().join(" ");
    let processes = linux_processes(&process_blob);
    if processes.is_empty() {
        return vec![listener(bind_address, port, None, None)];
    }
    processes
        .into_iter()
        .map(|(pid, process)| listener(bind_address.clone(), port, Some(pid), process))
        .collect()
}

fn linux_processes(blob: &str) -> Vec<(u32, Option<String>)> {
    let mut found = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = blob[cursor..].find("pid=") {
        let start = cursor + relative;
        let Some(pid) = value_after(&blob[start..], "pid=").and_then(|value| value.parse().ok())
        else {
            cursor = start.saturating_add(4);
            continue;
        };
        let process = blob[..start].rfind("(\"").and_then(|name_start| {
            let value = &blob[name_start + 2..];
            value.find('"').map(|end| value[..end].to_string())
        });
        found.push((pid, process));
        cursor = start.saturating_add(4);
    }
    found
}

fn listener(
    bind_address: String,
    port: u16,
    pid: Option<u32>,
    process: Option<String>,
) -> PortListener {
    PortListener {
        bind_address,
        port,
        pid,
        process,
        workspace_path: None,
        attribution: AttributionConfidence::Unavailable,
    }
}

fn split_socket_address(value: &str) -> Option<(String, u16)> {
    let split = value.rfind(':')?;
    let port = value[split + 1..].parse().ok()?;
    let address = value[..split].trim_matches(['[', ']']).to_string();
    Some((address, port))
}

fn value_after<'a>(input: &'a str, marker: &str) -> Option<&'a str> {
    let start = input.find(marker)? + marker.len();
    let tail = &input[start..];
    let end = tail
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(tail.len());
    Some(&tail[..end])
}

pub fn parse_macos_lsof(stdout: &str, stderr: &str) -> PortSnapshot {
    let mut listeners = Vec::new();
    let mut pid = None;
    let mut process = None;
    let mut rejected = 0_usize;
    let mut name_records = 0_usize;
    for line in stdout.lines() {
        match line.chars().next() {
            Some('p') => pid = line[1..].parse().ok(),
            Some('c') => process = Some(line[1..].to_string()),
            Some('n') => {
                name_records += 1;
                if let Some((bind_address, port)) = split_socket_address(&line[1..]) {
                    listeners.push(PortListener {
                        bind_address,
                        port,
                        pid,
                        process: process.clone(),
                        workspace_path: None,
                        attribution: AttributionConfidence::Unavailable,
                    });
                } else {
                    rejected += 1;
                }
            }
            Some(_) if !line.trim().is_empty() => rejected += 1,
            _ => {}
        }
    }
    listeners.sort_by_key(|listener| listener.port);
    let mut warnings = Vec::new();
    if !stderr.trim().is_empty() {
        warnings.push(stderr.trim().to_string());
    }
    if rejected != 0 {
        warnings.push(format!(
            "Could not parse {rejected} lsof listener record(s)."
        ));
    }
    let warning = (!warnings.is_empty()).then(|| warnings.join(" "));
    let all_name_records_rejected = name_records != 0 && listeners.is_empty();
    PortSnapshot {
        listeners,
        completeness: if all_name_records_rejected {
            ProbeCompleteness::Failed
        } else if warning.is_some() {
            ProbeCompleteness::Partial
        } else {
            ProbeCompleteness::Full
        },
        warning,
    }
}

pub fn attribute_linux_cwds(snapshot: &mut PortSnapshot, workspaces: &[PathBuf]) {
    attribute_linux_cwds_with(snapshot, workspaces, |pid| {
        std::fs::read_link(format!("/proc/{pid}/cwd"))
    });
}

fn attribute_linux_cwds_with(
    snapshot: &mut PortSnapshot,
    workspaces: &[PathBuf],
    mut resolve: impl FnMut(u32) -> std::io::Result<PathBuf>,
) {
    if workspaces.is_empty() {
        return;
    }
    let mut failures = 0_usize;
    for listener in &mut snapshot.listeners {
        let Some(pid) = listener.pid else {
            continue;
        };
        let Ok(cwd) = resolve(pid) else {
            failures += 1;
            continue;
        };
        if let Some(workspace) = longest_containing_path(&cwd, workspaces) {
            listener.workspace_path = Some(workspace.clone());
            listener.attribution = AttributionConfidence::ExactCwd;
        }
    }
    if failures != 0 {
        snapshot.completeness = ProbeCompleteness::Partial;
        append_warning(
            snapshot,
            &format!(
                "Could not inspect the working directory for {failures} listener process(es)."
            ),
        );
    }
}

fn append_warning(snapshot: &mut PortSnapshot, warning: &str) {
    snapshot.warning = Some(match snapshot.warning.take() {
        Some(existing) => format!("{existing} {warning}"),
        None => warning.to_string(),
    });
}

fn longest_containing_path<'a>(cwd: &Path, roots: &'a [PathBuf]) -> Option<&'a PathBuf> {
    roots
        .iter()
        .filter(|root| cwd.starts_with(root))
        .max_by_key(|root| root.components().count())
}

pub fn failed_probe(message: impl Into<String>) -> PortSnapshot {
    PortSnapshot {
        listeners: Vec::new(),
        completeness: ProbeCompleteness::Failed,
        warning: Some(message.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_probe_schemas_do_not_look_like_an_empty_full_result() {
        let linux = parse_linux_ss("LISTEN changed-schema\n", "");
        assert_eq!(linux.completeness, ProbeCompleteness::Failed);
        assert!(linux.warning.unwrap().contains("Could not parse"));

        let macos = parse_macos_lsof("p42\ncapi\nnchanged-schema\n", "");
        assert_eq!(macos.completeness, ProbeCompleteness::Failed);
        assert!(macos.warning.unwrap().contains("Could not parse"));
    }

    #[test]
    fn denied_linux_cwd_attribution_is_visible() {
        let mut snapshot = PortSnapshot {
            listeners: vec![listener(
                "127.0.0.1".to_string(),
                8080,
                Some(42),
                Some("api".to_string()),
            )],
            completeness: ProbeCompleteness::Full,
            warning: None,
        };
        attribute_linux_cwds_with(&mut snapshot, &[PathBuf::from("/srv/app")], |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "denied",
            ))
        });

        assert_eq!(snapshot.completeness, ProbeCompleteness::Partial);
        assert!(snapshot.warning.unwrap().contains("working directory"));
    }
}
