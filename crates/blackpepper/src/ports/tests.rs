use super::{
    parse_linux_ss, parse_macos_lsof, resolve_forward_target, AttributionConfidence, ForwardState,
    ForwardStatus, PortListener, ProbeCompleteness, RemotePortTarget,
};
use std::net::{Ipv4Addr, TcpListener};

#[test]
fn parses_linux_ss_with_and_without_process_visibility() {
    let output = concat!(
        "LISTEN 0 200 127.0.0.1:5432 0.0.0.0:* users:((\"postgres\",pid=42,fd=7))\n",
        "LISTEN 0 128 [::]:22 [::]:*\n"
    );
    let snapshot = parse_linux_ss(output, "");
    assert_eq!(snapshot.listeners.len(), 2);
    assert_eq!(snapshot.listeners[0].port, 22);
    assert_eq!(snapshot.listeners[1].pid, Some(42));
    assert_eq!(snapshot.listeners[1].process.as_deref(), Some("postgres"));
    assert_eq!(snapshot.completeness, ProbeCompleteness::Partial);
    assert!(snapshot.warning.unwrap().contains("permissions"));
}

#[test]
fn linux_probe_failure_is_not_reported_as_no_ports() {
    let snapshot = parse_linux_ss("", "ss: permission denied");
    assert!(snapshot.listeners.is_empty());
    assert_eq!(snapshot.completeness, ProbeCompleteness::Partial);
    assert!(snapshot.warning.unwrap().contains("permission denied"));
}

#[test]
fn linux_shared_socket_emits_one_listener_per_process() {
    let output = concat!(
        "LISTEN 0 128 127.0.0.1:8080 0.0.0.0:* ",
        "users:((\"api\",pid=42,fd=7),(\"worker\",pid=84,fd=9))\n"
    );

    let snapshot = parse_linux_ss(output, "");

    assert_eq!(snapshot.listeners.len(), 2);
    assert_eq!(snapshot.listeners[0].pid, Some(42));
    assert_eq!(snapshot.listeners[0].process.as_deref(), Some("api"));
    assert_eq!(snapshot.listeners[1].pid, Some(84));
    assert_eq!(snapshot.listeners[1].process.as_deref(), Some("worker"));
    assert!(resolve_forward_target(&snapshot.listeners, 8080, None)
        .unwrap_err()
        .contains("TCP forwarding cannot select one process"));
}

#[test]
fn parses_macos_lsof_field_output() {
    let output = "p123\ncnode\nn127.0.0.1:3000\np456\ncapi\nn*:8080\n";
    let snapshot = parse_macos_lsof(output, "");
    assert_eq!(snapshot.listeners.len(), 2);
    assert_eq!(snapshot.listeners[0].pid, Some(123));
    assert_eq!(snapshot.listeners[0].process.as_deref(), Some("node"));
    assert_eq!(snapshot.listeners[1].port, 8080);
    assert_eq!(
        snapshot.listeners[1].attribution,
        AttributionConfidence::Unavailable
    );
}

#[test]
fn initial_forward_falls_back_but_reconnect_does_not_change_url() {
    let occupied = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let occupied_port = occupied.local_addr().unwrap().port();
    let mut forward = ForwardState::new(
        crate::core::HostId::new(),
        crate::core::WorkspaceId::new(),
        RemotePortTarget::from_bind_address("127.0.0.1", occupied_port).unwrap(),
    )
    .unwrap();
    assert_eq!(forward.requested_local_address.port(), occupied_port);
    assert_ne!(forward.local_address.port(), occupied_port);

    let selected = forward.local_address.port();
    let blocker = TcpListener::bind((Ipv4Addr::LOCALHOST, selected)).unwrap();
    forward.mark_reconnecting();
    assert_eq!(forward.local_address.port(), selected);
    assert_eq!(forward.status, ForwardStatus::PortConflict);
    drop(blocker);
}

#[test]
fn listener_targets_preserve_specific_addresses_and_normalize_wildcards() {
    let target = |address, port| RemotePortTarget::from_bind_address(address, port).unwrap();

    assert_eq!(target("0.0.0.0", 3000).remote_host, "127.0.0.1");
    assert_eq!(target("::", 3000).remote_host, "::1");
    assert_eq!(target("::1", 3000).endpoint(), "[::1]:3000");
    assert_eq!(
        target("fe80::1%eth0", 3000).endpoint(),
        "[fe80::1%eth0]:3000"
    );
    assert_eq!(target("192.0.2.8", 3000).remote_host, "192.0.2.8");
    assert!(RemotePortTarget::from_bind_address("not an address", 3000).is_err());
}

#[test]
fn port_only_selection_rejects_different_addresses() {
    let listeners = [
        listener("127.0.0.1", 3000, Some(10)),
        listener("::1", 3000, Some(11)),
    ];

    let error = resolve_forward_target(&listeners, 3000, None).unwrap_err();
    assert!(error.contains("multiple listener addresses"));
    assert_eq!(
        resolve_forward_target(&listeners, 3000, Some("::1")).unwrap(),
        RemotePortTarget::from_bind_address("::1", 3000).unwrap()
    );
}

#[test]
fn same_socket_shared_by_processes_is_not_selectable() {
    let listeners = [
        listener("0.0.0.0", 8080, Some(20)),
        listener("127.0.0.1", 8080, Some(21)),
    ];

    let error = resolve_forward_target(&listeners, 8080, Some("127.0.0.1")).unwrap_err();
    assert!(error.contains("TCP forwarding cannot select one process"));
}

#[test]
fn wildcard_and_specific_interface_overlap_is_rejected() {
    let listeners = [
        listener("0.0.0.0", 8080, Some(20)),
        listener("192.0.2.8", 8080, Some(21)),
    ];

    let error = resolve_forward_target(&listeners, 8080, Some("192.0.2.8")).unwrap_err();
    assert!(error.contains("overlapping listener"));
}

fn listener(address: &str, port: u16, pid: Option<u32>) -> PortListener {
    PortListener {
        bind_address: address.to_string(),
        port,
        pid,
        process: None,
        workspace_path: None,
        attribution: AttributionConfidence::Unavailable,
    }
}
