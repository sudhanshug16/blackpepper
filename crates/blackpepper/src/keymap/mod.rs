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
