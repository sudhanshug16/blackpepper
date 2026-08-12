//! Bounded OSC handling needed when Blackpepper is the outer terminal.
//!
//! Zellij remains responsible for copy mode and emits OSC 52 writes. Clipboard
//! reads are intentionally ignored so a remote process cannot exfiltrate the
//! client's clipboard through an otherwise ordinary terminal query.

use base64::{engine::general_purpose::STANDARD, Engine as _};

const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;
// A 1 MiB clipboard value expands under base64. Keep the parser bounded while
// still accepting the documented decoded limit when an OSC write is split.
const MAX_PENDING_OSC_BYTES: usize = MAX_CLIPBOARD_BYTES.div_ceil(3) * 4 + 16;

#[derive(Debug, PartialEq, Eq)]
pub enum OscAction {
    WriteToPty(Vec<u8>),
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
                if remainder.len() <= MAX_PENDING_OSC_BYTES {
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
mod tests {
    use super::*;

    #[test]
    fn handles_split_clipboard_write_without_returning_evidence() {
        let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));
        assert!(protocol.process(b"\x1b").is_empty());
        assert!(protocol.process(b"]52;c;aGV").is_empty());
        assert_eq!(
            protocol.process(b"sbG8=\x1b\\"),
            vec![OscAction::SetClipboard {
                target: ClipboardTarget::System,
                text: "hello".to_string(),
            }]
        );
    }

    #[test]
    fn preserves_primary_selection_and_rejects_unknown_targets() {
        let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));
        assert_eq!(
            protocol.process(b"\x1b]52;p;aGVsbG8=\x07"),
            vec![OscAction::SetClipboard {
                target: ClipboardTarget::Primary,
                text: "hello".to_string(),
            }]
        );
        assert!(protocol.process(b"\x1b]52;s;aGVsbG8=\x07").is_empty());
    }

    #[test]
    fn clipboard_read_is_ignored() {
        let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));
        assert!(protocol.process(b"\x1b]52;c;?\x07").is_empty());
    }

    #[test]
    fn outer_clipboard_write_is_bounded_and_normalized() {
        assert_eq!(
            clipboard_write_sequence(ClipboardTarget::System, "hello"),
            Some(b"\x1b]52;c;aGVsbG8=\x07".to_vec())
        );
        assert_eq!(
            clipboard_write_sequence(ClipboardTarget::Primary, "hello"),
            Some(b"\x1b]52;p;aGVsbG8=\x07".to_vec())
        );
        assert!(clipboard_write_sequence(
            ClipboardTarget::System,
            &"x".repeat(MAX_CLIPBOARD_BYTES + 1)
        )
        .is_none());
    }

    #[test]
    fn split_clipboard_write_accepts_the_full_decoded_limit() {
        let text = "x".repeat(MAX_CLIPBOARD_BYTES);
        let sequence = clipboard_write_sequence(ClipboardTarget::System, &text).unwrap();
        let split = sequence.len() - 1;
        let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));

        assert!(protocol.process(&sequence[..split]).is_empty());
        let actions = protocol.process(&sequence[split..]);
        assert!(matches!(
            actions.as_slice(),
            [OscAction::SetClipboard {
                target: ClipboardTarget::System,
                text: decoded,
            }] if decoded.len() == MAX_CLIPBOARD_BYTES
        ));
    }

    #[test]
    fn replies_to_terminal_color_queries() {
        let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));
        assert_eq!(
            protocol.process(b"\x1b]10;?\x07\x1b]11;?\x07"),
            vec![
                OscAction::WriteToPty(b"\x1b]10;rgb:0101/0202/0303\x07".to_vec()),
                OscAction::WriteToPty(b"\x1b]11;rgb:0404/0505/0606\x07".to_vec()),
            ]
        );
    }
}
