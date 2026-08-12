use super::ClientRuntime;
use crate::agent_status::Provider;
use crate::client::ClientEvent;
use crate::core::{AgentRunId, HostId, PaneId, WorkspaceId};
use crate::transport::{HostCommand, RunningCommand};
use std::io::{self, BufRead, BufReader, Read};
use std::process::ChildStdin;
use std::sync::mpsc::Sender;
use std::time::Duration;

const MAX_TRANSITION_BYTES: usize = 64 * 1024;

pub(super) struct BlockerWatcher {
    pub host_id: HostId,
    instance_id: uuid::Uuid,
    stdin: Option<ChildStdin>,
    child: Option<RunningCommand>,
}

impl BlockerWatcher {
    fn stop(&mut self) {
        self.stdin.take();
        if let Some(child) = self.child.as_mut() {
            for _ in 0..20 {
                if child.try_wait().ok().flatten().is_some() {
                    if let Some(child) = self.child.take() {
                        let _ = child.wait_with_output();
                    }
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        if let Some(child) = self.child.take() {
            let _ = child.cancel();
        }
    }
}

impl Drop for BlockerWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

impl ClientRuntime {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_blocker_watcher(
        &mut self,
        host_id: HostId,
        workspace_id: WorkspaceId,
        run_id: AgentRunId,
        pane_id: PaneId,
        provider: Provider,
        session: &str,
        zellij_version: &str,
        zellij_pane_id: &str,
        after_sequence: u64,
        sender: Sender<ClientEvent>,
    ) -> Result<(), String> {
        let helper = self.helper_path(host_id)?;
        let command = HostCommand::new(helper).args([
            "watch-blockers".to_string(),
            "--workspace-id".to_string(),
            workspace_id.to_string(),
            "--run-id".to_string(),
            run_id.to_string(),
            "--pane-id".to_string(),
            pane_id.to_string(),
            "--provider".to_string(),
            provider.to_string(),
            "--session".to_string(),
            session.to_string(),
            "--zellij-version".to_string(),
            zellij_version.to_string(),
            "--zellij-pane-id".to_string(),
            zellij_pane_id.to_string(),
            "--after-sequence".to_string(),
            after_sequence.to_string(),
        ]);
        let mut child = self.spawn_fail_closed_background_exec_with_stdin(host_id, &command)?;
        let stdin = child
            .take_stdin()
            .ok_or_else(|| "Blocker watcher has no cancellation channel.".to_string())?;
        let stdout = child
            .take_stdout()
            .ok_or_else(|| "Blocker watcher has no transition stream.".to_string())?;
        if let Some(mut stderr) = child.take_stderr() {
            std::thread::spawn(move || {
                let _ = io::copy(&mut stderr, &mut io::sink());
            });
        }
        let instance_id = uuid::Uuid::new_v4();
        std::thread::spawn(move || read_transitions(stdout, run_id, instance_id, sender));
        self.blocker_watchers.insert(
            run_id,
            BlockerWatcher {
                host_id,
                instance_id,
                stdin: Some(stdin),
                child: Some(child),
            },
        );
        Ok(())
    }

    pub(super) fn stop_blocker_watchers(&mut self, host_id: HostId) {
        self.blocker_watchers
            .retain(|_, watcher| watcher.host_id != host_id);
    }

    pub(crate) fn stop_blocker_watcher(&mut self, run_id: AgentRunId) {
        self.blocker_watchers.remove(&run_id);
    }

    pub(crate) fn stop_blocker_watcher_if_current(
        &mut self,
        run_id: AgentRunId,
        instance_id: uuid::Uuid,
    ) -> bool {
        if !self
            .blocker_watchers
            .get(&run_id)
            .is_some_and(|watcher| watcher.instance_id == instance_id)
        {
            return false;
        }
        self.blocker_watchers.remove(&run_id);
        true
    }

    pub(crate) fn blocker_watcher_is_current(
        &self,
        run_id: AgentRunId,
        instance_id: uuid::Uuid,
    ) -> bool {
        self.blocker_watchers
            .get(&run_id)
            .is_some_and(|watcher| watcher.instance_id == instance_id)
    }
}

fn read_transitions(
    stdout: impl Read,
    run_id: AgentRunId,
    instance_id: uuid::Uuid,
    sender: Sender<ClientEvent>,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        match read_bounded_line(&mut reader) {
            Ok(Some(line)) if line.is_empty() => continue,
            Ok(Some(line)) => {
                let Ok(transition) = serde_json::from_slice(&line) else {
                    continue;
                };
                if sender
                    .send(ClientEvent::BlockerTransition(instance_id, transition))
                    .is_err()
                {
                    return;
                }
            }
            Ok(None) | Err(_) => break,
        }
    }
    let _ = sender.send(ClientEvent::BlockerWatcherExited(run_id, instance_id));
}

fn read_bounded_line(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    let mut too_long = false;
    let mut saw_bytes = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if !saw_bytes || too_long {
                Ok(None)
            } else {
                Ok(Some(line))
            };
        }
        saw_bytes = true;
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        if !too_long {
            if line.len() + consumed > MAX_TRANSITION_BYTES {
                too_long = true;
                line.clear();
            } else {
                line.extend_from_slice(&available[..consumed]);
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            while matches!(line.last(), Some(b'\n' | b'\r')) {
                line.pop();
            }
            if too_long {
                too_long = false;
                saw_bytes = false;
                continue;
            }
            return Ok(Some(line));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::mpsc;

    #[test]
    fn bounded_reader_discards_an_oversize_record_without_echoing_it() {
        let input = format!(
            "{}\n{{\"safe\":true}}\n",
            "x".repeat(MAX_TRANSITION_BYTES + 1)
        );
        let mut reader = BufReader::new(input.as_bytes());
        assert_eq!(
            read_bounded_line(&mut reader).unwrap(),
            Some(br#"{"safe":true}"#.to_vec())
        );
    }

    #[test]
    fn watcher_exit_identifies_the_exact_reader_instance() {
        let run_id = AgentRunId::new();
        let instance_id = uuid::Uuid::new_v4();
        let (sender, receiver) = mpsc::channel();

        read_transitions(Cursor::new(Vec::<u8>::new()), run_id, instance_id, sender);

        assert!(matches!(
            receiver.recv().unwrap(),
            ClientEvent::BlockerWatcherExited(run, instance)
                if run == run_id && instance == instance_id
        ));
    }

    #[test]
    fn watcher_transitions_identify_the_exact_reader_instance() {
        let run_id = AgentRunId::new();
        let pane_id = PaneId::new();
        let instance_id = uuid::Uuid::new_v4();
        let transition = crate::status_monitor::BlockerTransition {
            host_id: HostId::new(),
            workspace_id: WorkspaceId::new(),
            run_id,
            pane_id,
            provider: Provider::Codex,
            sequence: 1,
            observed_at_ms: 1,
            source: crate::status_monitor::BlockerSource::ZellijViewport,
            manifest_version: "test".to_owned(),
            state: crate::status_monitor::BlockerChange::Cleared,
        };
        let line = format!("{}\n", serde_json::to_string(&transition).unwrap());
        let (sender, receiver) = mpsc::channel();

        read_transitions(Cursor::new(line), run_id, instance_id, sender);

        assert!(matches!(
            receiver.recv().unwrap(),
            ClientEvent::BlockerTransition(instance, observed)
                if instance == instance_id && observed == transition
        ));
    }
}
