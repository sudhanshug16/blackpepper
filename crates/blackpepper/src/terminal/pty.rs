//! PTY spawning and session lifecycle.
//!
//! Handles the low-level portable-pty integration:
//! - Spawning shell processes in a pseudo-terminal
//! - Reading output via a background thread
//! - Writing input bytes to the PTY
//! - Resizing the terminal dimensions
//!
//! The session owns the PTY master, writer, and vt100 parser.
//! Output is sent to the app via AppEvent::PtyOutput.

use std::io::{self, Read, Write};
use std::path::Path;
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::mpsc::Sender;
use std::thread;

use arboard::Clipboard;
use base64::{engine::general_purpose::STANDARD, Engine as _};
#[cfg(not(target_os = "macos"))]
use notify_rust::Notification;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use vt100::Parser;

use crate::events::AppEvent;

use super::input_modes::InputModes;
use super::render::render_lines;

/// A running terminal session backed by a PTY.
///
/// Each session has a unique ID used to route output events back to the
/// correct tab. The session manages its own vt100 parser for screen state.
pub struct TerminalSession {
    id: u64,
    workspace_name: String,
    parser: Parser,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    _child: Box<dyn portable_pty::Child + Send + Sync>,
    rows: u16,
    cols: u16,
    default_fg: (u8, u8, u8),
    default_bg: (u8, u8, u8),
    osc_pending: Vec<u8>,
}

