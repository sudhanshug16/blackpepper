//! Mouse-coordinate translation for the embedded terminal viewport.
//!
//! Host terminals report positions relative to the full Blackpepper screen;
//! Zellij's PTY expects positions relative to the terminal panel. Keyboard and
//! paste bytes remain opaque.

mod protocol;

pub(super) use protocol::MouseInputProtocol;
use vt100::MouseProtocolEncoding;

const MAX_MOUSE_SEQUENCE: usize = 64;

#[derive(Clone, Copy)]
struct MouseEvent {
    code: u16,
    x: u16,
    y: u16,
    release: bool,
}

impl MouseEvent {
    fn encode(self, output: &mut Vec<u8>, format: MouseFormat) {
        match format {
            MouseFormat::Sgr => output.extend_from_slice(
                format!(
                    "\x1b[<{};{};{}{}",
                    self.code,
                    self.x,
                    self.y,
                    if self.release { 'm' } else { 'M' }
                )
                .as_bytes(),
            ),
            MouseFormat::Legacy(MouseProtocolEncoding::Default) => {
                if let (Ok(code), Ok(x), Ok(y)) = (
                    u8::try_from(self.code.saturating_add(32)),
                    u8::try_from(self.x.saturating_add(32)),
                    u8::try_from(self.y.saturating_add(32)),
                ) {
                    output.extend_from_slice(&[0x1b, b'[', b'M', code, x, y]);
                }
            }
            MouseFormat::Legacy(MouseProtocolEncoding::Utf8) => {
                output.extend_from_slice(b"\x1b[M");
                for value in [self.code, self.x, self.y] {
                    if let Some(character) = char::from_u32(u32::from(value) + 32) {
                        let mut encoded = [0; 4];
                        output.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
                    }
                }
            }
            MouseFormat::Legacy(MouseProtocolEncoding::Sgr) => {}
        }
    }
}

enum MouseFormat {
    Sgr,
    Legacy(MouseProtocolEncoding),
}

enum Sequence {
    Complete { length: usize, event: MouseEvent },
    Partial,
    Invalid,
}

fn sgr_sequence(bytes: &[u8]) -> Sequence {
    let Some(end) = bytes
        .iter()
        .take(MAX_MOUSE_SEQUENCE)
        .position(|byte| *byte == b'M' || *byte == b'm')
    else {
        return if bytes.len() < MAX_MOUSE_SEQUENCE {
            Sequence::Partial
        } else {
            Sequence::Invalid
        };
    };
    let Ok(body) = std::str::from_utf8(&bytes[3..end]) else {
        return Sequence::Invalid;
    };
    let mut fields = body.split(';').map(str::parse::<u16>);
    let (Some(Ok(code)), Some(Ok(x)), Some(Ok(y)), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return Sequence::Invalid;
    };
    Sequence::Complete {
        length: end + 1,
        event: MouseEvent {
            code,
            x,
            y,
            release: bytes[end] == b'm',
        },
    }
}

fn legacy_sequence(bytes: &[u8], encoding: MouseProtocolEncoding) -> Sequence {
    match encoding {
        MouseProtocolEncoding::Default => {
            if bytes.len() < 6 {
                return Sequence::Partial;
            }
            let [code, x, y] = [bytes[3], bytes[4], bytes[5]].map(|value| value.saturating_sub(32));
            Sequence::Complete {
                length: 6,
                event: MouseEvent {
                    code: u16::from(code),
                    x: u16::from(x),
                    y: u16::from(y),
                    release: code & 3 == 3,
                },
            }
        }
        MouseProtocolEncoding::Utf8 => utf8_legacy_sequence(bytes),
        MouseProtocolEncoding::Sgr => Sequence::Invalid,
    }
}

fn utf8_legacy_sequence(bytes: &[u8]) -> Sequence {
    let Ok(text) = std::str::from_utf8(&bytes[3..]) else {
        return if bytes.len() < MAX_MOUSE_SEQUENCE {
            Sequence::Partial
        } else {
            Sequence::Invalid
        };
    };
    let mut characters = text.char_indices();
    let (Some((_, code)), Some((_, x)), Some((last, y))) =
        (characters.next(), characters.next(), characters.next())
    else {
        return Sequence::Partial;
    };
    let length = 3 + last + y.len_utf8();
    let Some(code) = u16::try_from(u32::from(code))
        .ok()
        .and_then(|value| value.checked_sub(32))
    else {
        return Sequence::Invalid;
    };
    let Some(x) = u16::try_from(u32::from(x))
        .ok()
        .and_then(|value| value.checked_sub(32))
    else {
        return Sequence::Invalid;
    };
    let Some(y) = u16::try_from(u32::from(y))
        .ok()
        .and_then(|value| value.checked_sub(32))
    else {
        return Sequence::Invalid;
    };
    Sequence::Complete {
        length,
        event: MouseEvent {
            code,
            x,
            y,
            release: code & 3 == 3,
        },
    }
}

#[cfg(test)]
#[path = "mouse_tests.rs"]
mod tests;
