use crate::keymap::KeyChord;
use std::collections::HashSet;
use termwiz::input::{KeyCodeEncodeModes, KeyboardEncoding};

#[derive(Default)]
pub(super) struct ToggleMatcher {
    sequences: Vec<Vec<u8>>,
    max_len: usize,
    buffer: Vec<u8>,
}

impl ToggleMatcher {
    pub(super) fn new(sequences: Vec<Vec<u8>>) -> Self {
        let max_len = sequences.iter().map(Vec::len).max().unwrap_or(0);
        Self {
            sequences,
            max_len,
            buffer: Vec::new(),
        }
    }

    pub(super) fn update_sequences(&mut self, sequences: Vec<Vec<u8>>) {
        self.sequences = sequences;
        self.max_len = self.sequences.iter().map(Vec::len).max().unwrap_or(0);
        self.buffer.clear();
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) -> (Vec<u8>, bool, Vec<u8>) {
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

    pub(super) fn flush(&mut self) -> Vec<u8> {
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

pub(super) fn toggle_sequences(chord: Option<&KeyChord>) -> Vec<Vec<u8>> {
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
