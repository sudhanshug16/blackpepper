use super::footer::{default_footer_hint, work_attention};
use super::ports::render_ports;
use super::style::connected_client_label;
use crate::client::{ClientMode, ClientState, DisplayStatus, EmbeddedTerminal};
use crate::core::{HostRecord, HostTransport, RegistrySnapshot, WorkspaceRecord};
use crate::ports::{
    AttributionConfidence, PortListener, PortSnapshot, ProbeCompleteness, RemotePortTarget,
};
use crate::transport::{ProcessSpec, PtyProcess};
use portable_pty::PtySize;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

fn attached_state() -> ClientState {
    let (event_tx, _event_rx) = std::sync::mpsc::channel();
    let host = HostRecord::new("local", HostTransport::Local);
    let workspace = WorkspaceRecord::new(host.id, "/workspace");
    let workspace_id = workspace.id;
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        RegistrySnapshot {
            hosts: vec![host],
            workspaces: vec![workspace],
            ..RegistrySnapshot::default()
        },
        event_tx.clone(),
    );
    let process = PtyProcess::spawn(
        &ProcessSpec::new("/bin/sh").args(["-c", "exec sleep 30"]),
        PtySize::default(),
    )
    .unwrap();
    let terminal = EmbeddedTerminal::new(
        workspace_id,
        process,
        24,
        80,
        state.config.ui.foreground,
        state.config.ui.background,
        event_tx,
    )
    .unwrap();
    state.terminals.insert(workspace_id, terminal);
    state.active_workspace = Some(workspace_id);
    state.selected_workspace = Some(workspace_id);
    state.mode = ClientMode::Work;
    state
}

fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .fold(String::new(), |mut text, cell| {
            text.push_str(cell.symbol());
            text
        })
}

#[test]
fn connected_client_label_is_live_and_grammatical() {
    assert_eq!(connected_client_label(None), "");
    assert_eq!(connected_client_label(Some(0)), " · 0 clients");
    assert_eq!(connected_client_label(Some(1)), " · 1 client");
    assert_eq!(connected_client_label(Some(2)), " · 2 clients");
}

#[test]
fn footer_explains_controls_for_the_current_mode() {
    let (event_tx, _event_rx) = std::sync::mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        RegistrySnapshot::default(),
        event_tx,
    );

    assert!(default_footer_hint(&state).contains(": commands"));
    state.mode = ClientMode::Work;
    assert!(default_footer_hint(&state).contains("ctrl+] Manage"));
    state.mode = ClientMode::Authenticate;
    assert!(default_footer_hint(&state).contains("SSH prompt"));
}

#[test]
fn work_mode_is_a_full_width_terminal_canvas_with_one_status_row() {
    let mut state = attached_state();
    state.ports_area = Some(Rect::new(10, 4, 20, 8));
    state
        .port_click_targets
        .push(crate::client::state::PortClickTarget {
            workspace_id: state.active_workspace.unwrap(),
            target: RemotePortTarget {
                remote_host: "127.0.0.1".to_owned(),
                remote_port: 4_000,
            },
            x_start: 10,
            x_end: 20,
            y: 5,
        });
    let mut terminal = Terminal::new(TestBackend::new(180, 52)).unwrap();

    terminal
        .draw(|frame| super::render(&mut state, frame))
        .unwrap();

    assert_eq!(state.terminal_area, Some(Rect::new(0, 0, 180, 51)));
    assert!(state.ports_area.is_none());
    assert!(state.port_click_targets.is_empty());
    let rendered = buffer_text(&terminal);
    assert!(!rendered.contains("Hosts / Workspaces"));
    assert!(!rendered.contains(" Ports "));
    assert!(!rendered.contains(" Zellij "));
    assert!(rendered.contains(" WORK "));
    assert!(rendered.contains("ctrl+] Manage"));
}