impl TerminalSession {
    /// Spawn a new terminal session running the given shell command.
    ///
    /// Starts a background thread that reads PTY output and sends it
    /// via the provided channel. Returns an error if PTY creation fails.
    pub fn spawn(
        id: u64,
        workspace_name: &str,
        shell: &str,
        args: &[String],
        cwd: &Path,
        rows: u16,
        cols: u16,
        default_fg: (u8, u8, u8),
        default_bg: (u8, u8, u8),
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

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|err| err.to_string())?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|err| err.to_string())?;
        let writer = pair.master.take_writer().map_err(|err| err.to_string())?;

        // Background thread reads PTY output and forwards to app event loop
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
            workspace_name: workspace_name.to_string(),
            parser: Parser::new(rows.max(1), cols.max(1), 1000),
            writer,
            master: pair.master,
            _child: child,
            rows: rows.max(1),
            cols: cols.max(1),
            default_fg,
            default_bg,
            osc_pending: Vec::new(),
        })
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    /// Process bytes received from PTY output.
    pub fn process_bytes(&mut self, bytes: &[u8]) {
        self.respond_to_osc(bytes);
        self.parser.process(bytes);
    }

    /// Resize the terminal and notify the PTY.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        if rows == self.rows && cols == self.cols {
            return;
        }
        self.rows = rows;
        self.cols = cols;
        self.parser.screen_mut().set_size(rows, cols);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    /// Write input bytes to the PTY.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if self.writer.write_all(bytes).is_ok() {
            let _ = self.writer.flush();
        }
    }

    /// Render visible lines for display.
    pub fn render_lines(&self, rows: u16, cols: u16) -> Vec<ratatui::text::Line<'static>> {
        render_lines(&self.parser, rows, cols)
    }

    /// Current terminal input modes as requested by the session.
    pub fn input_modes(&self) -> InputModes {
        InputModes::from_screen(self.parser.screen())
    }

    fn respond_to_osc(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }

        if !self.osc_pending.is_empty() {
            let mut combined = Vec::with_capacity(self.osc_pending.len() + bytes.len());
            combined.extend_from_slice(&self.osc_pending);
            combined.extend_from_slice(bytes);
            self.osc_pending.clear();
            self.scan_osc_queries(&combined);
            return;
        }

        self.scan_osc_queries(bytes);
    }

    fn scan_osc_queries(&mut self, bytes: &[u8]) {
        let mut idx = 0;
        while idx < bytes.len() {
            if bytes[idx] == 0x1b && bytes.get(idx + 1) == Some(&b']') {
                let start = idx;
                idx += 2;
                let mut end = None;
                while idx < bytes.len() {
                    if bytes[idx] == 0x07 {
                        end = Some((idx + 1, 1));
                        break;
                    }
                    if bytes[idx] == 0x1b && bytes.get(idx + 1) == Some(&b'\\') {
                        end = Some((idx + 2, 2));
                        break;
                    }
                    idx += 1;
                }
                if let Some((end_idx, terminator_len)) = end {
                    let body_end = end_idx.saturating_sub(terminator_len);
                    if body_end > start + 2 {
                        self.maybe_reply_to_osc(&bytes[start + 2..body_end]);
                    }
                    idx = end_idx;
                } else {
                    self.osc_pending.extend_from_slice(&bytes[start..]);
                    if self.osc_pending.len() > 1024 {
                        self.osc_pending.clear();
                    }
                    return;
                }
            } else {
                if bytes[idx] == 0x07 {
                    write_to_stdout(&[0x07]);
                }
                idx += 1;
            }
        }
    }

    fn maybe_reply_to_osc(&mut self, body: &[u8]) {
        if body.starts_with(b"10;?") {
            self.write_bytes(&osc_color_response(10, self.default_fg));
            return;
        }
        if body.starts_with(b"11;?") {
            self.write_bytes(&osc_color_response(11, self.default_bg));
            return;
        }
        if body.starts_with(b"0;") || body.starts_with(b"2;") {
            write_to_stdout(b"\x1b]");
            write_to_stdout(body);
            write_to_stdout(b"\x07");
            return;
        }
        if let Some(message) = body
            .strip_prefix(b"9;")
            .or_else(|| body.strip_prefix(b"777;"))
        {
            let text = String::from_utf8_lossy(message).trim().to_string();
            if !text.is_empty() {
                let summary = format!("[bp] {}", self.workspace_name);
                send_notification(&summary, &text);
            }
            return;
        }
        if let Some(rest) = body.strip_prefix(b"52;") {
            let mut parts = rest.splitn(2, |byte| *byte == b';');
            let target = parts.next().unwrap_or_default();
            let payload = parts.next().unwrap_or_default();
            if payload == b"?" {
                if let Ok(mut clipboard) = Clipboard::new() {
                    if let Ok(text) = clipboard.get_text() {
                        let target = if target.is_empty() {
                            b"c".as_ref()
                        } else {
                            target
                        };
                        let encoded = STANDARD.encode(text.as_bytes());
                        let response = format!(
                            "\x1b]52;{};{}\x07",
                            String::from_utf8_lossy(target),
                            encoded
                        );
                        self.write_bytes(response.as_bytes());
                    }
                }
            } else if let Ok(decoded) = STANDARD.decode(payload) {
                let text = String::from_utf8_lossy(&decoded).to_string();
                if let Ok(mut clipboard) = Clipboard::new() {
                    let _ = clipboard.set_text(text);
                }
            }
        }
    }
}

fn write_to_stdout(bytes: &[u8]) {
    let mut stdout = io::stdout();
    if stdout.write_all(bytes).is_ok() {
        let _ = stdout.flush();
    }
}

#[cfg(target_os = "macos")]
fn send_notification(summary: &str, body: &str) {
    let summary = escape_osascript(summary);
    let body = escape_osascript(body);
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        body, summary
    );
    let _ = Command::new("osascript").arg("-e").arg(script).status();
}

#[cfg(target_os = "macos")]
fn escape_osascript(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

#[cfg(not(target_os = "macos"))]
fn send_notification(summary: &str, body: &str) {
    let _ = Notification::new().summary(summary).body(body).show();
}

fn osc_color_response(kind: u8, rgb: (u8, u8, u8)) -> Vec<u8> {
    let to_16 = |value: u8| u16::from(value) * 257;
    format!(
        "\x1b]{kind};rgb:{:04x}/{:04x}/{:04x}\x07",
        to_16(rgb.0),
        to_16(rgb.1),
        to_16(rgb.2)
    )
    .into_bytes()
}
