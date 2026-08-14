//! Focus-event tracking shared by the outer terminal and embedded sessions.

const FOCUS_IN: &[u8] = b"\x1b[I";
const FOCUS_OUT: &[u8] = b"\x1b[O";
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// Observe exact terminal focus events without consuming any input bytes.
///
/// Focus reports are three-byte CSI sequences (`CSI I` and `CSI O`). Keeping
/// this parser independent from key decoding lets Blackpepper remember the
/// outer window's focus while Manage mode owns input, and also handles an
/// event split across two PTY reads.
#[derive(Debug)]
pub(super) struct FocusTracker {
    pending: Vec<u8>,
    in_bracketed_paste: bool,
    focused: bool,
}

impl Default for FocusTracker {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            in_bracketed_paste: false,
            // A terminal starts an interactive client while its window is
            // focused. Later changes are authoritative CSI focus reports.
            focused: true,
        }
    }
}

impl FocusTracker {
    pub(super) fn observe(&mut self, bytes: &[u8]) -> Option<bool> {
        let mut observed = None;
        for byte in bytes {
            self.pending.push(*byte);
            loop {
                let candidates: &[&[u8]] = if self.in_bracketed_paste {
                    &[PASTE_END]
                } else {
                    &[FOCUS_IN, FOCUS_OUT, PASTE_START]
                };
                if candidates
                    .iter()
                    .any(|candidate| *candidate == self.pending)
                {
                    match self.pending.as_slice() {
                        FOCUS_IN => {
                            self.focused = true;
                            observed = Some(true);
                        }
                        FOCUS_OUT => {
                            self.focused = false;
                            observed = Some(false);
                        }
                        PASTE_START => self.in_bracketed_paste = true,
                        PASTE_END => self.in_bracketed_paste = false,
                        _ => unreachable!("candidate list contains every exact match"),
                    }
                    self.pending.clear();
                    break;
                }
                if candidates
                    .iter()
                    .any(|candidate| candidate.starts_with(&self.pending))
                {
                    break;
                }
                self.pending.remove(0);
                if self.pending.is_empty() {
                    break;
                }
            }
        }
        observed
    }

    pub(super) fn focused(&self) -> bool {
        self.focused
    }
}

#[cfg(test)]
mod tests {
    use super::FocusTracker;

    #[test]
    fn tracks_fragmented_focus_events() {
        let mut tracker = FocusTracker::default();

        assert_eq!(tracker.observe(b"before\x1b["), None);
        assert_eq!(tracker.observe(b"O"), Some(false));
        assert!(!tracker.focused());
        assert_eq!(tracker.observe(b"\x1b[I"), Some(true));
        assert!(tracker.focused());
    }

    #[test]
    fn ignores_other_csi_sequences() {
        let mut tracker = FocusTracker::default();

        assert_eq!(tracker.observe(b"\x1b[A\x1b[1;2I"), None);
        assert!(tracker.focused());
    }

    #[test]
    fn ignores_focus_shaped_text_inside_fragmented_bracketed_paste() {
        let mut tracker = FocusTracker::default();

        assert_eq!(tracker.observe(b"\x1b[20"), None);
        assert_eq!(tracker.observe(b"0~text\x1b[O"), None);
        assert!(tracker.focused());
        assert_eq!(tracker.observe(b"\x1b[201"), None);
        assert_eq!(tracker.observe(b"~\x1b[O"), Some(false));
        assert!(!tracker.focused());
    }
}
