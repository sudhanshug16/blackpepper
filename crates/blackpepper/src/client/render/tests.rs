mod colors;
mod layout;
mod ports;
mod views;

use crate::client::{ClientMode, ClientState};
use crate::client_config::ColorTier;
use crate::core::{
    HostRecord, HostTransport, RegistrySnapshot, RepositoryIdentity, WorkspaceRecord,
};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

pub(super) fn workspace_state() -> ClientState {
    let (event_tx, _event_rx) = std::sync::mpsc::channel();
    let host = HostRecord::new("local", HostTransport::Local);
    let mut workspace = WorkspaceRecord::new(host.id, "/workspace/blackpepper");
    workspace.display_name = Some("blackpepper".to_owned());
    workspace.repository =
        Some(RepositoryIdentity::remote("https://github.com/example/blackpepper.git").unwrap());
    let workspace_id = workspace.id;
    let mut config = crate::client_config::load_contents(None, None, None).unwrap();
    config.ui.color_tier = ColorTier::TrueColor;
    let mut state = ClientState::new(
        config,
        RegistrySnapshot {
            hosts: vec![host.clone()],
            workspaces: vec![workspace],
            ..RegistrySnapshot::default()
        },
        event_tx,
    );
    state
        .connections
        .insert(host.id, crate::client::HostConnection::Local);
    state.active_workspace = Some(workspace_id);
    state.selected_workspace = Some(workspace_id);
    state.mode = ClientMode::Manage;
    state.rebuild_tree();
    state
}

pub(super) fn empty_state() -> ClientState {
    let (event_tx, _event_rx) = std::sync::mpsc::channel();
    let mut config = crate::client_config::load_contents(None, None, None).unwrap();
    config.ui.color_tier = ColorTier::TrueColor;
    ClientState::new(config, RegistrySnapshot::default(), event_tx)
}

pub(super) fn draw(state: &mut ClientState, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| super::render(state, frame)).unwrap();
    terminal
}

pub(super) fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
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

pub(super) fn row_text(terminal: &Terminal<TestBackend>, row: u16) -> String {
    let width = terminal.backend().buffer().area.width;
    (0..width)
        .filter_map(|column| terminal.backend().buffer().cell((column, row)))
        .map(|cell| cell.symbol())
        .collect()
}
