//! Best-effort ownership of process-wide terminal mutations.

use crossterm::cursor::Hide;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen};
use crossterm::ExecutableCommand;
#[cfg(unix)]
use std::fs::OpenOptions;
use std::io::{self, Write};

// Disable every input mode Blackpepper or an embedded terminal can leave on.
// Repeating these terminal operations is safe, which also makes recovery from
// a partially written escape sequence conservative.
const CONSERVATIVE_INPUT_RESET: &[u8] = b"\x1b[0m\x1b>\x1b[?1l\x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1004l\x1b[?1005l\x1b[?1006l\x1b[?1007l\x1b[?1015l\x1b[?1016l\x1b[?2004l";
const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";
const SHOW_CURSOR: &[u8] = b"\x1b[?25h";

#[derive(Default)]
pub(super) struct TerminalSessionGuard {
    input_modes_armed: bool,
    raw_mode_armed: bool,
    alternate_screen_armed: bool,
    cursor_armed: bool,
    flush_armed: bool,
}

impl TerminalSessionGuard {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn enable_raw_mode(&mut self) -> io::Result<()> {
        self.enable_raw_mode_with(enable_raw_mode)
    }

    fn enable_raw_mode_with(&mut self, enable: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
        // Arm first because a failed terminal operation may have applied only
        // part of its mutation.
        self.input_modes_armed = true;
        self.raw_mode_armed = true;
        self.flush_armed = true;
        enable()
    }

    pub(super) fn enter_alternate_screen(&mut self, writer: &mut impl Write) -> io::Result<()> {
        self.alternate_screen_armed = true;
        self.flush_armed = true;
        writer.execute(EnterAlternateScreen).map(|_| ())
    }

    pub(super) fn hide_cursor(&mut self, writer: &mut impl Write) -> io::Result<()> {
        self.cursor_armed = true;
        self.flush_armed = true;
        writer.execute(Hide).map(|_| ())
    }

    /// Restores the terminal and retries unfinished output through its
    /// controlling TTY when stdout has already disappeared.
    pub(super) fn restore(&mut self, writer: &mut impl Write) -> io::Result<()> {
        let primary = self.restore_with(writer, disable_raw_mode);
        if primary.is_ok() || !self.has_pending_cleanup() {
            return primary;
        }

        #[cfg(unix)]
        if let Ok(mut tty) = OpenOptions::new().write(true).open("/dev/tty") {
            if self.restore_with(&mut tty, disable_raw_mode).is_ok() {
                return Ok(());
            }
        }

        primary
    }

    fn restore_with(
        &mut self,
        writer: &mut impl Write,
        mut disable_raw: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        if !self.has_pending_cleanup() {
            return Ok(());
        }

        let mut first_error = None;
        let mut wrote_input_modes = false;
        let mut wrote_alternate_screen = false;
        let mut wrote_cursor = false;
        if self.input_modes_armed {
            match writer.write_all(CONSERVATIVE_INPUT_RESET) {
                Ok(()) => wrote_input_modes = true,
                Err(error) => remember_first(&mut first_error, error),
            }
        }
        if self.raw_mode_armed {
            match disable_raw() {
                Ok(()) => self.raw_mode_armed = false,
                Err(error) => remember_first(&mut first_error, error),
            }
        }
        if self.alternate_screen_armed {
            match writer.write_all(LEAVE_ALTERNATE_SCREEN) {
                Ok(()) => wrote_alternate_screen = true,
                Err(error) => remember_first(&mut first_error, error),
            }
        }
        if self.cursor_armed {
            match writer.write_all(SHOW_CURSOR) {
                Ok(()) => wrote_cursor = true,
                Err(error) => remember_first(&mut first_error, error),
            }
        }
        if self.flush_armed {
            match writer.flush() {
                Ok(()) => {
                    if wrote_input_modes {
                        self.input_modes_armed = false;
                    }
                    if wrote_alternate_screen {
                        self.alternate_screen_armed = false;
                    }
                    if wrote_cursor {
                        self.cursor_armed = false;
                    }
                    if !self.has_armed_output() {
                        self.flush_armed = false;
                    }
                }
                Err(error) => remember_first(&mut first_error, error),
            }
        }

        first_error.map_or(Ok(()), Err)
    }

    fn has_armed_mutation(&self) -> bool {
        self.has_armed_output() || self.raw_mode_armed
    }

    fn has_armed_output(&self) -> bool {
        self.input_modes_armed || self.alternate_screen_armed || self.cursor_armed
    }

    fn has_pending_cleanup(&self) -> bool {
        self.has_armed_mutation() || self.flush_armed
    }
}

impl Drop for TerminalSessionGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = self.restore(&mut stdout);
    }
}

fn remember_first(first_error: &mut Option<io::Error>, error: io::Error) {
    if first_error.is_none() {
        *first_error = Some(error);
    }
}

#[cfg(test)]
mod tests;
