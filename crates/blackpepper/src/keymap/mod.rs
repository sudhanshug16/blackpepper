//! Key chord parsing and matching.
//!
//! Parses key chord strings like "ctrl+p" or "alt+shift+t" from config
//! and matches them against crossterm KeyEvents at runtime.
//!
//! Used for configurable keybindings (toggle mode, switch workspace, etc.).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone)]
pub struct KeyChord {
    pub key: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

pub const DEFAULT_WORK_TOGGLE_BYTE: u8 = 0x07;

pub fn parse_key_chord(input: &str) -> Option<KeyChord> {
    let trimmed = input.trim().to_lowercase();
    if trimmed.is_empty() {
        return None;
    }

    let parts: Vec<&str> = trimmed
        .split('+')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }

    let mut chord = KeyChord {
        key: KeyCode::Null,
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
    };

    for part in parts {
        match part {
            "ctrl" | "control" => chord.ctrl = true,
            "alt" | "option" => chord.alt = true,
            "shift" => chord.shift = true,
            "meta" | "cmd" | "super" => chord.meta = true,
            key => {
                if chord.key != KeyCode::Null {
                    return None;
                }
                chord.key = parse_key(key)?;
            }
        }
    }

    if chord.key == KeyCode::Null {
        return None;
    }

    Some(chord)
}

/// Parse a key chord into a single control byte for raw work-mode toggles.
///
/// Only control-only chords are supported (e.g., "ctrl+g", "ctrl+[", "ctrl+space").
pub fn parse_control_byte(input: &str) -> Option<u8> {
    let chord = parse_key_chord(input)?;
    if !chord.ctrl || chord.alt || chord.shift || chord.meta {
        return None;
    }
    match chord.key {
        KeyCode::Char(ch) => control_char_byte(ch),
        KeyCode::Esc => Some(0x1b),
        _ => None,
    }
}

fn parse_key(key: &str) -> Option<KeyCode> {
    match key {
        "esc" | "escape" => Some(KeyCode::Esc),
        "enter" | "return" => Some(KeyCode::Enter),
        "tab" => Some(KeyCode::Tab),
        "space" | "spacebar" => Some(KeyCode::Char(' ')),
        _ => {
            let mut chars = key.chars();
            let first = chars.next()?;
            if chars.next().is_none() {
                Some(KeyCode::Char(first))
            } else {
                None
            }
        }
    }
}

fn control_char_byte(ch: char) -> Option<u8> {
    match ch {
        ' ' => Some(0x00),
        '@' => Some(0x00),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '?' => Some(0x7f),
        _ => {
            let lower = ch.to_ascii_lowercase();
            if lower.is_ascii_lowercase() {
                Some((lower as u8 - b'a') + 1)
            } else {
                None
            }
        }
    }
}

pub fn matches_chord(event: KeyEvent, chord: &KeyChord) -> bool {
    if event.code != chord.key {
        return false;
    }

    let modifiers = event.modifiers;
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let alt = modifiers.contains(KeyModifiers::ALT);
    let shift = modifiers.contains(KeyModifiers::SHIFT);
    let meta = modifiers.contains(KeyModifiers::SUPER) || modifiers.contains(KeyModifiers::META);

    ctrl == chord.ctrl && alt == chord.alt && shift == chord.shift && meta == chord.meta
}

#[cfg(test)]
mod tests {
    use super::parse_control_byte;

    #[test]
    fn parse_control_byte_accepts_ctrl_letters() {
        assert_eq!(parse_control_byte("ctrl+g"), Some(0x07));
        assert_eq!(parse_control_byte("ctrl+A"), Some(0x01));
    }

    #[test]
    fn parse_control_byte_accepts_ctrl_symbols() {
        assert_eq!(parse_control_byte("ctrl+["), Some(0x1b));
        assert_eq!(parse_control_byte("ctrl+space"), Some(0x00));
        assert_eq!(parse_control_byte("ctrl+?"), Some(0x7f));
    }

    #[test]
    fn parse_control_byte_rejects_non_control() {
        assert_eq!(parse_control_byte("alt+g"), None);
        assert_eq!(parse_control_byte("shift+g"), None);
        assert_eq!(parse_control_byte("ctrl+shift+g"), None);
        assert_eq!(parse_control_byte("enter"), None);
    }
}
