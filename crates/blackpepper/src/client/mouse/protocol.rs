//! Stateful framing and viewport routing for decoded mouse events.

use super::{legacy_sequence, sgr_sequence, MouseEvent, MouseFormat, Sequence};
use ratatui::layout::Rect;
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

#[derive(Default)]
pub(in crate::client) struct MouseInputProtocol {
    pending: Vec<u8>,
    drag_active: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(in crate::client) struct MouseInput {
    pub(in crate::client) bytes: Vec<u8>,
    pub(in crate::client) shell_clicked: bool,
}

impl MouseInputProtocol {
    pub(in crate::client) fn process(
        &mut self,
        bytes: &[u8],
        area: Option<Rect>,
        encoding: MouseProtocolEncoding,
        mode: MouseProtocolMode,
    ) -> MouseInput {
        let mut input = std::mem::take(&mut self.pending);
        input.extend_from_slice(bytes);
        let mut output = MouseInput {
            bytes: Vec::with_capacity(input.len()),
            shell_clicked: false,
        };
        let mut index = 0;
        while index < input.len() {
            let remainder = &input[index..];
            if remainder.starts_with(b"\x1b[<") {
                match sgr_sequence(remainder) {
                    Sequence::Complete { length, event } => {
                        self.write_event(&mut output, event, area, MouseFormat::Sgr, mode);
                        index += length;
                    }
                    Sequence::Partial => {
                        self.pending.extend_from_slice(remainder);
                        break;
                    }
                    Sequence::Invalid => {
                        output.bytes.push(input[index]);
                        index += 1;
                    }
                }
            } else if remainder.starts_with(b"\x1b[M") {
                match legacy_sequence(remainder, encoding) {
                    Sequence::Complete { length, event } => {
                        self.write_event(
                            &mut output,
                            event,
                            area,
                            MouseFormat::Legacy(encoding),
                            mode,
                        );
                        index += length;
                    }
                    Sequence::Partial => {
                        self.pending.extend_from_slice(remainder);
                        break;
                    }
                    Sequence::Invalid => {
                        output.bytes.push(input[index]);
                        index += 1;
                    }
                }
            } else {
                output.bytes.push(input[index]);
                index += 1;
            }
        }
        // A footer click changes ownership from the PTY to Manage mode. Drop
        // any coalesced bytes from that same terminal read so a key intended
        // for the newly selected mode cannot leak into Zellij.
        if output.shell_clicked {
            output.bytes.clear();
        }
        output
    }

    fn write_event(
        &mut self,
        output: &mut MouseInput,
        mut event: MouseEvent,
        area: Option<Rect>,
        format: MouseFormat,
        mode: MouseProtocolMode,
    ) {
        let Some(area) = area else {
            if mode != MouseProtocolMode::None {
                event.encode(&mut output.bytes, format);
            }
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
        let is_plain_left_press =
            !event.release && event.code & 3 == 0 && event.code & (4 | 8 | 16 | 32 | 64) == 0;
        if !inside && is_plain_left_press {
            output.shell_clicked = true;
        }
        if mode == MouseProtocolMode::None {
            self.drag_active = false;
            return;
        }
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
        event.encode(&mut output.bytes, format);
    }
}
