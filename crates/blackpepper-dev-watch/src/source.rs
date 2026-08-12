use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Component, Path};
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(crate) enum SourceEvent {
    Changed,
    Failed(String),
}

pub(crate) struct SourceWatch {
    receiver: Receiver<SourceEvent>,
    _watcher: RecommendedWatcher,
}

impl SourceWatch {
    pub fn start(root: &Path) -> Result<Self, String> {
        let watched_root = root.to_path_buf();
        let (sender, receiver) = mpsc::channel();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<Event>| match result {
                Ok(event) if event_is_relevant(&watched_root, &event) => {
                    let _ = sender.send(SourceEvent::Changed);
                }
                Ok(_) => {}
                Err(error) => {
                    let _ = sender.send(SourceEvent::Failed(error.to_string()));
                }
            })
            .map_err(|error| format!("could not create source watcher: {error}"))?;
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|error| format!("could not watch {}: {error}", root.display()))?;
        Ok(Self {
            receiver,
            _watcher: watcher,
        })
    }

    pub fn try_recv(&self) -> Result<Option<SourceEvent>, String> {
        match self.receiver.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                Err("source watcher stopped unexpectedly".to_owned())
            }
        }
    }
}

fn event_is_relevant(root: &Path, event: &Event) -> bool {
    if !matches!(
        event.kind,
        EventKind::Any | EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        return false;
    }
    event.paths.iter().any(|path| path_is_relevant(root, path))
}

fn path_is_relevant(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    if relative.components().any(|component| {
        matches!(component, Component::Normal(value) if value == ".git" || value == "target" || value == "__pycache__")
    }) {
        return false;
    }
    let text = relative.to_string_lossy();
    text == "Cargo.toml"
        || text == "Cargo.lock"
        || text == "rust-toolchain"
        || text == "rust-toolchain.toml"
        || text == ".cargo"
        || text.starts_with(".cargo/")
        || text == "crates/blackpepper"
        || text.starts_with("crates/blackpepper/")
}

#[cfg(test)]
mod tests {
    use super::path_is_relevant;
    use std::path::Path;

    #[test]
    fn source_filter_keeps_build_inputs_and_ignores_outputs() {
        let root = Path::new("/repo");
        assert!(path_is_relevant(
            root,
            Path::new("/repo/crates/blackpepper/src/main.rs")
        ));
        assert!(path_is_relevant(root, Path::new("/repo/Cargo.lock")));
        assert!(!path_is_relevant(root, Path::new("/repo/target/debug/bp")));
        assert!(!path_is_relevant(root, Path::new("/repo/README.md")));
    }
}
