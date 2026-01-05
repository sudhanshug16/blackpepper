use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::Sender;
use std::thread;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use vt100::{Color as VtColor, MouseProtocolEncoding, MouseProtocolMode, Parser};

use crate::events::AppEvent;

pub struct TerminalSession {
    id: u64,
    parser: Parser,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    _child: Box<dyn portable_pty::Child + Send + Sync>,
    rows: u16,
    cols: u16,
}

pub struct RenderOverlay<'a> {
    pub selection: Option<&'a [Vec<(u16, u16)>]>,
    pub search: Option<&'a [Vec<(u16, u16)>]>,
    pub active_search: Option<(u16, u16, u16)>,
}

impl TerminalSession {
    pub fn spawn(
        id: u64,
        shell: &str,
        args: &[String],
        cwd: &Path,
        rows: u16,
        cols: u16,
        output_tx: Sender<AppEvent>,
    ) -> Result<Self, String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| err.to_string())?;

        let mut cmd = CommandBuilder::new(shell);
        if !args.is_empty() {
            cmd.args(args.iter().map(|arg| arg.as_str()));
        }
        cmd.cwd(cwd);

        let child = pair.slave.spawn_command(cmd).map_err(|err| err.to_string())?;
        let mut reader = pair.master.try_clone_reader().map_err(|err| err.to_string())?;
        let writer = pair.master.take_writer().map_err(|err| err.to_string())?;

        thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(size) => {
                        if output_tx
                            .send(AppEvent::PtyOutput(id, buffer[..size].to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = output_tx.send(AppEvent::PtyExit(id));
        });

        Ok(Self {
            id,
            parser: Parser::new(rows.max(1), cols.max(1), 1000),
            writer,
            master: pair.master,
            _child: child,
            rows: rows.max(1),
            cols: cols.max(1),
        })
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn mouse_protocol(&self) -> (MouseProtocolMode, MouseProtocolEncoding) {
        let screen = self.parser.screen();
        (screen.mouse_protocol_mode(), screen.mouse_protocol_encoding())
    }

    pub fn title(&self) -> &str {
        self.parser.screen().title()
    }

    pub fn alternate_screen(&self) -> bool {
        self.parser.screen().alternate_screen()
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn scrollback(&self) -> usize {
        self.parser.screen().scrollback()
    }

    pub fn scroll_lines(&mut self, delta: isize) {
        if delta == 0 {
            return;
        }
        let max_offset = self.max_scrollback_offset();
        let current = self.parser.screen().scrollback() as isize;
        let next = (current + delta).max(0).min(max_offset as isize) as usize;
        self.parser.set_scrollback(next);
    }

    pub fn scroll_to_top(&mut self) {
        let max_offset = self.max_scrollback_offset();
        self.parser.set_scrollback(max_offset);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.parser.set_scrollback(0);
    }

    fn max_scrollback_offset(&self) -> usize {
        self.rows as usize
    }

    pub fn visible_rows_text(&self, rows: u16, cols: u16) -> Vec<String> {
        self.parser
            .screen()
            .rows(0, cols.max(1))
            .take(rows.max(1) as usize)
            .collect()
    }

    pub fn contents_between(
        &self,
        start_row: u16,
        start_col: u16,
        end_row: u16,
        end_col: u16,
    ) -> String {
        self.parser
            .screen()
            .contents_between(start_row, start_col, end_row, end_col)
    }

    pub fn scrollback_contents(&mut self) -> String {
        let current = self.parser.screen().scrollback();
        self.parser.set_scrollback(usize::MAX);
        let contents = self.parser.screen().contents();
        self.parser.set_scrollback(current);
        contents
    }

    pub fn process_bytes(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        if rows == self.rows && cols == self.cols {
            return;
        }
        self.rows = rows;
        self.cols = cols;
        self.parser.set_size(rows, cols);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if self.writer.write_all(bytes).is_ok() {
            let _ = self.writer.flush();
        }
    }

    pub fn render_lines_with_overlay(
        &self,
        rows: u16,
        cols: u16,
        overlay: RenderOverlay<'_>,
    ) -> Vec<Line<'static>> {
        let rows = rows.max(1);
        let cols = cols.max(1);
        let screen = self.parser.screen();
        let (cursor_row, cursor_col) = screen.cursor_position();
        let show_cursor = !screen.hide_cursor() && screen.scrollback() == 0;

        let mut lines = Vec::with_capacity(rows as usize);
        for row in 0..rows {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut current_text = String::new();
            let mut current_style = Style::default();
            let mut has_style = false;

            for col in 0..cols {
                let cell = match screen.cell(row, col) {
                    Some(cell) => cell,
                    None => {
                        push_span(
                            &mut spans,
                            &mut current_text,
                            &mut current_style,
                            &mut has_style,
                            Style::default(),
                            " ".to_string(),
                        );
                        continue;
                    }
                };

                if cell.is_wide_continuation() {
                    continue;
                }

                let mut style = style_for_cell(cell);
                if in_ranges(overlay.selection, row, col) {
                    style = style.bg(Color::White).fg(Color::Black);
                } else if in_active_search(overlay.active_search, row, col) {
                    style = style.bg(Color::Yellow).fg(Color::Black);
                } else if in_ranges(overlay.search, row, col) {
                    style = style.bg(Color::Blue).fg(Color::White);
                }
                if show_cursor && row == cursor_row && col == cursor_col {
                    style = style.add_modifier(Modifier::REVERSED);
                }

                let content = if cell.has_contents() {
                    cell.contents()
                } else {
                    " ".to_string()
                };

                push_span(
                    &mut spans,
                    &mut current_text,
                    &mut current_style,
                    &mut has_style,
                    style,
                    content,
                );
            }

            if has_style {
                spans.push(Span::styled(current_text, current_style));
            } else {
                spans.push(Span::raw(String::new()));
            }

            lines.push(Line::from(spans));
        }

        lines
    }
}

