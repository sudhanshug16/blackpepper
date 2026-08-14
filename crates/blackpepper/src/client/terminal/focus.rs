use super::super::focus::FocusTracker;

#[derive(Debug, Default)]
pub(super) struct VisibilityFocus {
    pub(super) delivered: Option<bool>,
    forwarded: FocusTracker,
    #[cfg(test)]
    pub(super) history: Vec<bool>,
}

impl VisibilityFocus {
    pub(super) fn reset(&mut self) {
        self.delivered = None;
        self.forwarded = FocusTracker::default();
    }

    pub(super) fn observe_forwarded(&mut self, bytes: &[u8], focus_reporting: bool) {
        if !focus_reporting {
            return;
        }
        if let Some(focused) = self.forwarded.observe(bytes) {
            self.record(focused);
        }
    }

    pub(super) fn record(&mut self, focused: bool) {
        if self.delivered == Some(focused) {
            return;
        }
        self.delivered = Some(focused);
        #[cfg(test)]
        self.history.push(focused);
    }
}
