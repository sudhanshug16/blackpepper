use super::{ClientMode, ClientState};
use crate::terminal::InputModes;

impl ClientState {
    pub fn update_input_modes(&mut self) {
        let desired = match self.mode {
            ClientMode::Manage => InputModes::manage_interface(),
            ClientMode::Work => self
                .active_terminal_mut()
                .map(|terminal| terminal.input_modes())
                .unwrap_or_default(),
            // OpenSSH owns authentication input. It must receive ordinary
            // terminal bytes, never mouse/application modes inherited from a
            // previously attached Zellij client.
            ClientMode::Authenticate => InputModes::default(),
        };
        let bytes = desired.diff_bytes(&self.input_modes_applied);
        if !bytes.is_empty() {
            self.pending_input_mode_bytes.extend(bytes);
            self.input_modes_applied = desired;
        }
    }

    pub fn reset_input_modes(&mut self) {
        let desired = InputModes::default();
        self.pending_input_mode_bytes
            .extend(desired.diff_bytes(&self.input_modes_applied));
        self.input_modes_applied = desired;
    }
}
