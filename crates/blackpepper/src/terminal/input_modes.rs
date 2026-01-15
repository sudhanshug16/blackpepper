//! Input mode tracking for the host terminal.
//!
//! tmux toggles terminal input modes (mouse, application cursor, etc.)
//! via escape sequences. We mirror those modes onto the host terminal so
//! tmux can receive mouse events directly.

use vt100::{MouseProtocolEncoding, MouseProtocolMode, Screen};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputModes {
    pub application_keypad: bool,
    pub application_cursor: bool,
    pub bracketed_paste: bool,
    pub mouse_mode: MouseProtocolMode,
    pub mouse_encoding: MouseProtocolEncoding,
}

impl Default for InputModes {
    fn default() -> Self {
        Self {
            application_keypad: false,
            application_cursor: false,
            bracketed_paste: false,
            mouse_mode: MouseProtocolMode::None,
            mouse_encoding: MouseProtocolEncoding::Default,
        }
    }
}

impl InputModes {
    pub fn from_screen(screen: &Screen) -> Self {
        Self {
            application_keypad: screen.application_keypad(),
            application_cursor: screen.application_cursor(),
            bracketed_paste: screen.bracketed_paste(),
            mouse_mode: screen.mouse_protocol_mode(),
            mouse_encoding: screen.mouse_protocol_encoding(),
        }
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

        write_mouse_mode_diff(self.mouse_mode, prev.mouse_mode, out);
        write_mouse_encoding_diff(self.mouse_encoding, prev.mouse_encoding, out);
    }
}

fn write_mouse_mode_diff(mode: MouseProtocolMode, prev: MouseProtocolMode, out: &mut Vec<u8>) {
    if mode == prev {
        return;
    }

    match mode {
        MouseProtocolMode::None => match prev {
            MouseProtocolMode::None => {}
            MouseProtocolMode::Press => out.extend_from_slice(b"\x1b[?9l"),
            MouseProtocolMode::PressRelease => {
                out.extend_from_slice(b"\x1b[?1000l");
            }
            MouseProtocolMode::ButtonMotion => {
                out.extend_from_slice(b"\x1b[?1002l");
            }
            MouseProtocolMode::AnyMotion => {
                out.extend_from_slice(b"\x1b[?1003l");
            }
        },
        MouseProtocolMode::Press => out.extend_from_slice(b"\x1b[?9h"),
        MouseProtocolMode::PressRelease => {
            out.extend_from_slice(b"\x1b[?1000h");
        }
        MouseProtocolMode::ButtonMotion => {
            out.extend_from_slice(b"\x1b[?1002h");
        }
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

    match encoding {
        MouseProtocolEncoding::Default => match prev {
            MouseProtocolEncoding::Default => {}
            MouseProtocolEncoding::Utf8 => out.extend_from_slice(b"\x1b[?1005l"),
            MouseProtocolEncoding::Sgr => out.extend_from_slice(b"\x1b[?1006l"),
        },
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
    fn diff_enables_mouse_encoding() {
        let prev = InputModes::default();
        let next = InputModes {
            mouse_encoding: MouseProtocolEncoding::Sgr,
            ..prev
        };
        assert_eq!(next.diff_bytes(&prev), b"\x1b[?1006h");
    }
}
