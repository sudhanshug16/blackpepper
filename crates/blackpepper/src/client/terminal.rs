use crate::core::WorkspaceId;
use crate::terminal::{
    osc::{clipboard_write_sequence, ClipboardTarget, OscAction, OscProtocol},
    render::render_lines,
    InputModes,
};
use crate::transport::{PtyProcess, TransportError};
use portable_pty::PtySize;
use ratatui::layout::Rect;
use ratatui::text::Line;
use std::io::{self, Read, Write};
use std::sync::mpsc::Sender;
use std::thread;
use vt100::Parser;

use super::{mouse::MouseInputProtocol, ClientEvent};

pub struct EmbeddedTerminal {
    workspace_id: WorkspaceId,
    attachment_id: uuid::Uuid,
    parser: Parser,
    process: PtyProcess,
    rows: u16,
    cols: u16,
    osc: OscProtocol,
    terminal_queries: TerminalQueryProtocol,
    mouse_input: MouseInputProtocol,
    event_tx: Sender<ClientEvent>,
}

impl EmbeddedTerminal {
    pub fn new(
        workspace_id: WorkspaceId,
        mut process: PtyProcess,
        rows: u16,
        cols: u16,
        foreground: (u8, u8, u8),
        background: (u8, u8, u8),
        event_tx: Sender<ClientEvent>,
    ) -> Result<Self, TransportError> {
        let attachment_id = uuid::Uuid::new_v4();
        let mut reader = process.take_reader()?;
        let output_tx = event_tx.clone();
        thread::spawn(move || read_output(workspace_id, attachment_id, &mut reader, output_tx));
        Ok(Self {
            workspace_id,
            attachment_id,
            parser: Parser::new(rows.max(1), cols.max(1), 10_000),
            process,
            rows: rows.max(1),
            cols: cols.max(1),
            osc: OscProtocol::new(foreground, background),
            terminal_queries: TerminalQueryProtocol::default(),
            mouse_input: MouseInputProtocol::default(),
            event_tx,
        })
    }

    pub fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub fn attachment_id(&self) -> uuid::Uuid {
        self.attachment_id
    }

    pub fn process_bytes(&mut self, bytes: &[u8]) {
        for response in self.terminal_queries.process(bytes, self.rows, self.cols) {
            if let Err(error) = self.process.write_all(&response) {
                self.report_notice(format!(
                    "The embedded terminal could not answer a size query: {error}"
                ));
            }
        }
        for action in self.osc.process(bytes) {
            match action {
                OscAction::WriteToPty(response) => {
                    if let Err(error) = self.process.write_all(&response) {
                        self.report_notice(format!(
                            "The embedded terminal could not answer a terminal query: {error}"
                        ));
                    }
                }
                OscAction::SetClipboard { target, text } => {
                    if let Some(message) = dispatch_clipboard(
                        target,
                        &text,
                        set_native_clipboard,
                        write_outer_clipboard,
                    ) {
                        self.report_notice(message);
                    }
                }
            }
        }
        self.parser.process(bytes);
    }

    pub fn write(
        &mut self,
        bytes: &[u8],
        terminal_area: Option<Rect>,
    ) -> Result<(), TransportError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let translated = self.mouse_input.process(
            bytes,
            terminal_area,
            self.parser.screen().mouse_protocol_encoding(),
        );
        self.process.write_all(&translated)
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), TransportError> {
        let rows = rows.max(1);
        let cols = cols.max(1);
        if rows == self.rows && cols == self.cols {
            return Ok(());
        }
        self.process.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        self.rows = rows;
        self.cols = cols;
        self.parser.screen_mut().set_size(rows, cols);
        Ok(())
    }

    pub fn render(&self, rows: u16, cols: u16) -> Vec<Line<'static>> {
        render_lines(&self.parser, rows, cols)
    }

    pub fn input_modes(&self) -> InputModes {
        InputModes::from_screen(self.parser.screen())
    }

    fn report_notice(&self, message: String) {
        let _ = self.event_tx.send(ClientEvent::TerminalNotice(
            self.workspace_id,
            self.attachment_id,
            message,
        ));
    }
}

