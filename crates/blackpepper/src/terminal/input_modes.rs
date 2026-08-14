//! Input mode tracking for the host terminal.
//!
//! Zellij toggles terminal input modes (mouse, application cursor, etc.) via
//! escape sequences. We mirror those modes onto the host terminal so the
//! attached Zellij client receives mouse events directly.

use vt100::{MouseProtocolEncoding, MouseProtocolMode, Screen};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputModes {
    pub application_keypad: bool,
    pub application_cursor: bool,
    pub bracketed_paste: bool,
    pub focus_reporting: bool,
    pub mouse_mode: MouseProtocolMode,
    pub mouse_encoding: MouseProtocolEncoding,
}

impl Default for InputModes {
    fn default() -> Self {
        Self {
            application_keypad: false,
            application_cursor: false,
            bracketed_paste: false,
            focus_reporting: false,
            mouse_mode: MouseProtocolMode::None,
            mouse_encoding: MouseProtocolEncoding::Default,
        }
    }
}

impl InputModes {
    pub fn manage_interface() -> Self {
        Self {
            bracketed_paste: true,
            mouse_mode: MouseProtocolMode::PressRelease,
            mouse_encoding: MouseProtocolEncoding::Sgr,
            ..Self::default()
        }
    }

    pub fn from_screen(screen: &Screen) -> Self {
        Self {
            application_keypad: screen.application_keypad(),
            application_cursor: screen.application_cursor(),
            bracketed_paste: screen.bracketed_paste(),
            focus_reporting: false,
            mouse_mode: screen.mouse_protocol_mode(),
            mouse_encoding: screen.mouse_protocol_encoding(),
        }
    }

    /// Keep one Blackpepper-owned status row clickable even when the embedded
    /// application is not currently asking for mouse reports. Mouse events in
    /// the PTY viewport are discarded in that case; if the application did ask
    /// for them, its exact mode and encoding remain unchanged.
    pub fn with_shell_pointer_capture(mut self) -> Self {
        if self.mouse_mode == MouseProtocolMode::None {
            self.mouse_mode = MouseProtocolMode::PressRelease;
            self.mouse_encoding = MouseProtocolEncoding::Sgr;
        }
        self
    }

    pub fn with_focus_reporting(mut self, enabled: bool) -> Self {
        self.focus_reporting = enabled;
        self
    }

    pub fn diff_bytes(&self, prev: &Self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write_diff(prev, &mut out);
        out
    }

    fn write_diff(&self, prev: &Self, out: &mut Vec<u8>) {
        if self.application_keypad != prev.application_keypad {
            if self.application_keypad {
                out.extend_from_slice(b"\x1b=");
            } else {
                out.extend_from_slice(b"\x1b>");
            }
        }

        if self.application_cursor != prev.application_cursor {
            if self.application_cursor {
                out.extend_from_slice(b"\x1b[?1h");
            } else {
                out.extend_from_slice(b"\x1b[?1l");
            }
        }

        if self.bracketed_paste != prev.bracketed_paste {
            if self.bracketed_paste {
                out.extend_from_slice(b"\x1b[?2004h");
            } else {
                out.extend_from_slice(b"\x1b[?2004l");
            }
        }

        // Outer focus reporting is owned for Blackpepper's full TUI lifetime
        // by TerminalSessionGuard. This child flag only decides which Zellij
        // clients receive synthetic CSI I/O; mode transitions must not toggle
        // the real terminal's report stream.

        write_mouse_mode_diff(self.mouse_mode, prev.mouse_mode, out);
        write_mouse_encoding_diff(self.mouse_encoding, prev.mouse_encoding, out);
    }
}

fn write_mouse_mode_diff(mode: MouseProtocolMode, prev: MouseProtocolMode, out: &mut Vec<u8>) {
    if mode == prev {
        return;
    }

    match prev {
        MouseProtocolMode::None => {}
        MouseProtocolMode::Press => out.extend_from_slice(b"\x1b[?9l"),
        MouseProtocolMode::PressRelease => out.extend_from_slice(b"\x1b[?1000l"),
        MouseProtocolMode::ButtonMotion => out.extend_from_slice(b"\x1b[?1002l"),
        MouseProtocolMode::AnyMotion => out.extend_from_slice(b"\x1b[?1003l"),
    }
    match mode {
        MouseProtocolMode::None => {}
        MouseProtocolMode::Press => out.extend_from_slice(b"\x1b[?9h"),
        MouseProtocolMode::PressRelease => out.extend_from_slice(b"\x1b[?1000h"),
        MouseProtocolMode::ButtonMotion => out.extend_from_slice(b"\x1b[?1002h"),
        MouseProtocolMode::AnyMotion => out.extend_from_slice(b"\x1b[?1003h"),
    }
}

