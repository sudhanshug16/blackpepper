use super::{ClientRuntime, HostSlot, SshHost};
use crate::client::ClientEvent;
use crate::core::{HostId, HostRecord, HostTransport as StoredTransport};
use crate::transport::{ConnectionState, SshConfig, SshTransport};
use portable_pty::PtySize;
use std::io::Read;
use std::sync::mpsc::Sender;

#[cfg(test)]
use std::collections::BTreeSet;

mod operations;
mod registry_sync;
mod remote_helper;

use operations::path_text;
pub(super) use operations::{refresh_registry, registry_operation, registry_operation_tracked};
pub(super) use registry_sync::{reconcile_remote_snapshot, synchronize_registry_with_reserved};

#[cfg(test)]
use registry_sync::{validate_remote_host_identity, validate_reserved_host_identity};

#[derive(Debug)]
pub(crate) enum ConnectionUpdate {
    Ready { previous: HostId, host_id: HostId },
    Failed { host_id: HostId, message: String },
}

pub(super) fn start(
    runtime: &mut ClientRuntime,
    host: HostRecord,
    sender: Sender<ClientEvent>,
) -> Result<(), String> {
    let StoredTransport::Ssh { destination } = &host.transport else {
        return Err("The local host is always connected.".to_string());
    };
    if host.id == runtime.local_host_id {
        return Err(
            "An SSH connection cannot replace this client's local host identity.".to_owned(),
        );
    }
    if runtime.hosts.contains_key(&host.id) {
        return Err("This SSH host is already connecting or connected.".to_owned());
    }
    let mut transport =
        SshTransport::new(SshConfig::new(destination)).map_err(|error| error.to_string())?;
    transport
        .start_master(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| error.to_string())?;
    let mut reader = transport
        .master_pty_mut()
        .ok_or_else(|| "SSH authentication PTY was not created.".to_string())?
        .take_reader()
        .map_err(|error| error.to_string())?;
    let host_id = host.id;
    std::thread::spawn(move || read_authentication(host_id, &mut reader, sender));
    runtime.hosts.insert(
        host.id,
        HostSlot::Ssh(Box::new(SshHost {
            alias: host.display_name,
            transport,
            registry_synchronized: false,
            registry_synchronizing: false,
        })),
    );
    Ok(())
}

pub(super) fn poll(runtime: &mut ClientRuntime) -> Vec<ConnectionUpdate> {
    let pending = runtime
        .hosts
        .iter()
        .filter_map(|(id, slot)| match slot {
            HostSlot::Ssh(_) => Some(*id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut updates = Vec::new();
    for host_id in pending {
        let (state, synchronized, synchronizing) = match runtime.hosts.get_mut(&host_id) {
            Some(HostSlot::Ssh(host)) => (
                host.transport.poll_connection(),
                host.registry_synchronized,
                host.registry_synchronizing,
            ),
            _ => continue,
        };
        match state {
            Ok(ConnectionState::Ready) if !synchronized && !synchronizing => {
                if let Some(HostSlot::Ssh(host)) = runtime.hosts.get_mut(&host_id) {
                    host.registry_synchronizing = true;
                }
                // Helper discovery/upload and registry synchronization can be
                // slow on a cold remote host. The runner moves this exact
                // transport into its generation-checked restoration worker.
                updates.push(ConnectionUpdate::Ready {
                    previous: host_id,
                    host_id,
                });
            }
            Ok(ConnectionState::Failed { status }) => {
                runtime.hosts.remove(&host_id);
                runtime.helper_paths.remove(&host_id);
                updates.push(ConnectionUpdate::Failed {
                    host_id,
                    message: format!("SSH control master exited ({status:?})."),
                });
            }
            Err(error) => {
                runtime.hosts.remove(&host_id);
                runtime.helper_paths.remove(&host_id);
                updates.push(ConnectionUpdate::Failed {
                    host_id,
                    message: error.to_string(),
                });
            }
            _ => {}
        }
    }
    updates
}

pub(super) fn send_input(
    runtime: &mut ClientRuntime,
    host_id: HostId,
    bytes: &[u8],
) -> Result<(), String> {
    let Some(HostSlot::Ssh(host)) = runtime.hosts.get_mut(&host_id) else {
        return Err("SSH authentication is no longer active.".to_string());
    };
    host.transport
        .master_pty_mut()
        .ok_or_else(|| "SSH authentication is no longer active.".to_string())?
        .write_all(bytes)
        .map_err(|error| error.to_string())
}

pub(super) fn disconnect(runtime: &mut ClientRuntime, host_id: HostId) -> Result<(), String> {
    if host_id == runtime.local_host_id {
        return Err("The local host cannot be disconnected.".to_string());
    }
    let Some(mut slot) = runtime.hosts.remove(&host_id) else {
        return Ok(());
    };
    runtime.helper_paths.remove(&host_id);
    match &mut slot {
        HostSlot::Local(_) => unreachable!("the local host was handled above"),
        HostSlot::Ssh(host) => host
            .transport
            .disconnect()
            .map_err(|error| error.to_string()),
    }
}

/// Drop a restoration-owned transport without waiting for a cooperative mux
/// exit. `SshTransport::drop` kills and reaps the foreground master; every
/// restore child has already received its fail-closed cancellation first.
pub(super) fn abort(runtime: &mut ClientRuntime, host_id: HostId) {
    runtime.helper_paths.remove(&host_id);
    runtime.hosts.remove(&host_id);
}

fn read_authentication(host_id: HostId, reader: &mut dyn Read, sender: Sender<ClientEvent>) {
    let mut buffer = [0u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(size) => {
                if sender
                    .send(ClientEvent::HostAuthenticationOutput(
                        host_id,
                        buffer[..size].to_vec(),
                    ))
                    .is_err()
                {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