fn in_ranges(ranges: Option<&[Vec<(u16, u16)>]>, row: u16, col: u16) -> bool {
    let Some(ranges) = ranges else {
        return false;
    };
    let Some(row_ranges) = ranges.get(row as usize) else {
        return false;
    };
    row_ranges
        .iter()
        .any(|(start, end)| col >= *start && col < *end)
}

fn in_active_search(active: Option<(u16, u16, u16)>, row: u16, col: u16) -> bool {
    let Some((active_row, start, end)) = active else {
        return false;
    };
    row == active_row && col >= start && col < end
}

pub fn key_event_to_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    match key.code {
        KeyCode::Char(ch) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                let lowercase = ch.to_ascii_lowercase();
                if lowercase >= 'a' && lowercase <= 'z' {
                    let code = (lowercase as u8 - b'a') + 1;
                    return Some(vec![code]);
                }
            }
            let mut buffer = [0u8; 4];
            let encoded = ch.encode_utf8(&mut buffer);
            Some(encoded.as_bytes().to_vec())
        }
        KeyCode::Enter => Some(vec![b'\r']),
        KeyCode::Tab => Some(vec![b'\t']),
        KeyCode::Backspace => Some(vec![0x7f]),
        KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Up => Some(b"\x1b[A".to_vec()),
        KeyCode::Down => Some(b"\x1b[B".to_vec()),
        KeyCode::Right => Some(b"\x1b[C".to_vec()),
        KeyCode::Left => Some(b"\x1b[D".to_vec()),
        KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
        KeyCode::Home => Some(b"\x1b[H".to_vec()),
        KeyCode::End => Some(b"\x1b[F".to_vec()),
        KeyCode::PageUp => Some(b"\x1b[5~".to_vec()),
        KeyCode::PageDown => Some(b"\x1b[6~".to_vec()),
        _ => None,
    }
}

pub fn mouse_event_to_bytes(
    event: MouseEvent,
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    if mode == MouseProtocolMode::None {
        return None;
    }

    let allow_event = match event.kind {
        MouseEventKind::ScrollUp
        | MouseEventKind::ScrollDown
        | MouseEventKind::ScrollLeft
        | MouseEventKind::ScrollRight => true,
        MouseEventKind::Down(_) => true,
        MouseEventKind::Up(_) => matches!(
            mode,
            MouseProtocolMode::PressRelease
                | MouseProtocolMode::ButtonMotion
                | MouseProtocolMode::AnyMotion
        ),
        MouseEventKind::Drag(_) => matches!(
            mode,
            MouseProtocolMode::PressRelease
                | MouseProtocolMode::ButtonMotion
                | MouseProtocolMode::AnyMotion
        ),
        MouseEventKind::Moved => mode == MouseProtocolMode::AnyMotion,
    };

    if !allow_event {
        return None;
    }

    let mut code: u8 = match event.kind {
        MouseEventKind::ScrollUp => 64,
        MouseEventKind::ScrollDown => 65,
        MouseEventKind::ScrollLeft => 66,
        MouseEventKind::ScrollRight => 67,
        MouseEventKind::Down(button) => match button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
        },
        MouseEventKind::Up(button) => {
            if encoding == MouseProtocolEncoding::Sgr {
                match button {
                    MouseButton::Left => 0,
                    MouseButton::Middle => 1,
                    MouseButton::Right => 2,
                }
            } else {
                3
            }
        }
        MouseEventKind::Drag(button) => match button {
            MouseButton::Left => 32,
            MouseButton::Middle => 33,
            MouseButton::Right => 34,
        },
        MouseEventKind::Moved => 35,
    };

    if event.modifiers.contains(KeyModifiers::SHIFT) {
        code += 4;
    }
    if event.modifiers.contains(KeyModifiers::ALT) {
        code += 8;
    }
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        code += 16;
    }

    let x = event.column.saturating_add(1);
    let y = event.row.saturating_add(1);

    match encoding {
        MouseProtocolEncoding::Sgr => {
            let suffix = match event.kind {
                MouseEventKind::Up(_) => 'm',
                _ => 'M',
            };
            Some(format!("\x1b[<{};{};{}{}", code, x, y, suffix).into_bytes())
        }
        MouseProtocolEncoding::Default | MouseProtocolEncoding::Utf8 => {
            let cb = code.saturating_add(32);
            let cx = (x as u8).saturating_add(32);
            let cy = (y as u8).saturating_add(32);
            Some(vec![0x1b, b'[', b'M', cb, cx, cy])
        }
    }
}

fn style_for_cell(cell: &vt100::Cell) -> Style {
    let mut style = Style::default();
    style = style.fg(map_color(cell.fgcolor()));
    style = style.bg(map_color(cell.bgcolor()));

    if cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if cell.inverse() {
        style = style.add_modifier(Modifier::REVERSED);
    }

    style
}

fn map_color(color: VtColor) -> Color {
    match color {
        VtColor::Default => Color::Reset,
        VtColor::Idx(idx) => Color::Indexed(idx),
        VtColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn push_span(
    spans: &mut Vec<Span<'static>>,
    current_text: &mut String,
    current_style: &mut Style,
    has_style: &mut bool,
    style: Style,
    content: String,
) {
    if !*has_style {
        *current_style = style;
        *has_style = true;
        current_text.push_str(&content);
        return;
    }

    if *current_style == style {
        current_text.push_str(&content);
        return;
    }

    spans.push(Span::styled(std::mem::take(current_text), *current_style));
    *current_style = style;
    current_text.push_str(&content);
}