fn write_mouse_encoding_diff(
    encoding: MouseProtocolEncoding,
    prev: MouseProtocolEncoding,
    out: &mut Vec<u8>,
) {
    if encoding == prev {
        return;
    }

    match prev {
        MouseProtocolEncoding::Default => {}
        MouseProtocolEncoding::Utf8 => out.extend_from_slice(b"\x1b[?1005l"),
        MouseProtocolEncoding::Sgr => out.extend_from_slice(b"\x1b[?1006l"),
    }
    match encoding {
        MouseProtocolEncoding::Default => {}
        MouseProtocolEncoding::Utf8 => out.extend_from_slice(b"\x1b[?1005h"),
        MouseProtocolEncoding::Sgr => out.extend_from_slice(b"\x1b[?1006h"),
    }
}

#[cfg(test)]
mod tests {
    use super::InputModes;
    use vt100::{MouseProtocolEncoding, MouseProtocolMode};

    #[test]
    fn diff_no_changes_is_empty() {
        let modes = InputModes::default();
        assert!(modes.diff_bytes(&modes).is_empty());
    }

    #[test]
    fn diff_enables_application_cursor() {
        let prev = InputModes::default();
        let next = InputModes {
            application_cursor: true,
            ..prev
        };
        assert_eq!(next.diff_bytes(&prev), b"\x1b[?1h");
    }

    #[test]
    fn diff_enables_mouse_mode() {
        let prev = InputModes::default();
        let next = InputModes {
            mouse_mode: MouseProtocolMode::ButtonMotion,
            ..prev
        };
        assert_eq!(next.diff_bytes(&prev), b"\x1b[?1002h");
    }

    #[test]
    fn diff_disables_mouse_mode() {
        let prev = InputModes {
            mouse_mode: MouseProtocolMode::PressRelease,
            ..InputModes::default()
        };
        let next = InputModes::default();
        assert_eq!(next.diff_bytes(&prev), b"\x1b[?1000l");
    }

    #[test]
    fn diff_replaces_the_pointer_capture_with_the_child_mouse_mode() {
        let prev = InputModes::default().with_shell_pointer_capture();
        let next = InputModes {
            mouse_mode: MouseProtocolMode::ButtonMotion,
            mouse_encoding: MouseProtocolEncoding::Utf8,
            ..InputModes::default()
        };

        assert_eq!(
            next.diff_bytes(&prev),
            b"\x1b[?1000l\x1b[?1002h\x1b[?1006l\x1b[?1005h"
        );
    }

    #[test]
    fn diff_enables_mouse_encoding() {
        let prev = InputModes::default();
        let next = InputModes {
            mouse_encoding: MouseProtocolEncoding::Sgr,
            ..prev
        };
        assert_eq!(next.diff_bytes(&prev), b"\x1b[?1006h");
    }

    #[test]
    fn focus_reporting_is_not_toggled_by_child_mode_diffs() {
        let prev = InputModes::default();
        let next = prev.with_focus_reporting(true);
        assert!(next.diff_bytes(&prev).is_empty());
        assert!(prev.diff_bytes(&next).is_empty());
    }

    #[test]
    fn shell_pointer_capture_only_supplies_a_mode_when_the_child_has_none() {
        let capture = InputModes::default().with_shell_pointer_capture();
        assert_eq!(capture.mouse_mode, MouseProtocolMode::PressRelease);
        assert_eq!(capture.mouse_encoding, MouseProtocolEncoding::Sgr);

        let child = InputModes {
            mouse_mode: MouseProtocolMode::ButtonMotion,
            mouse_encoding: MouseProtocolEncoding::Utf8,
            ..InputModes::default()
        };
        assert_eq!(child.with_shell_pointer_capture(), child);
    }
}
