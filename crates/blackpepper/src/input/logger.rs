use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use termwiz::input::InputEvent;

#[derive(Default)]
pub(super) struct InputLogger {
    enabled: bool,
    path: PathBuf,
    file: Option<std::fs::File>,
}

impl InputLogger {
    pub(super) fn new() -> Self {
        let enabled = std::env::var("BLACKPEPPER_DEBUG_INPUT")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        Self {
            enabled,
            path: PathBuf::from("/tmp/blackpepper-input.log"),
            file: None,
        }
    }

    pub(super) fn log_raw(&mut self, bytes: &[u8]) {
        if !self.enabled || bytes.is_empty() {
            return;
        }
        let mut line = String::from("raw:");
        for byte in bytes {
            line.push(' ');
            line.push_str(&format!("{:02x}", byte));
        }
        self.write_line(&line);
    }

    pub(super) fn log_event(&mut self, event: &InputEvent) {
        if !self.enabled {
            return;
        }
        self.write_line(&format!("event: {:?}", event));
    }

    pub(super) fn log_toggle(&mut self, matched: &[u8]) {
        if !self.enabled || matched.is_empty() {
            return;
        }
        let mut line = String::from("toggle:");
        for byte in matched {
            line.push(' ');
            line.push_str(&format!("{:02x}", byte));
        }
        self.write_line(&line);
    }

    fn write_line(&mut self, line: &str) {
        if !self.enabled {
            return;
        }
        if self.file.is_none() {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path);
            match file {
                Ok(file) => {
                    self.file = Some(file);
                }
                Err(_) => {
                    self.enabled = false;
                    return;
                }
            }
        }
        if let Some(file) = self.file.as_mut() {
            let _ = writeln!(file, "{}", line);
        }
    }
}
