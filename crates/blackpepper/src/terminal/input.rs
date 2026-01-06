//! Input encoding for terminal PTY.
//!
//! Converts crossterm key and mouse events into byte sequences
//! that the PTY/shell understands. Handles:
//! - Standard keys (characters, enter, tab, arrows, etc.)
//! - Control key combinations (Ctrl+A through Ctrl+Z)
//! - Mouse events in various protocols (X10, SGR, etc.)

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

/// Convert a key event to bytes for the PTY.
///
/// Returns None for keys we don't handle (e.g., function keys).
pub fn key_event_to_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    match key.code {
        KeyCode::Char(ch) => {
            // Ctrl+letter produces control codes (1-26)
            let mut bytes = if key.modifiers.contains(KeyModifiers::CONTROL) {
                let lowercase = ch.to_ascii_lowercase();
                if lowercase.is_ascii_lowercase() {
                    let code = (lowercase as u8 - b'a') + 1;
                    vec![code]
                } else {
                    let mut buffer = [0u8; 4];
                    let encoded = ch.encode_utf8(&mut buffer);
                    encoded.as_bytes().to_vec()
                }
            } else {
                let mut buffer = [0u8; 4];
                let encoded = ch.encode_utf8(&mut buffer);
                encoded.as_bytes().to_vec()
            };
            if key.modifiers.contains(KeyModifiers::ALT) {
                let mut prefixed = Vec::with_capacity(bytes.len() + 1);
                prefixed.push(0x1b);
                prefixed.append(&mut bytes);
                return Some(prefixed);
            }
            Some(bytes)
        }
        KeyCode::Enter => Some(encode_enter(key.modifiers)),
        KeyCode::Tab => Some(encode_tab(key.modifiers)),
        KeyCode::BackTab => Some(encode_shift_tab(key.modifiers)),
        KeyCode::Backspace => Some(with_alt_prefix(key.modifiers, vec![0x7f])),
        KeyCode::Esc => Some(with_alt_prefix(key.modifiers, vec![0x1b])),
        KeyCode::Up => Some(encode_csi_key(key.modifiers, "A", "\x1b[A")),
        KeyCode::Down => Some(encode_csi_key(key.modifiers, "B", "\x1b[B")),
        KeyCode::Right => Some(encode_csi_key(key.modifiers, "C", "\x1b[C")),
        KeyCode::Left => Some(encode_csi_key(key.modifiers, "D", "\x1b[D")),
        KeyCode::Delete => Some(encode_csi_tilde_key(key.modifiers, "3", "\x1b[3~")),
        KeyCode::Home => Some(encode_csi_key(key.modifiers, "H", "\x1b[H")),
        KeyCode::End => Some(encode_csi_key(key.modifiers, "F", "\x1b[F")),
        KeyCode::PageUp => Some(encode_csi_tilde_key(key.modifiers, "5", "\x1b[5~")),
        KeyCode::PageDown => Some(encode_csi_tilde_key(key.modifiers, "6", "\x1b[6~")),
        _ => None,
    }
}

fn with_alt_prefix(modifiers: KeyModifiers, bytes: Vec<u8>) -> Vec<u8> {
    if !modifiers.contains(KeyModifiers::ALT) {
        return bytes;
    }
    let mut prefixed = Vec::with_capacity(bytes.len() + 1);
    prefixed.push(0x1b);
    prefixed.extend(bytes);
    prefixed
}

fn modifier_param(modifiers: KeyModifiers) -> Option<u8> {
    let mut value = 1;
    let mut has = false;
    if modifiers.contains(KeyModifiers::SHIFT) {
        value += 1;
        has = true;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        value += 2;
        has = true;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        value += 4;
        has = true;
    }
    if has {
        Some(value)
    } else {
        None
    }
}

fn encode_csi_key(modifiers: KeyModifiers, final_byte: &str, base: &str) -> Vec<u8> {
    if let Some(param) = modifier_param(modifiers) {
        return format!("\x1b[1;{param}{final_byte}").into_bytes();
    }
    base.as_bytes().to_vec()
}

fn encode_csi_tilde_key(modifiers: KeyModifiers, code: &str, base: &str) -> Vec<u8> {
    if let Some(param) = modifier_param(modifiers) {
        return format!("\x1b[{code};{param}~").into_bytes();
    }
    base.as_bytes().to_vec()
}

fn encode_tab(modifiers: KeyModifiers) -> Vec<u8> {
    if modifiers.contains(KeyModifiers::SHIFT) {
        return encode_shift_tab(modifiers);
    }
    with_alt_prefix(modifiers, vec![b'\t'])
}

fn encode_shift_tab(modifiers: KeyModifiers) -> Vec<u8> {
    if let Some(param) = modifier_param(modifiers) {
        return format!("\x1b[1;{param}Z").into_bytes();
    }
    b"\x1b[Z".to_vec()
}

fn encode_enter(modifiers: KeyModifiers) -> Vec<u8> {
    if let Some(param) = modifier_param(modifiers) {
        return format!("\x1b[13;{param}u").into_bytes();
    }
    with_alt_prefix(modifiers, vec![b'\r'])
}

/// Convert a mouse event to bytes for the PTY.
///
/// Respects the terminal's mouse protocol mode and encoding.
/// Returns None if the event shouldn't be sent (e.g., mode is None).
pub fn mouse_event_to_bytes(
    event: MouseEvent,
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    if mode == MouseProtocolMode::None {
        return None;
    }

    // Check if this event type is allowed by the current mode
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

    // Build the button/action code
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
                3 // X10 release code
            }
        }
        MouseEventKind::Drag(button) => match button {
            MouseButton::Left => 32,
            MouseButton::Middle => 33,
            MouseButton::Right => 34,
        },
        MouseEventKind::Moved => 35,
    };

    // Add modifier bits
    if event.modifiers.contains(KeyModifiers::SHIFT) {
        code += 4;
    }
    if event.modifiers.contains(KeyModifiers::ALT) {
        code += 8;
    }
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        code += 16;
    }

    // 1-based coordinates
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
