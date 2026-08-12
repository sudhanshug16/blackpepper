use super::*;
use termwiz::input::{KeyCode, Modifiers};

#[test]
fn toggle_sequences_include_ctrl_mapping() {
    let chord = KeyChord {
        key: KeyCode::Char(']'),
        modifiers: Modifiers::CTRL,
    };
    let sequences = toggle_sequences(Some(&chord));
    assert!(sequences.iter().any(|seq| seq == b"\x1d"));
}

#[test]
fn matcher_strips_toggle_sequence() {
    let chord = KeyChord {
        key: KeyCode::Char(']'),
        modifiers: Modifiers::CTRL,
    };
    let sequences = toggle_sequences(Some(&chord));
    let mut matcher = ToggleMatcher::new(sequences);
    let (out, toggled, matched) = matcher.feed(b"hello\x1dworld");
    assert!(toggled);
    assert_eq!(out, b"hello");
    assert_eq!(matched, b"\x1d");
}

#[test]
fn matcher_buffers_partial_sequence() {
    let chord = KeyChord {
        key: KeyCode::Char(']'),
        modifiers: Modifiers::CTRL,
    };
    let sequences = toggle_sequences(Some(&chord));
    let sequence = sequences
        .iter()
        .find(|seq| seq.len() > 1)
        .cloned()
        .expect("expected multi-byte toggle sequence");
    let split_at = 2.min(sequence.len() - 1);
    let (first, rest) = sequence.split_at(split_at);
    let mut matcher = ToggleMatcher::new(sequences);
    let (out, toggled, _) = matcher.feed(first);
    assert!(!toggled);
    assert!(out.is_empty());
    let (out, toggled, _) = matcher.feed(rest);
    assert!(toggled);
    assert!(out.is_empty());
}

#[test]
fn input_decoder_matches_overlay_and_switch_chords() {
    let toggle = KeyChord {
        key: KeyCode::Char(']'),
        modifiers: Modifiers::CTRL,
    };
    let overlay = KeyChord {
        key: KeyCode::Char('o'),
        modifiers: Modifiers::CTRL,
    };
    let switch = KeyChord {
        key: KeyCode::Char('u'),
        modifiers: Modifiers::CTRL,
    };
    let overlay_sequence = toggle_sequences(Some(&overlay))
        .into_iter()
        .next()
        .expect("overlay sequence");
    let switch_sequence = toggle_sequences(Some(&switch))
        .into_iter()
        .next()
        .expect("switch sequence");
    let mut decoder = InputDecoder::new(Some(toggle), Some(overlay), Some(switch));
    let (out, matched) = decoder.consume_work_bytes(&overlay_sequence);
    assert!(out.is_empty());
    assert_eq!(matched, MatchedChord::WorkspaceOverlay);
    let (out, matched) = decoder.consume_work_bytes(&switch_sequence);
    assert!(out.is_empty());
    assert_eq!(matched, MatchedChord::Switch);
}
