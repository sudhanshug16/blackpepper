use super::{buffer_text, workspace_state};
use crate::ports::{AttributionConfidence, PortListener, PortSnapshot, ProbeCompleteness};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

fn state_with_ports(count: u16) -> crate::client::ClientState {
    let mut state = workspace_state();
    let workspace = state.snapshot.workspaces[0].clone();
    state.ports.insert(
        workspace.host_id,
        PortSnapshot {
            listeners: (0..count)
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
    state
}

#[test]
fn port_rows_name_the_mouse_action_and_build_matching_targets() {
    let mut state = state_with_ports(2);
    let mut terminal = Terminal::new(TestBackend::new(30, 8)).unwrap();

    terminal
        .draw(|frame| super::super::ports::render_ports(&mut state, frame, Rect::new(0, 0, 30, 8)))
        .unwrap();

    let rendered = buffer_text(&terminal);
    assert!(rendered.contains("PORTS"));
    // Each listener is a right-aligned action row over a dim detail row.
    assert!(
        rendered.contains("4000") && rendered.contains("click to forward"),
        "missing port row in:\n{rendered}"
    );
    assert!(
        rendered.contains("service-0 · 127.0.0.1:4000"),
        "missing detail row in:\n{rendered}"
    );
    assert_eq!(state.port_click_targets.len(), 2);
    assert_eq!(state.port_click_targets[0].y, 1);
    assert_eq!(state.port_click_targets[0].x_start, 0);
    assert_eq!(state.port_click_targets[0].x_end, 30);
    // The detail row directly under a port is never its own hit target.
    assert_eq!(state.port_click_targets[1].y, 3);
}

#[test]
fn compact_ports_panel_scrolls_all_listeners_and_rebuilds_click_targets() {
    let mut state = state_with_ports(12);
    let mut terminal = Terminal::new(TestBackend::new(30, 8)).unwrap();

    terminal
        .draw(|frame| super::super::ports::render_ports(&mut state, frame, Rect::new(0, 0, 30, 8)))
        .unwrap();
    // Two rows per listener means seven visible rows cover four listeners
    // (the seventh row is a detail line, which is not clickable).
    assert_eq!(state.port_click_targets.len(), 4);
    assert_eq!(state.port_click_targets[0].target.remote_port, 4_000);
    assert_eq!(state.port_click_targets[3].target.remote_port, 4_003);

    state.ports_scroll = 6;
    terminal
        .draw(|frame| super::super::ports::render_ports(&mut state, frame, Rect::new(0, 0, 30, 8)))
        .unwrap();
    assert_eq!(state.port_click_targets.len(), 4);
    assert_eq!(state.port_click_targets[0].target.remote_port, 4_003);
    assert_eq!(state.port_click_targets[3].target.remote_port, 4_006);
}

#[test]
fn manage_footer_keeps_enter_bound_to_workspace_attach() {
    let state = state_with_ports(1);
    let hint = super::super::footer::default_footer_hint(&state);
    assert!(hint.contains("enter attach"));
    assert!(!hint.contains("enter forward"));
}
