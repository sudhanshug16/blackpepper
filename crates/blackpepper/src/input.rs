//! Raw input decoding and toggle detection.

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use termwiz::input::{InputEvent, InputParser, KeyCodeEncodeModes, KeyboardEncoding};

use crate::keymap::KeyChord;

pub struct InputDecoder {
    parser: InputParser,
    toggle_matcher: ToggleMatcher,
    logger: InputLogger,
}

impl InputDecoder {
    pub fn new(toggle_chord: Option<KeyChord>) -> Self {
        Self {
            parser: InputParser::new(),
            toggle_matcher: ToggleMatcher::new(toggle_sequences(toggle_chord.as_ref())),
            logger: InputLogger::new(),
        }
    }

    pub fn update_toggle_chord(&mut self, toggle_chord: Option<KeyChord>) {
        self.toggle_matcher
            .update_sequences(toggle_sequences(toggle_chord.as_ref()));
    }

    pub fn parse_manage_vec(&mut self, bytes: &[u8], maybe_more: bool) -> Vec<InputEvent> {
        self.logger.log_raw(bytes);
        let events = self.parser.parse_as_vec(bytes, maybe_more);
        for event in &events {
            self.logger.log_event(event);
        }
        events
    }

    pub fn flush_manage_vec(&mut self) -> Vec<InputEvent> {
        self.parse_manage_vec(&[], false)
    }

    pub fn consume_work_bytes(&mut self, bytes: &[u8]) -> (Vec<u8>, bool) {
        self.logger.log_raw(bytes);
        let (out, toggled, matched) = self.toggle_matcher.feed(bytes);
        if toggled {
            self.logger.log_toggle(&matched);
        }
        (out, toggled)
    }

    pub fn flush_work(&mut self) -> Vec<u8> {
        self.toggle_matcher.flush()
    }
}

#[derive(Default)]
struct ToggleMatcher {
    sequences: Vec<Vec<u8>>,
    max_len: usize,
    buffer: Vec<u8>,
}

impl ToggleMatcher {
    fn new(sequences: Vec<Vec<u8>>) -> Self {
        let max_len = sequences.iter().map(Vec::len).max().unwrap_or(0);
        Self {
            sequences,
            max_len,
            buffer: Vec::new(),
        }
    }

    fn update_sequences(&mut self, sequences: Vec<Vec<u8>>) {
        self.sequences = sequences;
        self.max_len = self.sequences.iter().map(Vec::len).max().unwrap_or(0);
        self.buffer.clear();
    }

    fn feed(&mut self, bytes: &[u8]) -> (Vec<u8>, bool, Vec<u8>) {
        if self.sequences.is_empty() {
            return (bytes.to_vec(), false, Vec::new());
        }

        self.buffer.extend_from_slice(bytes);
        if let Some((pos, len)) = self.find_first_match() {
            let mut out = Vec::new();
            out.extend_from_slice(&self.buffer[..pos]);
            let matched = self.buffer[pos..pos + len].to_vec();
            self.buffer.clear();
            return (out, true, matched);
        }

        let keep = self.longest_suffix_prefix();
        let mut out = Vec::new();
        if self.buffer.len() > keep {
            let drain_len = self.buffer.len() - keep;
            out.extend_from_slice(&self.buffer[..drain_len]);
            self.buffer.drain(..drain_len);
        }

        (out, false, Vec::new())
    }

    fn flush(&mut self) -> Vec<u8> {
        if self.buffer.is_empty() {
            return Vec::new();
        }
        let out = self.buffer.clone();
        self.buffer.clear();
        out
    }

    fn find_first_match(&self) -> Option<(usize, usize)> {
        for idx in 0..self.buffer.len() {
            for seq in &self.sequences {
                if seq.is_empty() {
                    continue;
                }
                if self.buffer[idx..].starts_with(seq) {
                    return Some((idx, seq.len()));
                }
            }
        }
        None
    }

    fn longest_suffix_prefix(&self) -> usize {
        if self.buffer.is_empty() {
            return 0;
        }
        let max = self.max_len.saturating_sub(1).min(self.buffer.len());
        for len in (1..=max).rev() {
            let suffix = &self.buffer[self.buffer.len() - len..];
            if self
                .sequences
                .iter()
                .any(|seq| seq.len() >= len && seq.starts_with(suffix))
            {
                return len;
            }
        }
        0
    }
}

fn toggle_sequences(chord: Option<&KeyChord>) -> Vec<Vec<u8>> {
    let Some(chord) = chord else {
        return Vec::new();
    };
    let mut sequences = HashSet::new();
    let mods = chord.modifiers.remove_positional_mods();

    let encodings = [KeyboardEncoding::Xterm, KeyboardEncoding::CsiU];
    let modify_other_keys = [None, Some(1), Some(2)];
    let bools = [false, true];

    for encoding in &encodings {
        for &modify in &modify_other_keys {
            if *encoding == KeyboardEncoding::CsiU && modify.is_some() {
                continue;
            }
            for &application_cursor_keys in &bools {
                for &newline_mode in &bools {
                    let modes = KeyCodeEncodeModes {
                        encoding: *encoding,
                        application_cursor_keys,
                        newline_mode,
                        modify_other_keys: modify,
                    };
                    if let Ok(seq) = chord.key.encode(mods, modes, true) {
                        if !seq.is_empty() {
                            sequences.insert(seq.into_bytes());
                        }
                    }
                }
            }
        }
    }

    sequences.into_iter().collect()
}

#[derive(Default)]
struct InputLogger {
    enabled: bool,
    path: PathBuf,
    file: Option<std::fs::File>,
}

impl InputLogger {
    fn new() -> Self {
        let enabled = std::env::var("BLACKPEPPER_DEBUG_INPUT")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        Self {
            enabled,
            path: PathBuf::from("/tmp/blackpepper-input.log"),
            file: None,
        }
    }

    fn log_raw(&mut self, bytes: &[u8]) {
        if !self.enabled || bytes.is_empty() {
            return;
        }
        let mut line = String::from("raw:");
        for byte in bytes {
            line.push(' ');
            line.push_str(&format!("{:02x}", byte));
        }
        self.write_line(&line);
    }

    fn log_event(&mut self, event: &InputEvent) {
        if !self.enabled {
            return;
        }
        self.write_line(&format!("event: {:?}", event));
    }

    fn log_toggle(&mut self, matched: &[u8]) {
        if !self.enabled || matched.is_empty() {
            return;
        }
        let mut line = String::from("toggle:");
        for byte in matched {
            line.push(' ');
            line.push_str(&format!("{:02x}", byte));
        }
        self.write_line(&line);
    }

    fn write_line(&mut self, line: &str) {
        if !self.enabled {
            return;
        }
        if self.file.is_none() {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path);
            match file {
                Ok(file) => {
                    self.file = Some(file);
                }
                Err(_) => {
                    self.enabled = false;
                    return;
                }
            }
        }
        if let Some(file) = self.file.as_mut() {
            let _ = writeln!(file, "{}", line);
        }
    }
}

#[cfg(test)]
mod tests {
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
}
