use super::style::{panel_block, panel_style, ui_style};
use crate::client::{ClientMode, ClientState};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

pub(super) fn render_terminal(state: &mut ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let inner = if state.mode == ClientMode::Work {
        area
    } else {
        let block = panel_block(state).title(" Zellij ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    };
    if state.mode == ClientMode::Authenticate {
        state.terminal_area = None;
        let output = String::from_utf8_lossy(&state.authentication_output);
        frame.render_widget(
            Paragraph::new(output.into_owned())
                .block(Block::default().title(" SSH authentication "))
                .style(panel_style(state))
                .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    if state.mode == ClientMode::Manage {
        if let Some(pending) = &state.pending_approval {
            state.terminal_area = None;
            frame.render_widget(
                Paragraph::new(pending.review.clone())
                    .style(panel_style(state))
                    .block(
                        Block::default()
                            .title(" Approval review — ↑/↓ scroll · :approve run · Esc dismiss "),
                    )
                    .scroll((state.approval_scroll, 0))
                    .wrap(Wrap { trim: false }),
                inner,
            );
            return;
        }
        if let Some(detail) = &state.detail {
            state.terminal_area = None;
            frame.render_widget(
                Paragraph::new(detail.body.clone())
                    .style(panel_style(state))
                    .block(
                        Block::default()
                            .title(format!(" {} — ↑/↓ scroll · Esc close ", detail.title))
                            .borders(Borders::NONE),
                    )
                    .scroll((state.detail_scroll, 0))
                    .wrap(Wrap { trim: false }),
                inner,
            );
            return;
        }
    }
    let Some(active) = state.active_workspace else {
        state.terminal_area = None;
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    "Blackpepper",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Line::raw(""),
                Line::raw("Select a workspace and press Enter."),
                Line::raw("Use :help for host, worktree, agent, and port actions."),
            ])
            .style(panel_style(state))
            .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    };
    let Some(terminal) = state.terminals.get_mut(&active) else {
        state.terminal_area = None;
        frame.render_widget(
            Paragraph::new("Workspace is detached. Press Enter to attach.")
                .style(panel_style(state)),
            inner,
        );
        return;
    };
    state.terminal_area = Some(inner);
    let resize_error = terminal.resize(inner.height, inner.width).err();
    let lines = terminal.render(inner.height, inner.width);
    if let Some(error) = resize_error {
        state.set_output(format!(
            "The attached terminal could not be resized; input remains available: {error}"
        ));
    }
    frame.render_widget(Paragraph::new(lines).style(ui_style(state)), inner);
}
