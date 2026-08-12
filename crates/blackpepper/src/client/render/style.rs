use crate::client::{ClientState, DisplayStatus, HostConnection};
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders};

pub(super) fn ui_style(state: &ClientState) -> Style {
    Style::default()
        .fg(Color::Rgb(
            state.config.ui.foreground.0,
            state.config.ui.foreground.1,
            state.config.ui.foreground.2,
        ))
        .bg(Color::Rgb(
            state.config.ui.background.0,
            state.config.ui.background.1,
            state.config.ui.background.2,
        ))
}

/// Management surfaces sit just above the configured terminal background.
/// Deriving the shade from the user's colors preserves custom light and dark
/// themes instead of imposing another hard-coded palette.
pub(super) fn panel_style(state: &ClientState) -> Style {
    ui_style(state).bg(blend_color(
        state.config.ui.background,
        state.config.ui.foreground,
        6,
    ))
}

pub(super) fn panel_block(state: &ClientState) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(panel_style(state))
        .border_style(Style::default().fg(blend_color(
            state.config.ui.background,
            state.config.ui.foreground,
            26,
        )))
}

pub(super) fn selected_style(state: &ClientState) -> Style {
    Style::default()
        .fg(Color::Rgb(
            state.config.ui.foreground.0,
            state.config.ui.foreground.1,
            state.config.ui.foreground.2,
        ))
        .bg(blend_color(
            state.config.ui.background,
            state.config.ui.foreground,
            16,
        ))
}

fn blend_color(base: (u8, u8, u8), toward: (u8, u8, u8), percent: u16) -> Color {
    let blend = |base: u8, toward: u8| {
        ((u16::from(base) * (100 - percent) + u16::from(toward) * percent) / 100) as u8
    };
    Color::Rgb(
        blend(base.0, toward.0),
        blend(base.1, toward.1),
        blend(base.2, toward.2),
    )
}

pub(super) fn status_span(status: DisplayStatus) -> Span<'static> {
    let (label, color) = match status {
        DisplayStatus::Idle => ("", Color::Reset),
        DisplayStatus::Unknown => ("?", Color::DarkGray),
        DisplayStatus::Ready => ("ready", Color::DarkGray),
        DisplayStatus::Working => ("working", Color::Cyan),
        DisplayStatus::Done => ("done", Color::Green),
        DisplayStatus::NeedsInput => ("input", Color::Yellow),
        DisplayStatus::Exited => ("exited", Color::Red),
    };
    Span::styled(label.to_string(), Style::default().fg(color))
}

pub(super) fn connection_symbol(connection: HostConnection) -> &'static str {
    match connection {
        HostConnection::Local | HostConnection::Connected => "●",
        HostConnection::Authenticating | HostConnection::Reconnecting => "◐",
        HostConnection::NeedsAuthentication => "◆",
        HostConnection::HostKeyBlocked | HostConnection::Failed => "!",
        HostConnection::Disconnected => "○",
    }
}

pub(super) fn connection_style(connection: HostConnection) -> Style {
    let color = match connection {
        HostConnection::Local | HostConnection::Connected => Color::Green,
        HostConnection::Authenticating | HostConnection::Reconnecting => Color::Yellow,
        HostConnection::NeedsAuthentication => Color::Yellow,
        HostConnection::HostKeyBlocked | HostConnection::Failed => Color::Red,
        HostConnection::Disconnected => Color::DarkGray,
    };
    Style::default().fg(color)
}

pub(super) fn connected_client_label(count: Option<usize>) -> String {
    match count {
        Some(1) => " · 1 client".to_owned(),
        Some(count) => format!(" · {count} clients"),
        None => String::new(),
    }
}
