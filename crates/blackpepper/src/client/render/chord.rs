//! Key chords as the design writes them.
//!
//! Config keeps `ctrl+]` as the binding's source of truth — that is what a user
//! types into `config.toml` and what the parser accepts. Status rows are the
//! tightest surface in the client, so they spend two columns on `^]` instead of
//! six on `ctrl+]`.

/// `ctrl+X` becomes `^X`. Anything with a second modifier, a named key, or a
/// multi-character tail is left exactly as configured, because an abbreviation
/// nobody can map back to a keystroke is worse than a long one.
pub(super) fn chord_label(binding: &str) -> String {
    let trimmed = binding.trim();
    let Some(rest) = trimmed
        .strip_prefix("ctrl+")
        .or_else(|| trimmed.strip_prefix("Ctrl+"))
    else {
        return trimmed.to_owned();
    };
    let mut characters = rest.chars();
    match (characters.next(), characters.next()) {
        (Some(key), None) => format!("^{}", key.to_ascii_uppercase()),
        _ => trimmed.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::chord_label;

    #[test]
    fn single_character_control_chords_use_caret_notation() {
        assert_eq!(chord_label("ctrl+]"), "^]");
        assert_eq!(chord_label("ctrl+n"), "^N");
        assert_eq!(chord_label("ctrl+\\"), "^\\");
    }

    #[test]
    fn anything_unabbreviable_is_left_as_configured() {
        assert_eq!(chord_label("ctrl+shift+n"), "ctrl+shift+n");
        assert_eq!(chord_label("alt+n"), "alt+n");
        assert_eq!(chord_label("f5"), "f5");
        assert_eq!(chord_label("ctrl+enter"), "ctrl+enter");
    }
}
