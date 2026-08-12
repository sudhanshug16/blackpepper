use super::super::{ClientMode, ClientState, HostConnection};
use crate::client::runtime::ClientRuntime;
use crate::core::HostTransport as StoredTransport;
use std::path::PathBuf;

const MAX_IMPORT_PREVIEWS: usize = 12;

pub(super) fn add(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    name: String,
    destination: String,
) -> Result<(), String> {
    let host_id = runtime.add_ssh_host(&name, &destination)?;
    let registered = runtime.find_host(&host_id.to_string())?;
    if !state.snapshot.hosts.iter().any(|host| host.id == host_id) {
        state.snapshot.hosts.push(registered.clone());
    }
    state
        .connections
        .insert(host_id, HostConnection::Disconnected);
    state.selected_host = Some(host_id);
    if registered.display_name == name {
        state.set_output(format!("Added SSH host {name}; use :host connect {name}."));
    } else {
        state.set_output(format!(
            "That SSH destination is already registered as {}; use :host connect {}.",
            registered.display_name, registered.display_name
        ));
    }
    Ok(())
}

pub(super) fn import(state: &mut ClientState, runtime: &mut ClientRuntime) -> Result<(), String> {
    let path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ssh/config");
    let host_id = runtime.local_host_id();
    let label = "Resolving SSH import preview".to_owned();
    let token = runtime.start_host_operation(
        host_id,
        label.clone(),
        crate::client::runtime::HostOperationContext::SshImportPreview,
        state.event_tx.clone(),
        Box::new(move |_| {
            let aliases = crate::ssh_config::discover_literal_aliases(&path)?;
            let alias_count = aliases.len();
            let mut previews = Vec::with_capacity(alias_count.min(MAX_IMPORT_PREVIEWS) + 1);
            for alias in aliases.into_iter().take(MAX_IMPORT_PREVIEWS) {
                if crate::transport::CommandCancellation::scope_is_cancelled() {
                    return Err("SSH import preview was cancelled.".to_owned());
                }
                match crate::ssh_config::preview_alias(std::path::Path::new("ssh"), &alias) {
                    Ok(preview) => previews.push(format!(
                        "{} → {}",
                        alias,
                        preview
                            .hostname
                            .as_deref()
                            .unwrap_or("OpenSSH-resolved host")
                    )),
                    Err(error) => previews.push(format!(
                        "{alias} → unavailable: {}",
                        bounded_line(&error, 240)
                    )),
                }
            }
            if alias_count > MAX_IMPORT_PREVIEWS {
                previews.push(format!(
                    "… {} more literal alias(es) not resolved here; add one explicitly with :host add <name> <alias>.",
                    alias_count - MAX_IMPORT_PREVIEWS
                ));
            }
            Ok(crate::client::runtime::HostOperationValue::SshImportPreview(previews))
        }),
    )?;
    state.host_operations.insert(host_id, (token, label));
    state.set_output("Resolving SSH aliases in the background; press Esc to cancel.");
    Ok(())
}

pub(in crate::client) fn apply_import_preview(state: &mut ClientState, previews: Vec<String>) {
    if previews.is_empty() {
        state.set_output("No literal positive aliases found in ~/.ssh/config.");
        return;
    }
    state.set_detail("SSH import preview", previews.join("\n"));
    state.set_output(
        "SSH import preview complete; unavailable and omitted aliases are shown explicitly. Esc closes the preview.",
    );
}

fn bounded_line(value: &str, max_chars: usize) -> String {
    let single_line = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = single_line.chars();
    let result = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{result}…")
    } else {
        result
    }
}

pub(super) fn connect(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    name: &str,
) -> Result<(), String> {
    let host = runtime.find_host(name)?;
    if matches!(host.transport, StoredTransport::Local) {
        state.selected_host = Some(host.id);
        state.set_output(format!("Selected local host {}.", host.display_name));
        return Ok(());
    }
    if matches!(
        state.connections.get(&host.id),
        Some(HostConnection::Connected | HostConnection::Reconnecting)
    ) {
        state.selected_host = Some(host.id);
        state.set_output(
            if state.connections.get(&host.id) == Some(&HostConnection::Reconnecting) {
                format!(
                    "Selected {}; its SSH registry and workspaces are still being restored.",
                    host.display_name
                )
            } else {
                format!("Selected connected host {}.", host.display_name)
            },
        );
        return Ok(());
    }
    runtime.start_connection(host.clone(), state.event_tx.clone())?;
    state
        .connections
        .insert(host.id, HostConnection::Authenticating);
    state.selected_host = Some(host.id);
    state.authentication_host = Some(host.id);
    state.authentication_output.clear();
    state.mode = ClientMode::Authenticate;
    Ok(())
}

pub(super) fn disconnect(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    name: &str,
) -> Result<(), String> {
    let host = runtime.find_host(name)?;
    let report = runtime.disconnect_host_with_restores(host.id)?;
    super::super::runner::operations::apply_deferred_results(state, report.deferred_results);
    for forward in state
        .forwards
        .iter_mut()
        .filter(|forward| forward.host_id == host.id)
    {
        forward.mark_reconnecting();
    }
    state
        .connections
        .insert(host.id, HostConnection::Disconnected);
    for connection_id in report.restoring_host_ids {
        state
            .connections
            .insert(connection_id, HostConnection::Disconnected);
    }
    // A first connection is keyed by a temporary config-derived ID until its
    // background registry handshake proves the host's stable identity. Mark
    // every restoring alias for the same destination disconnected so a queued
    // completion cannot make it appear connected again.
    if let StoredTransport::Ssh { destination } = &host.transport {
        for record in &state.snapshot.hosts {
            if matches!(
                &record.transport,
                StoredTransport::Ssh { destination: candidate } if candidate == destination
            ) && state.connections.get(&record.id) == Some(&HostConnection::Reconnecting)
            {
                state
                    .connections
                    .insert(record.id, HostConnection::Disconnected);
            }
        }
    }
    state.terminals.retain(|workspace_id, _| {
        state
            .snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.id == *workspace_id)
            .is_none_or(|workspace| workspace.host_id != host.id)
    });
    state
        .connected_clients
        .retain(|workspace_id, _| state.terminals.contains_key(workspace_id));
    state.set_output(match report.warning {
        Some(warning) => format!(
            "Disconnected from {}; sessions remain running. {warning}",
            host.display_name
        ),
        None => format!(
            "Disconnected from {}; sessions remain running.",
            host.display_name
        ),
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_errors_are_bounded_and_single_line() {
        let input = format!("first line\n{}", "x".repeat(300));
        let output = bounded_line(&input, 40);
        assert!(!output.contains('\n'));
        assert_eq!(output.chars().count(), 41);
        assert!(output.ends_with('…'));
    }
}
