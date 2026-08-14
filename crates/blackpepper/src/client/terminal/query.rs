//! Bounded parsing for terminal queries and modes not exposed by `vt100`.

const MAX_SEQUENCE_BYTES: usize = 64;

#[derive(Default)]
pub(super) struct TerminalQueryProtocol {
    pending: Vec<u8>,
    focus_reporting: bool,
}

impl TerminalQueryProtocol {
    pub(super) fn process(&mut self, bytes: &[u8], rows: u16, cols: u16) -> Vec<Vec<u8>> {
        if bytes.is_empty() {
            return Vec::new();
        }
        let combined;
        let input = if self.pending.is_empty() {
            bytes
        } else {
            combined = [self.pending.as_slice(), bytes].concat();
            self.pending.clear();
            &combined
        };
        let mut replies = Vec::new();
        let mut cursor = 0;
        while cursor < input.len() {
            let Some(offset) = input[cursor..].iter().position(|byte| *byte == 0x1b) else {
                break;
            };
            let start = cursor + offset;
            if start + 1 >= input.len() {
                self.pending.extend_from_slice(&input[start..]);
                break;
            }
            if input[start + 1] != b'[' {
                cursor = start + 2;
                continue;
            }
            let mut end = start + 2;
            while end < input.len()
                && !(0x40..=0x7e).contains(&input[end])
                && end - start < MAX_SEQUENCE_BYTES
            {
                end += 1;
            }
            if end >= input.len() {
                if input.len() - start <= MAX_SEQUENCE_BYTES {
                    self.pending.extend_from_slice(&input[start..]);
                }
                break;
            }
            if end - start >= MAX_SEQUENCE_BYTES {
                cursor = end;
                continue;
            }

            let parameters = &input[start + 2..end];
            match (input[end], parameters) {
                (b't', b"18") => {
                    replies.push(format!("\x1b[8;{};{}t", rows.max(1), cols.max(1)).into_bytes())
                }
                // A focus-aware embedded Zellij client asks its immediate
                // terminal (Blackpepper) for CSI I/O. `vt100` does not expose
                // this DEC mode, so remember it for outer-mode mirroring.
                (b'h', b"?1004") => self.focus_reporting = true,
                (b'l', b"?1004") => self.focus_reporting = false,
                _ => {}
            }
            cursor = end + 1;
        }
        replies
    }

    pub(super) fn focus_reporting(&self) -> bool {
        self.focus_reporting
    }
}

#[cfg(test)]
mod tests {
    use super::TerminalQueryProtocol;

    #[test]
    fn answers_text_area_size_queries_across_chunks() {
        let mut protocol = TerminalQueryProtocol::default();

        assert_eq!(
            protocol.process(b"before\x1b[18tafter", 42, 137),
            [b"\x1b[8;42;137t".to_vec()]
        );
        assert!(protocol.process(b"\x1b[1", 42, 137).is_empty());
        assert_eq!(protocol.process(b"8t", 24, 80), [b"\x1b[8;24;80t".to_vec()]);
    }

    #[test]
    fn tracks_split_focus_reporting_mode() {
        let mut protocol = TerminalQueryProtocol::default();

        protocol.process(b"\x1b[?10", 24, 80);
        protocol.process(b"04h", 24, 80);
        assert!(protocol.focus_reporting());

        protocol.process(b"\x1b[?1004l", 24, 80);
        assert!(!protocol.focus_reporting());
    }

    #[test]
    fn ignores_unknown_queries_and_modes() {
        let mut protocol = TerminalQueryProtocol::default();

        assert!(protocol
            .process(b"\x1b[14t\x1b[16t\x1b[6n\x1b[?1003h", 24, 80)
            .is_empty());
        assert!(!protocol.focus_reporting());
    }
}
