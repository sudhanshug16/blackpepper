//! Mouse-coordinate translation for the embedded terminal viewport.
//!
//! Host terminals report positions relative to the full Blackpepper screen;
//! Zellij's PTY expects positions relative to the terminal panel. Keyboard and
//! paste bytes remain opaque.

use ratatui::layout::Rect;
use vt100::MouseProtocolEncoding;

const MAX_MOUSE_SEQUENCE: usize = 64;

#[derive(Default)]
pub(super) struct MouseInputProtocol {
    pending: Vec<u8>,
    drag_active: bool,
}

impl MouseInputProtocol {
    pub(super) fn process(
        &mut self,
        bytes: &[u8],
        area: Option<Rect>,
        encoding: MouseProtocolEncoding,
    ) -> Vec<u8> {
        let mut input = std::mem::take(&mut self.pending);
        input.extend_from_slice(bytes);
        let mut output = Vec::with_capacity(input.len());
        let mut index = 0;
        while index < input.len() {
            let remainder = &input[index..];
            if remainder.starts_with(b"\x1b[<") {
                match sgr_sequence(remainder) {
                    Sequence::Complete { length, event } => {
                        self.write_event(&mut output, event, area, MouseFormat::Sgr);
                        index += length;
                    }
                    Sequence::Partial => {
                        self.pending.extend_from_slice(remainder);
                        break;
                    }
                    Sequence::Invalid => {
                        output.push(input[index]);
                        index += 1;
                    }
                }
            } else if remainder.starts_with(b"\x1b[M") {
                match legacy_sequence(remainder, encoding) {
                    Sequence::Complete { length, event } => {
                        self.write_event(&mut output, event, area, MouseFormat::Legacy(encoding));
                        index += length;
                    }
                    Sequence::Partial => {
                        self.pending.extend_from_slice(remainder);
                        break;
                    }
                    Sequence::Invalid => {
                        output.push(input[index]);
                        index += 1;
                    }
                }
            } else {
                output.push(input[index]);
                index += 1;
            }
        }
        output
    }

    fn write_event(
        &mut self,
        output: &mut Vec<u8>,
        mut event: MouseEvent,
        area: Option<Rect>,
        format: MouseFormat,
    ) {
        let Some(area) = area else {
            event.encode(output, format);
            return;
        };
        if area.width == 0 || area.height == 0 {
            self.drag_active = false;
            return;
        }
        let inside = event.x > area.x
            && event.x <= area.x.saturating_add(area.width)
            && event.y > area.y
            && event.y <= area.y.saturating_add(area.height);
        let is_release = event.release || event.code & 3 == 3;
        let is_motion = event.code & 32 != 0;
        let is_wheel = event.code & 64 != 0;
        let should_forward = inside || self.drag_active && (is_motion || is_release);
        if !should_forward {
            if !is_motion && !is_wheel {
                self.drag_active = false;
            }
            return;
        }

        event.x = event
            .x
            .clamp(area.x.saturating_add(1), area.x.saturating_add(area.width))
            - area.x;
        event.y = event
            .y
            .clamp(area.y.saturating_add(1), area.y.saturating_add(area.height))
            - area.y;
        if is_release {
            self.drag_active = false;
        } else if !is_motion && !is_wheel {
            self.drag_active = true;
        }
        event.encode(output, format);
    }
}

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
