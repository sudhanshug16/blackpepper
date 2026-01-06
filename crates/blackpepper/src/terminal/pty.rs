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

use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::Sender;
use std::thread;

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use vt100::{MouseProtocolEncoding, MouseProtocolMode, Parser};

use crate::events::AppEvent;

use super::render::render_lines;

/// A running terminal session backed by a PTY.
///
/// Each session has a unique ID used to route output events back to the
/// correct tab. The session manages its own vt100 parser for screen state.
pub struct TerminalSession {
    id: u64,
    parser: Parser,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    _child: Box<dyn portable_pty::Child + Send + Sync>,
    rows: u16,
    cols: u16,
}

impl TerminalSession {
    /// Spawn a new terminal session running the given shell command.
    ///
    /// Starts a background thread that reads PTY output and sends it
    /// via the provided channel. Returns an error if PTY creation fails.
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

    /// Returns the current mouse protocol mode and encoding from vt100.
    pub fn mouse_protocol(&self) -> (MouseProtocolMode, MouseProtocolEncoding) {
        let screen = self.parser.screen();
        (
            screen.mouse_protocol_mode(),
            screen.mouse_protocol_encoding(),
        )
    }

    /// Returns true if the terminal is in alternate screen mode.
    pub fn alternate_screen(&self) -> bool {
        self.parser.screen().alternate_screen()
    }

    /// Process bytes received from PTY output.
    pub fn process_bytes(&mut self, bytes: &[u8]) {
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
        self.parser.set_size(rows, cols);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    /// Write input bytes to the PTY (keystrokes, mouse events).
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
}