#[cfg(target_os = "linux")]
fn set_native_clipboard(target: ClipboardTarget, text: &str) -> Result<(), String> {
    use arboard::{LinuxClipboardKind, SetExtLinux};

    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| format!("could not open the system clipboard: {error}"))?;
    clipboard
        .set()
        .clipboard(match target {
            ClipboardTarget::System => LinuxClipboardKind::Clipboard,
            ClipboardTarget::Primary => LinuxClipboardKind::Primary,
        })
        .text(text.to_string())
        .map_err(|error| format!("could not write the system clipboard: {error}"))
}

#[cfg(not(target_os = "linux"))]
fn set_native_clipboard(_target: ClipboardTarget, text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| format!("could not open the system clipboard: {error}"))?;
    clipboard
        .set_text(text)
        .map_err(|error| format!("could not write the system clipboard: {error}"))
}

fn write_outer_clipboard(sequence: &[u8]) -> Result<(), String> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    output
        .write_all(sequence)
        .and_then(|()| output.flush())
        .map_err(|error| format!("could not write OSC 52 to the outer terminal: {error}"))
}

fn dispatch_clipboard(
    target: ClipboardTarget,
    text: &str,
    mut set_native: impl FnMut(ClipboardTarget, &str) -> Result<(), String>,
    mut write_outer: impl FnMut(&[u8]) -> Result<(), String>,
) -> Option<String> {
    let Some(sequence) = clipboard_write_sequence(target, text) else {
        return Some(
            "Clipboard copy was rejected because it exceeded Blackpepper's 1 MiB limit."
                .to_string(),
        );
    };
    let native = set_native(target, text);
    let outer = write_outer(&sequence);
    match (native, outer) {
        (Ok(()), _) => Some("Copied.".to_owned()),
        (Err(_), Ok(())) => Some("Copy sent to your terminal.".to_owned()),
        (Err(_), Err(_)) => Some("Copy failed.".to_owned()),
    }
}

/// Replies to terminal-window queries that an embedded application would
/// normally send directly to a hardware terminal emulator.
///
/// Zellij uses the PTY window size for its primary sizing path, but terminal
/// applications are also allowed to request the text-area size with CSI 18 t.
/// Blackpepper consumes the escape stream instead of forwarding it to an outer
/// emulator, so it must answer this query itself.
#[derive(Default)]
struct TerminalQueryProtocol {
    pending: Vec<u8>,
}

impl TerminalQueryProtocol {
    fn process(&mut self, bytes: &[u8], rows: u16, cols: u16) -> Vec<Vec<u8>> {
        const MAX_SEQUENCE: usize = 64;

        if bytes.is_empty() {
            return Vec::new();
        }
        let combined;
        let input = if self.pending.is_empty() {
            bytes
        } else {
            combined = [self.pending.as_slice(), bytes].concat();
            self.pending.clear();
            &combined
        };
        let mut replies = Vec::new();
        let mut cursor = 0;
        while cursor < input.len() {
            let Some(offset) = input[cursor..].iter().position(|byte| *byte == 0x1b) else {
                break;
            };
            let start = cursor + offset;
            if start + 1 >= input.len() {
                self.pending.extend_from_slice(&input[start..]);
                break;
            }
            if input[start + 1] != b'[' {
                cursor = start + 2;
                continue;
            }
            let mut end = start + 2;
            while end < input.len()
                && !(0x40..=0x7e).contains(&input[end])
                && end - start < MAX_SEQUENCE
            {
                end += 1;
            }
            if end >= input.len() {
                if input.len() - start <= MAX_SEQUENCE {
                    self.pending.extend_from_slice(&input[start..]);
                }
                break;
            }
            if end - start >= MAX_SEQUENCE {
                cursor = end;
                continue;
            }
            if input[end] == b't' && &input[start + 2..end] == b"18" {
                replies.push(format!("\x1b[8;{};{}t", rows.max(1), cols.max(1)).into_bytes());
            }
            cursor = end + 1;
        }
        replies
    }
}

fn read_output(
    workspace_id: WorkspaceId,
    attachment_id: uuid::Uuid,
    reader: &mut dyn Read,
    event_tx: Sender<ClientEvent>,
) {
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => {
                if event_tx
                    .send(ClientEvent::TerminalOutput(
                        workspace_id,
                        attachment_id,
                        buffer[..size].to_vec(),
                    ))
                    .is_err()
                {
                    return;
                }
            }
            Err(_) => break,
        }
    }
    let _ = event_tx.send(ClientEvent::TerminalExited(workspace_id, attachment_id));
}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;
