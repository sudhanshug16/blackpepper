//! Raw input decoding and toggle detection.

mod logger;
mod matcher;

use logger::InputLogger;
use matcher::{toggle_sequences, ToggleMatcher};
use termwiz::input::{InputEvent, InputParser};

use crate::keymap::KeyChord;

/// Which chord was matched in work mode input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchedChord {
    None,
    Toggle,
    WorkspaceOverlay,
    Switch,
}

pub struct InputDecoder {
    parser: InputParser,
    toggle_matcher: ToggleMatcher,
    overlay_matcher: ToggleMatcher,
    switch_matcher: ToggleMatcher,
    logger: InputLogger,
}

impl InputDecoder {
    pub fn new(
        toggle_chord: Option<KeyChord>,
        overlay_chord: Option<KeyChord>,
        switch_chord: Option<KeyChord>,
    ) -> Self {
        Self {
            parser: InputParser::new(),
            toggle_matcher: ToggleMatcher::new(toggle_sequences(toggle_chord.as_ref())),
            overlay_matcher: ToggleMatcher::new(toggle_sequences(overlay_chord.as_ref())),
            switch_matcher: ToggleMatcher::new(toggle_sequences(switch_chord.as_ref())),
            logger: InputLogger::new(),
        }
    }

    pub fn update_chords(
        &mut self,
        toggle_chord: Option<KeyChord>,
        overlay_chord: Option<KeyChord>,
        switch_chord: Option<KeyChord>,
    ) {
        self.toggle_matcher
            .update_sequences(toggle_sequences(toggle_chord.as_ref()));
        self.overlay_matcher
            .update_sequences(toggle_sequences(overlay_chord.as_ref()));
        self.switch_matcher
            .update_sequences(toggle_sequences(switch_chord.as_ref()));
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

    pub fn consume_work_bytes(&mut self, bytes: &[u8]) -> (Vec<u8>, MatchedChord) {
        self.logger.log_raw(bytes);

        // Check toggle chord first
        let (out, toggled, matched) = self.toggle_matcher.feed(bytes);
        if toggled {
            self.logger.log_toggle(&matched);
            // Also feed to other matchers to keep them in sync (discard result)
            let _ = self.overlay_matcher.feed(bytes);
            let _ = self.switch_matcher.feed(bytes);
            return (out, MatchedChord::Toggle);
        }

        // Check workspace overlay chord
        let (out2, opened, matched2) = self.overlay_matcher.feed(bytes);
        if opened {
            self.logger.log_toggle(&matched2);
            let _ = self.switch_matcher.feed(bytes);
            return (out2, MatchedChord::WorkspaceOverlay);
        }

        // Check switch chord
        let (out3, switched, matched3) = self.switch_matcher.feed(bytes);
        if switched {
            self.logger.log_toggle(&matched3);
            return (out3, MatchedChord::Switch);
        }

        // Neither matched - return the output from toggle_matcher
        // (both matchers should produce same passthrough output)
        (out, MatchedChord::None)
    }

    pub fn flush_work(&mut self) -> Vec<u8> {
        let t = self.toggle_matcher.flush();
        let _ = self.overlay_matcher.flush();
        let _ = self.switch_matcher.flush();
        t
    }
}

#[cfg(test)]
mod tests;
