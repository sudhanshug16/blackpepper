mod footer;
mod ports;
mod sidebar;
mod style;
mod terminal;

use super::{ClientMode, ClientState};
use footer::render_footer;
use ports::render_ports;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::Block;
use sidebar::render_sidebar;
use style::ui_style;
use terminal::render_terminal;

pub fn render(state: &mut ClientState, frame: &mut ratatui::Frame) {
    state.expire_transient_output();
    frame.render_widget(Block::default().style(ui_style(state)), frame.area());
    if state.mode == ClientMode::Work {
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(frame.area());
        // Management controls have no invisible hit targets while Zellij owns
        // the canvas. The footer remains outside the PTY mouse coordinates.
        state.ports_area = None;
        state.port_click_targets.clear();
        render_terminal(state, frame, outer[0]);
        render_footer(state, frame, outer[1]);
        return;
    }
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(frame.area());
    let content = if outer[0].width >= 100 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(30),
                Constraint::Min(40),
                Constraint::Length(26),
            ])
            .split(outer[0])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(26), Constraint::Min(30)])
            .split(outer[0])
    };

    render_sidebar(state, frame, content[0]);
    if content.len() > 2 {
        render_terminal(state, frame, content[1]);
        render_ports(state, frame, content[2]);
    } else if state.mode == ClientMode::Manage && content[1].height >= 18 {
        let main = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(9), Constraint::Length(8)])
            .split(content[1]);
        render_terminal(state, frame, main[0]);
        render_ports(state, frame, main[1]);
    } else {
        render_terminal(state, frame, content[1]);
        state.ports_area = None;
        state.port_click_targets.clear();
    }
    render_footer(state, frame, outer[1]);
}

#[cfg(test)]
mod tests;