#[test]
fn work_layout_has_no_width_breakpoint_at_100_columns() {
    for width in [99, 100] {
        let mut state = attached_state();
        let mut terminal = Terminal::new(TestBackend::new(width, 20)).unwrap();

        terminal
            .draw(|frame| super::render(&mut state, frame))
            .unwrap();

        assert_eq!(state.terminal_area, Some(Rect::new(0, 0, width, 19)));
    }
}

#[test]
fn manage_mode_keeps_the_workspace_and_ports_dashboard() {
    let mut state = attached_state();
    state.mode = ClientMode::Manage;
    let mut terminal = Terminal::new(TestBackend::new(180, 52)).unwrap();

    terminal
        .draw(|frame| super::render(&mut state, frame))
        .unwrap();

    let rendered = buffer_text(&terminal);
    assert!(rendered.contains("Hosts / Workspaces"));
    assert!(rendered.contains(" Ports "));
    assert!(rendered.contains(" Zellij "));
    assert!(rendered.contains(" MANAGE "));
    let sidebar_border = terminal.backend().buffer().cell((0, 0)).unwrap();
    let sidebar_surface = terminal.backend().buffer().cell((1, 1)).unwrap();
    assert_eq!(sidebar_border.fg, ratatui::style::Color::Rgb(104, 104, 104));
    assert_eq!(sidebar_border.bg, ratatui::style::Color::Rgb(63, 63, 63));
    assert_eq!(sidebar_surface.bg, ratatui::style::Color::Rgb(63, 63, 63));
}

#[test]
fn work_footer_preserves_attention_hidden_with_the_dashboard() {
    let mut state = attached_state();
    let workspace_id = state.active_workspace.unwrap();
    let host_id = state.host_for_workspace(workspace_id).unwrap();
    state
        .statuses
        .insert(workspace_id, DisplayStatus::NeedsInput);
    state.connected_clients.insert(workspace_id, 2);
    state.ports.insert(
        host_id,
        PortSnapshot {
            listeners: Vec::new(),
            completeness: ProbeCompleteness::Partial,
            warning: Some("some listeners could not be attributed".to_owned()),
        },
    );

    assert_eq!(
        work_attention(&state),
        ["input needed", "2 clients", "ports"]
    );
}

#[test]
fn compact_ports_panel_scrolls_every_listener_and_rebuilds_click_targets() {
    let (event_tx, _event_rx) = std::sync::mpsc::channel();
    let host = HostRecord::new("local", HostTransport::Local);
    let workspace = WorkspaceRecord::new(host.id, "/workspace");
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        RegistrySnapshot {
            hosts: vec![host.clone()],
            workspaces: vec![workspace.clone()],
            ..RegistrySnapshot::default()
        },
        event_tx,
    );
    state.ports.insert(
        host.id,
        PortSnapshot {
            listeners: (0..12)
                .map(|offset| PortListener {
                    bind_address: "127.0.0.1".to_owned(),
                    port: 4_000 + offset,
                    pid: Some(u32::from(offset) + 1),
                    process: Some(format!("service-{offset}")),
                    workspace_path: Some(workspace.root_path.clone().into()),
                    attribution: AttributionConfidence::ExactCwd,
                })
                .collect(),
            completeness: ProbeCompleteness::Full,
            warning: None,
        },
    );
    let mut terminal = Terminal::new(TestBackend::new(30, 8)).unwrap();

    terminal
        .draw(|frame| render_ports(&mut state, frame, Rect::new(0, 0, 30, 8)))
        .unwrap();
    assert_eq!(state.port_click_targets.len(), 6);
    assert_eq!(state.port_click_targets[0].target.remote_port, 4_000);
    assert_eq!(state.port_click_targets[5].target.remote_port, 4_005);

    state.ports_scroll = 6;
    terminal
        .draw(|frame| render_ports(&mut state, frame, Rect::new(0, 0, 30, 8)))
        .unwrap();
    assert_eq!(state.port_click_targets.len(), 6);
    assert_eq!(state.port_click_targets[0].target.remote_port, 4_006);
    assert_eq!(state.port_click_targets[5].target.remote_port, 4_011);
}
