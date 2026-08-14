use super::{ClientMode, ClientState};
use crate::terminal::InputModes;

impl ClientState {
    pub fn update_input_modes(&mut self) {
        let displayed = (self.mode == ClientMode::Work)
            .then_some(self.active_workspace)
            .flatten();
        let outer_focused = self.outer_focus.focused();
        for (workspace_id, terminal) in &mut self.terminals {
            terminal.sync_visibility_focus(displayed == Some(*workspace_id), outer_focused);
        }

        let desired = match self.mode {
            ClientMode::Manage => InputModes::manage_interface(),
            ClientMode::Work => self
                .active_terminal_mut()
                .map(|terminal| terminal.input_modes())
                .unwrap_or_default()
                .with_shell_pointer_capture(),
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
