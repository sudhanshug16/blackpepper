//! Bounded OSC handling needed when Blackpepper is the outer terminal.
//!
//! The embedded terminal emits clipboard and notification-related sequences
//! which the client validates here. Clipboard reads are intentionally ignored
//! so a remote process cannot exfiltrate the client's clipboard through an
//! otherwise ordinary terminal query.

mod notification;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use notification::notification_sequence;

const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;
// OSC 777 carries two independently capped UTF-8 fields. Four bytes per
// Unicode scalar plus command framing is the largest valid split sequence.
const MAX_NOTIFICATION_OSC_BYTES: usize = notification::MAX_NOTIFICATION_FIELD_CHARS * 8 + 32;
// A 1 MiB clipboard value expands under base64. Keep the parser bounded while
// still accepting the documented decoded limit when an OSC write is split.
const MAX_PENDING_OSC_BYTES: usize = MAX_CLIPBOARD_BYTES.div_ceil(3) * 4 + 16;

#[derive(Debug, PartialEq, Eq)]
pub enum OscAction {
    WriteToPty(Vec<u8>),
    WriteToOuter(Vec<u8>),
    SetClipboard {
        target: ClipboardTarget,
        text: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardTarget {
    System,
    Primary,
}

impl ClipboardTarget {
    fn selector(self) -> char {
        match self {
            Self::System => 'c',
            Self::Primary => 'p',
        }
    }
}

pub struct OscProtocol {
    pending: Vec<u8>,
    foreground: (u8, u8, u8),
    background: (u8, u8, u8),
}

impl OscProtocol {
    pub fn new(foreground: (u8, u8, u8), background: (u8, u8, u8)) -> Self {
        Self {
            pending: Vec::new(),
            foreground,
            background,
        }
    }

    pub fn process(&mut self, bytes: &[u8]) -> Vec<OscAction> {
        if bytes.is_empty() {
            return Vec::new();
        }
        if self.pending.is_empty() {
            return self.scan(bytes);
        }
        let mut input = std::mem::take(&mut self.pending);
        if input.len().saturating_add(bytes.len()) > MAX_PENDING_OSC_BYTES {
            return Vec::new();
        }
        input.extend_from_slice(bytes);
        self.scan(&input)
    }

    fn scan(&mut self, bytes: &[u8]) -> Vec<OscAction> {
        let mut actions = Vec::new();
        let mut index = 0;
        while index < bytes.len() {
            // Preserve terminal bells for the real outer terminal instead of
            // only feeding them to Blackpepper's screen parser.
            if bytes[index] == 0x07 {
                push_outer(&mut actions, &[0x07]);
                index += 1;
                continue;
            }
            if bytes[index] != 0x1b || bytes.get(index + 1) != Some(&b']') {
                index += 1;
                continue;
            }
            let start = index;
            index += 2;
            let mut end = None;
            while index < bytes.len() {
                if bytes[index] == 0x07 {
                    end = Some((index, index + 1));
                    break;
                }
                if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
                    end = Some((index, index + 2));
                    break;
                }
                index += 1;
            }
            let Some((body_end, next)) = end else {
                let remainder = &bytes[start..];
                let maximum = if remainder.starts_with(b"\x1b]9;")
                    || remainder.starts_with(b"\x1b]777;")
                    || remainder.starts_with(b"\x1b]99;")
                {
                    MAX_NOTIFICATION_OSC_BYTES
                } else {
                    MAX_PENDING_OSC_BYTES
                };
                if remainder.len() <= maximum {
                    self.pending.extend_from_slice(remainder);
                }
                break;
            };
            self.action_for(&bytes[start + 2..body_end], &mut actions);
            index = next;
        }
        if self.pending.is_empty() && bytes.last() == Some(&0x1b) {
            self.pending.push(0x1b);
        }
        actions
    }

    fn action_for(&self, body: &[u8], actions: &mut Vec<OscAction>) {
        if body.starts_with(b"10;?") {
            actions.push(OscAction::WriteToPty(color_response(10, self.foreground)));
            return;
        }
        if body.starts_with(b"11;?") {
            actions.push(OscAction::WriteToPty(color_response(11, self.background)));
            return;
        }
        if body.starts_with(b"9;") || body.starts_with(b"777;") || body.starts_with(b"99;") {
            if let Some(sequence) = notification_sequence(body) {
                push_outer(actions, &sequence);
            }
            return;
        }
        let Some(rest) = body.strip_prefix(b"52;") else {
            return;
        };
        let mut parts = rest.splitn(2, |byte| *byte == b';');
        let target = match parts.next() {
            Some(b"" | b"c") => ClipboardTarget::System,
            Some(b"p") => ClipboardTarget::Primary,
            _ => return,
        };
        let Some(payload) = parts.next() else {
            return;
        };
        if payload == b"?" || payload.len() > encoded_limit(MAX_CLIPBOARD_BYTES) {
            return;
        }
        let Ok(decoded) = STANDARD.decode(payload) else {
            return;
        };
        if decoded.len() > MAX_CLIPBOARD_BYTES {
            return;
        }
        if let Ok(text) = String::from_utf8(decoded) {
            actions.push(OscAction::SetClipboard { target, text });
        }
    }
}

/// Keep adjacent outer-terminal bytes in one write. A noisy process can emit
/// thousands of BELs in one PTY read; forwarding each as its own allocation
/// and stdout flush would stall Blackpepper's event loop.
fn push_outer(actions: &mut Vec<OscAction>, bytes: &[u8]) {
    if let Some(OscAction::WriteToOuter(sequence)) = actions.last_mut() {
        sequence.extend_from_slice(bytes);
    } else {
        actions.push(OscAction::WriteToOuter(bytes.to_vec()));
    }
}

/// Rebuild an accepted clipboard write before passing it to the outer
/// terminal. This deliberately rebuilds the source encoding and terminator.
/// Blackpepper preserves Zellij's system/primary choice, never forwards
/// clipboard reads, and never copies an unvalidated escape sequence.
pub fn clipboard_write_sequence(target: ClipboardTarget, text: &str) -> Option<Vec<u8>> {
    (text.len() <= MAX_CLIPBOARD_BYTES).then(|| {
        let payload = STANDARD.encode(text.as_bytes());
        format!("\x1b]52;{};{payload}\x07", target.selector()).into_bytes()
    })
}

fn encoded_limit(decoded: usize) -> usize {
    decoded.saturating_add(2) / 3 * 4
}

fn color_response(kind: u8, rgb: (u8, u8, u8)) -> Vec<u8> {
    let component = |value: u8| u16::from(value) * 257;
    format!(
        "\x1b]{kind};rgb:{:04x}/{:04x}/{:04x}\x07",
        component(rgb.0),
        component(rgb.1),
        component(rgb.2)
    )
    .into_bytes()
}

#[cfg(test)]
#[path = "osc/tests.rs"]
mod tests;
