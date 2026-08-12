mod chord;
mod chrome;
mod command;
mod footer;
mod glyph;
mod header;
mod help;
mod picker;
mod ports;
mod sidebar;
mod style;
mod terminal;

use super::{ClientMode, ClientState};
use footer::render_footer;
use header::render_header;
use help::render_help;
use picker::render_picker;
use ports::render_ports;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::Block;
use sidebar::render_sidebar;
use style::ui_style;
use terminal::render_terminal;

const WIDE_COLUMNS: u16 = 32 + 40 + 30;
const MEDIUM_COLUMNS: u16 = 32 + 30;
const COMPACT_SELECTOR_ROWS: u16 = 6;
const FOCUSED_SELECTOR_ROWS: u16 = 4;
const PORTS_ROWS: u16 = 8;

pub fn render(state: &mut ClientState, frame: &mut ratatui::Frame) {
    state.expire_transient_output();
    frame.render_widget(Block::default().style(ui_style(state)), frame.area());
    if state.mode == ClientMode::Work {
        render_work(state, frame);
        return;
    }

    // The completion list grows upward from the status row, so the prompt
    // itself never moves while you type.
    let completion_rows = command::completion_rows(state).min(frame.area().height / 2);
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(completion_rows),
            Constraint::Length(1),
        ])
        .split(frame.area());
    render_header(state, frame, outer[0]);
    render_manage_body(state, frame, outer[1]);
    if completion_rows > 0 {
        command::render_completion(state, frame, outer[2]);
    }
    render_footer(state, frame, outer[3]);
}

fn render_work(state: &mut ClientState, frame: &mut ratatui::Frame) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());
    // Management controls have no invisible hit targets while Zellij owns the
    // canvas. The status row stays outside the PTY mouse coordinates.
    clear_ports(state);
    render_terminal(state, frame, outer[0]);
    render_footer(state, frame, outer[1]);
}

fn render_manage_body(state: &mut ClientState, frame: &mut ratatui::Frame, area: Rect) {
    // Help owns the whole body: it is a reference surface, and splitting it
    // across a column would put the notes column somewhere unreadable.
    if state.help.is_some() {
        clear_ports(state);
        state.terminal_area = None;
        render_help(state, frame, area);
        return;
    }
    let focused_view = state.mode == ClientMode::Authenticate
        || state.pending_approval.is_some()
        || state.detail.is_some();
    if area.width >= WIDE_COLUMNS {
        let content = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(32),
                Constraint::Min(40),
                Constraint::Length(30),
            ])
            .split(area);
        render_sidebar(state, frame, content[0]);
        if focused_view {
            render_terminal(state, frame, union_horizontal(content[1], content[2]));
            clear_ports(state);
        } else {
            render_terminal(state, frame, content[1]);
            render_ports(state, frame, content[2]);
        }
        if state.picker.is_some() {
            render_picker(state, frame, content[1]);
        }
        return;
    }

    if area.width >= MEDIUM_COLUMNS {
        let content = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(32), Constraint::Min(30)])
            .split(area);
        render_sidebar(state, frame, content[0]);
        if !focused_view && content[1].height >= PORTS_ROWS + 8 {
            let session = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(8), Constraint::Length(PORTS_ROWS)])
                .split(content[1]);
            render_terminal(state, frame, session[0]);
            render_ports(state, frame, session[1]);
        } else {
            render_terminal(state, frame, content[1]);
            clear_ports(state);
        }
        if state.picker.is_some() {
            render_picker(state, frame, content[1]);
        }
        return;
    }

    let selector_limit = if focused_view {
        FOCUSED_SELECTOR_ROWS
    } else {
        COMPACT_SELECTOR_ROWS
    };
    let selector_rows = area.height.saturating_sub(1).clamp(1, selector_limit);
    let content = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(selector_rows), Constraint::Min(1)])
        .split(area);
    render_sidebar(state, frame, content[0]);
    render_terminal(state, frame, content[1]);
    clear_ports(state);
    if state.picker.is_some() {
        render_picker(state, frame, content[1]);
    }
}

fn clear_ports(state: &mut ClientState) {
    state.ports_area = None;
    state.port_click_targets.clear();
}

fn union_horizontal(left: Rect, right: Rect) -> Rect {
    Rect::new(
        left.x,
        left.y,
        left.width.saturating_add(right.width),
        left.height,
    )
}

#[cfg(test)]
mod tests;
