//! Key chord parsing and matching for configurable bindings.

use termwiz::input::{KeyCode, KeyEvent, Modifiers};

#[derive(Debug, Clone)]
pub struct KeyChord {
    pub key: KeyCode,
    pub modifiers: Modifiers,
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

    let mut key = None;
    let mut modifiers = Modifiers::NONE;

    for part in parts {
        match part {
            "ctrl" | "control" => modifiers |= Modifiers::CTRL,
            "alt" | "option" | "opt" | "meta" => modifiers |= Modifiers::ALT,
            "shift" => modifiers |= Modifiers::SHIFT,
            "super" | "cmd" | "command" | "win" => modifiers |= Modifiers::SUPER,
            value => {
                if key.is_some() {
                    return None;
                }
                key = parse_key(value);
            }
        }
    }

    let key = key?;
    Some(KeyChord { key, modifiers })
}

pub fn matches_chord(event: &KeyEvent, chord: &KeyChord) -> bool {
    let mods = event.modifiers.remove_positional_mods();
    let chord_mods = chord.modifiers.remove_positional_mods();
    if event.key == chord.key {
        if mods == chord_mods {
            return true;
        }
        if chord.key == KeyCode::Char('|')
            && mods.contains(Modifiers::SHIFT)
            && (mods & !Modifiers::SHIFT) == chord_mods
        {
            return true;
        }
    }
    if chord.key == KeyCode::Char('|') && event.key == KeyCode::Char('\\') {
        if !mods.contains(Modifiers::SHIFT) {
            return false;
        }
        let mods_no_shift = mods & !Modifiers::SHIFT;
        return mods_no_shift == chord_mods;
    }
    false
}

fn parse_key(key: &str) -> Option<KeyCode> {
    match key {
        "esc" | "escape" => Some(KeyCode::Escape),
        "enter" | "return" => Some(KeyCode::Enter),
        "tab" => Some(KeyCode::Tab),
        "space" | "spacebar" => Some(KeyCode::Char(' ')),
        "backspace" | "bs" => Some(KeyCode::Backspace),
        "up" => Some(KeyCode::UpArrow),
        "down" => Some(KeyCode::DownArrow),
        "left" => Some(KeyCode::LeftArrow),
        "right" => Some(KeyCode::RightArrow),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_key_chord_accepts_simple() {
        let chord = parse_key_chord("ctrl+]").expect("chord");
        assert_eq!(chord.key, KeyCode::Char(']'));
        assert!(chord.modifiers.contains(Modifiers::CTRL));
    }

    #[test]
    fn parse_key_chord_rejects_duplicate_key() {
        assert!(parse_key_chord("ctrl+a+b").is_none());
    }

    #[test]
    fn matches_chord_ignores_positional_mods() {
        let chord = KeyChord {
            key: KeyCode::Char('p'),
            modifiers: Modifiers::CTRL,
        };
        let event = KeyEvent {
            key: KeyCode::Char('p'),
            modifiers: Modifiers::CTRL | Modifiers::LEFT_CTRL,
        };
        assert!(matches_chord(&event, &chord));
    }

    #[test]
    fn matches_chord_pipe_accepts_backslash_variants() {
        let chord = KeyChord {
            key: KeyCode::Char('|'),
            modifiers: Modifiers::CTRL,
        };
        let event = KeyEvent {
            key: KeyCode::Char('\\'),
            modifiers: Modifiers::CTRL,
        };
        let event_shift = KeyEvent {
            key: KeyCode::Char('\\'),
            modifiers: Modifiers::CTRL | Modifiers::SHIFT,
        };
        let pipe_shift = KeyEvent {
            key: KeyCode::Char('|'),
            modifiers: Modifiers::CTRL | Modifiers::SHIFT,
        };
        assert!(!matches_chord(&event, &chord));
        assert!(matches_chord(&event_shift, &chord));
        assert!(matches_chord(&pipe_shift, &chord));
    }
}
