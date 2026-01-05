use crossterm::event::{KeyEvent, MouseEvent};

#[derive(Debug)]
pub enum AppEvent {
    Input(KeyEvent),
    PtyOutput(u64, Vec<u8>),
    PtyExit(u64),
    Mouse(MouseEvent),
    #[allow(dead_code)]
    Resize(u16, u16),
    CommandDone {
        name: String,
        args: Vec<String>,
        result: crate::commands::CommandResult,
    },
}
