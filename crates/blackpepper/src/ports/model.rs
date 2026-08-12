use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::path::PathBuf;

use crate::core::{HostId, WorkspaceId};

/// The host-side TCP endpoint selected from one discovered listener.
///
/// Keeping the address (rather than only the port) is required because two
/// services can listen on the same port on different interfaces.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RemotePortTarget {
    pub remote_host: String,
    pub remote_port: u16,
}

impl RemotePortTarget {
    pub fn from_bind_address(bind_address: &str, remote_port: u16) -> Result<Self, String> {
        if remote_port == 0 {
            return Err("A forwarded port must be non-zero.".to_string());
        }
        Ok(Self {
            remote_host: normalize_bind_address(bind_address)?,
            remote_port,
        })
    }

    pub fn endpoint(&self) -> String {
        format_endpoint(&self.remote_host, self.remote_port)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeCompleteness {
    Full,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionConfidence {
    ExactCwd,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortListener {
    pub bind_address: String,
    pub port: u16,
    pub pid: Option<u32>,
    pub process: Option<String>,
    pub workspace_path: Option<PathBuf>,
    pub attribution: AttributionConfidence,
}

impl PortListener {
    pub fn is_loopback(&self) -> bool {
        self.bind_address == "localhost"
            || self
                .bind_address
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    }

    pub fn forward_target(&self) -> Result<RemotePortTarget, String> {
        RemotePortTarget::from_bind_address(&self.bind_address, self.port)
    }

    pub fn bind_endpoint(&self) -> String {
        format_endpoint(&self.bind_address, self.port)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortSnapshot {
    pub listeners: Vec<PortListener>,
    pub completeness: ProbeCompleteness,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwardStatus {
    /// The service already listens on client loopback; no tunnel is owned.
    Direct,
    Active,
    Reconnecting,
    /// The owning workspace disappeared and the exact client-owned tunnel is
    /// being cancelled outside the render thread.
    Cancelling,
    PortConflict,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardState {
    pub id: uuid::Uuid,
    pub host_id: HostId,
    pub workspace_id: WorkspaceId,
    pub remote_host: String,
    pub remote_port: u16,
    pub requested_local_address: SocketAddr,
    pub local_address: SocketAddr,
    pub status: ForwardStatus,
}

impl ForwardState {
    pub fn new(
        host_id: HostId,
        workspace_id: WorkspaceId,
        target: RemotePortTarget,
    ) -> std::io::Result<Self> {
        let local_port = choose_initial_local_port(target.remote_port)?;
        let requested_local_address =
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), target.remote_port);
        Ok(Self {
            id: uuid::Uuid::new_v4(),
            host_id,
            workspace_id,
            remote_host: target.remote_host,
            remote_port: target.remote_port,
            requested_local_address,
            local_address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), local_port),
            status: ForwardStatus::Active,
        })
    }

    pub fn target(&self) -> RemotePortTarget {
        RemotePortTarget {
            remote_host: self.remote_host.clone(),
            remote_port: self.remote_port,
        }
    }

    pub fn remote_endpoint(&self) -> String {
        self.target().endpoint()
    }

    /// Reconnects must keep the exact URL. A collision is reported rather than
    /// silently choosing a different port.
    pub fn mark_reconnecting(&mut self) {
        if self.status == ForwardStatus::Direct {
            return;
        }
        self.status = if port_is_available(self.local_address.port()) {
            ForwardStatus::Reconnecting
        } else {
            ForwardStatus::PortConflict
        };
    }
}

/// Select one exact discovered listener. Port-only selection is accepted only
/// when it cannot silently choose between interfaces or processes.
pub fn resolve_forward_target<'a>(
    listeners: impl IntoIterator<Item = &'a PortListener>,
    remote_port: u16,
    requested_bind_address: Option<&str>,
) -> Result<RemotePortTarget, String> {
    let requested_target = requested_bind_address
        .map(|address| RemotePortTarget::from_bind_address(address, remote_port))
        .transpose()?;
    let listeners = listeners
        .into_iter()
        .filter(|listener| listener.port == remote_port)
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for listener in &listeners {
        let target = listener.forward_target().map_err(|error| {
            format!(
                "Cannot forward discovered listener {}: {error}",
                listener.bind_endpoint()
            )
        })?;
        if requested_target
            .as_ref()
            .is_none_or(|requested| requested == &target)
        {
            candidates.push(target);
        }
    }

    match candidates.as_slice() {
        [target] if !target_is_ambiguous(listeners.iter().copied(), target) => Ok(target.clone()),
        [target] => Err(format!(
            "Multiple processes can accept {}; TCP forwarding cannot select one process. Stop the overlapping listener or use another port.",
            target.endpoint()
        )),
        [] if requested_bind_address.is_some() => Err(format!(
            "No discovered listener matches {}. Run :ports --all-host, then use an address exactly as listed.",
            requested_target.expect("present when an address was requested").endpoint()
        )),
        [] => Err(format!(
            "No discovered listener uses port {remote_port}. Run :ports first (or :ports --all-host for unattributed services)."
        )),
        _ => {
            let distinct = candidates
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            if distinct.len() == 1 {
                Err(format!(
                    "Multiple processes share {}; TCP forwarding cannot select one process. Stop the duplicate listener or use another port.",
                    distinct.iter().next().expect("non-empty").endpoint()
                ))
            } else {
                Err(format!(
                    "Port {remote_port} has multiple listener addresses ({}). Click the exact listener or run :forward <address>:{remote_port}.",
                    distinct
                        .iter()
                        .map(RemotePortTarget::endpoint)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        }
    }
}

/// Whether more than one discovered socket can receive a connection to this
/// exact target. Wildcard sockets overlap specific interfaces even though
/// their printed bind addresses differ.
pub fn target_is_ambiguous<'a>(
    listeners: impl IntoIterator<Item = &'a PortListener>,
    target: &RemotePortTarget,
) -> bool {
    listeners
        .into_iter()
        .filter(|listener| listener_accepts_target(listener, target))
        .take(2)
        .count()
        > 1
}

fn listener_accepts_target(listener: &PortListener, target: &RemotePortTarget) -> bool {
    if listener.port != target.remote_port {
        return false;
    }
    match listener.bind_address.trim() {
        "" | "*" | "::" => true,
        "0.0.0.0" => target.remote_host.parse::<std::net::Ipv4Addr>().is_ok(),
        _ => listener
            .forward_target()
            .is_ok_and(|candidate| candidate == *target),
    }
}

fn normalize_bind_address(bind_address: &str) -> Result<String, String> {
    let address = bind_address.trim();
    match address {
        "" | "*" | "0.0.0.0" => Ok(Ipv4Addr::LOCALHOST.to_string()),
        "::" => Ok(std::net::Ipv6Addr::LOCALHOST.to_string()),
        "localhost" => Ok(Ipv4Addr::LOCALHOST.to_string()),
        value => {
            if let Some((address, scope)) = value.rsplit_once('%') {
                let address = address
                    .parse::<std::net::Ipv6Addr>()
                    .map_err(|_| format!("unsupported bind address '{value}'"))?;
                if scope.is_empty()
                    || !scope.chars().all(|character| {
                        character.is_ascii_alphanumeric() || "_.-".contains(character)
                    })
                {
                    return Err(format!("unsupported IPv6 scope in '{value}'"));
                }
                return Ok(format!("{address}%{scope}"));
            }
            value
                .parse::<IpAddr>()
                .map(|address| address.to_string())
                .map_err(|_| format!("unsupported bind address '{value}'"))
        }
    }
}

fn format_endpoint(host: &str, port: u16) -> String {
    if host.contains(':') && !(host.starts_with('[') && host.ends_with(']')) {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

pub fn choose_initial_local_port(preferred: u16) -> std::io::Result<u16> {
    match TcpListener::bind((Ipv4Addr::LOCALHOST, preferred)) {
        Ok(listener) => Ok(listener.local_addr()?.port()),
        Err(_) => {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
            Ok(listener.local_addr()?.port())
        }
    }
}

pub fn port_is_available(port: u16) -> bool {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok()
}
