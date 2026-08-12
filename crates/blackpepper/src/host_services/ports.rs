use super::process::run_bounded;
use crate::core::HostRegistry;
use crate::ports::{
    attribute_linux_cwds, failed_probe, parse_linux_ss, parse_macos_lsof, platform_probe,
    AttributionConfidence, PortSnapshot, ProbeCompleteness,
};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::PathBuf;

const MAX_MACOS_CWD_PROBES: usize = 512;

pub(super) fn discover(registry: &HostRegistry) -> PortSnapshot {
    let probe = match platform_probe() {
        Ok(probe) => probe,
        Err(message) => return failed_probe(message),
    };
    let output = match run_bounded(OsStr::new(probe.program), probe.args) {
        Ok(output) => output,
        Err(error) => return failed_probe(format!("Could not run {}: {error}", probe.program)),
    };
    if !output.status.success() {
        return failed_probe(format!(
            "{} listener discovery failed: {}",
            probe.program,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut snapshot = if cfg!(target_os = "linux") {
        parse_linux_ss(&stdout, &stderr)
    } else {
        parse_macos_lsof(&stdout, &stderr)
    };
    if output.truncated {
        snapshot.completeness = ProbeCompleteness::Partial;
        append_warning(
            &mut snapshot,
            "Listener output exceeded the safe capture limit.",
        );
    }
    let workspaces = registry
        .snapshot()
        .map(|snapshot| {
            snapshot
                .workspaces
                .into_iter()
                .map(|workspace| PathBuf::from(workspace.root_path))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|error| {
            snapshot.completeness = ProbeCompleteness::Partial;
            append_warning(
                &mut snapshot,
                &format!("Workspace attribution is unavailable: {error}"),
            );
            Vec::new()
        });
    if cfg!(target_os = "linux") {
        attribute_linux_cwds(&mut snapshot, &workspaces);
    } else if cfg!(target_os = "macos") {
        attribute_macos_cwds(&mut snapshot, &workspaces);
    }
    snapshot
}

fn attribute_macos_cwds(snapshot: &mut PortSnapshot, workspaces: &[PathBuf]) {
    if workspaces.is_empty() {
        return;
    }
    let pids = snapshot
        .listeners
        .iter()
        .filter_map(|listener| listener.pid)
        .collect::<BTreeSet<_>>();
    if pids.len() > MAX_MACOS_CWD_PROBES {
        snapshot.completeness = ProbeCompleteness::Partial;
        append_warning(snapshot, "Too many processes to attribute every listener.");
    }
    let inspected = pids
        .into_iter()
        .take(MAX_MACOS_CWD_PROBES)
        .collect::<Vec<_>>();
    if inspected.is_empty() {
        return;
    }
    let pid_list = inspected
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let output = match run_bounded(
        OsStr::new("lsof"),
        ["-a", "-p", &pid_list, "-d", "cwd", "-Fpn"],
    ) {
        Ok(output) => output,
        Err(error) => {
            snapshot.completeness = ProbeCompleteness::Partial;
            append_warning(
                snapshot,
                &format!(
                    "Could not inspect the working directory for {} listener process(es): {error}",
                    inspected.len()
                ),
            );
            return;
        }
    };
    let cwd_by_pid = parse_macos_cwd_output(&output.stdout);
    let mut failures = 0_usize;
    for pid in inspected {
        let Some(cwd) = cwd_by_pid.get(&pid) else {
            failures += 1;
            continue;
        };
        let workspace = workspaces
            .iter()
            .filter(|root| cwd.starts_with(root))
            .max_by_key(|root| root.components().count());
        for listener in snapshot
            .listeners
            .iter_mut()
            .filter(|listener| listener.pid == Some(pid))
        {
            if let Some(workspace) = workspace {
                listener.workspace_path = Some(workspace.clone());
                listener.attribution = AttributionConfidence::ExactCwd;
            }
        }
    }
    if !output.status.success() || output.truncated {
        snapshot.completeness = ProbeCompleteness::Partial;
        append_warning(
            snapshot,
            "The batched lsof working-directory probe was incomplete.",
        );
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

fn parse_macos_cwd_output(output: &[u8]) -> BTreeMap<u32, PathBuf> {
    let mut current_pid = None;
    let mut paths = BTreeMap::new();
    for line in String::from_utf8_lossy(output).lines() {
        if let Some(value) = line.strip_prefix('p') {
            current_pid = value.parse().ok();
        } else if let (Some(pid), Some(path)) = (current_pid, line.strip_prefix('n')) {
            paths.insert(pid, PathBuf::from(path));
        }
    }
    paths
}

fn append_warning(snapshot: &mut PortSnapshot, warning: &str) {
    snapshot.warning = Some(match snapshot.warning.take() {
        Some(existing) => format!("{existing} {warning}"),
        None => warning.to_owned(),
    });
}

#[cfg(test)]
mod tests {
    use super::parse_macos_cwd_output;
    use std::path::PathBuf;

    #[test]
    fn batched_macos_lsof_cwds_stay_associated_with_each_pid() {
        let parsed =
            parse_macos_cwd_output(b"p41\nfcwd\nn/Users/dev/one\np57\nfcwd\nn/Users/dev/two\n");
        assert_eq!(parsed.get(&41), Some(&PathBuf::from("/Users/dev/one")));
        assert_eq!(parsed.get(&57), Some(&PathBuf::from("/Users/dev/two")));
    }
}
