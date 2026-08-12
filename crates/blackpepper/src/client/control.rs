//! Interactive input dispatch. Terminal bytes stay opaque in work mode.

mod manage;

use super::{ClientEvent, ClientMode, ClientState};
use crate::client::runtime::ClientRuntime;
use crate::client::runtime::{DeferredHostAction, DurableActionQueue};
use crate::input::MatchedChord;
use termwiz::input::InputEvent;

pub(super) fn handle_event(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    event: ClientEvent,
) {
    match event {
        ClientEvent::RawInput(bytes) => handle_raw(state, runtime, &bytes),
        ClientEvent::InputFlush => flush_input(state, runtime),
        ClientEvent::TerminalOutput(workspace_id, attachment_id, bytes) => {
            if let Some(terminal) = state
                .terminals
                .get_mut(&workspace_id)
                .filter(|terminal| terminal.attachment_id() == attachment_id)
            {
                terminal.process_bytes(&bytes);
            }
        }
        ClientEvent::TerminalNotice(workspace_id, attachment_id, message) => {
            if state
                .terminals
                .get(&workspace_id)
                .is_some_and(|terminal| terminal.attachment_id() == attachment_id)
            {
                state.set_transient_output(message, std::time::Duration::from_secs(3));
            }
        }
        ClientEvent::TerminalExited(workspace_id, attachment_id) => {
            if !state
                .terminals
                .get(&workspace_id)
                .is_some_and(|terminal| terminal.attachment_id() == attachment_id)
            {
                return;
            }
            state.terminals.remove(&workspace_id);
            state.connected_clients.remove(&workspace_id);
            let host_id = state.host_for_workspace(workspace_id);
            if state.active_workspace == Some(workspace_id) {
                state.mode = ClientMode::Manage;
                state.set_output("Zellij detached; the workspace session is still running.");
            }
            if let Some(host_id) = host_id {
                queue_durable_state(
                    state,
                    runtime,
                    host_id,
                    "Recording detached workspace session",
                    vec![DeferredHostAction::MarkDetached { workspace_id }],
                );
            }
        }
        ClientEvent::HostAuthenticationOutput(host_id, bytes) => {
            if state.authentication_host == Some(host_id) {
                state.authentication_output.extend_from_slice(&bytes);
                const MAX_AUTH_BYTES: usize = 256 * 1024;
                if state.authentication_output.len() > MAX_AUTH_BYTES {
                    let excess = state.authentication_output.len() - MAX_AUTH_BYTES;
                    state.authentication_output.drain(..excess);
                }
            }
        }
        ClientEvent::BlockerTransition(instance_id, transition) => {
            if runtime.blocker_watcher_is_current(transition.run_id, instance_id) {
                apply_blocker_transition(state, instance_id, transition);
            }
        }
        ClientEvent::BlockerWatcherExited(run_id, instance_id) => {
            if !runtime.stop_blocker_watcher_if_current(run_id, instance_id) {
                return;
            }
            let mut workspace_id = None;
            for (candidate, runs) in &mut state.agent_runs {
                if let Some(run) = runs.iter_mut().find(|run| run.run_id == run_id) {
                    run.blocker = None;
                    run.blocker_watcher_instance = None;
                    run.blocker_sequence = 0;
                    run.blocker_observed_at_ms = None;
                    workspace_id = Some(*candidate);
                    break;
                }
            }
            if let Some(workspace_id) = workspace_id {
                state.refresh_workspace_status(workspace_id);
                state.set_output(
                    "Screen-based needs-input detection stopped; provider status remains active.",
                );
            }
        }
        ClientEvent::BackgroundResult { operation, result } => match result {
            Ok(message) => state.set_output(format!("{operation}: {message}")),
            Err(message) => state.set_output(format!("{operation} failed: {message}")),
        },
        ClientEvent::PeriodicRefreshComplete { .. }
        | ClientEvent::PeriodicForwardCleanupComplete { .. }
        | ClientEvent::ConnectionRestoreProgress { .. }
        | ClientEvent::ConnectionRestoreComplete { .. }
        | ClientEvent::HostOperationProgress { .. }
        | ClientEvent::HostOperationComplete { .. }
        | ClientEvent::ManualRefreshRequested => {
            debug_assert!(false, "background runtime events are handled by the runner")
        }
        ClientEvent::Resize => {}
    }
    state.update_input_modes();
}

fn apply_blocker_transition(
    state: &mut ClientState,
    instance_id: uuid::Uuid,
    transition: crate::status_monitor::BlockerTransition,
) {
    if state.host_for_workspace(transition.workspace_id) != Some(transition.host_id) {
        return;
    }
    let Some(run) = state
        .agent_runs
        .get_mut(&transition.workspace_id)
        .and_then(|runs| runs.iter_mut().find(|run| run.run_id == transition.run_id))
    else {
        return;
    };
    run.begin_blocker_watcher(instance_id);
    if run.pane_id != transition.pane_id
        || run.provider != transition.provider
        || transition.sequence <= run.blocker_sequence
        || run
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.last_event_at_ms)
            .is_some_and(|event_at| transition.observed_at_ms < event_at)
        || run
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.state == crate::agent_status::AgentState::Exited)
        || run.healthy_snapshot_supersedes_blocker(transition.observed_at_ms)
    {
        return;
    }
    run.blocker_sequence = transition.sequence;
    run.blocker_observed_at_ms = Some(transition.observed_at_ms);
    run.blocker = match transition.state {
        crate::status_monitor::BlockerChange::NeedsInput {
            rule_id,
            confidence,
            priority,
        } => Some(crate::agent_status::BlockerExplain {
            provider: transition.provider,
            manifest_version: transition.manifest_version,
            rule_id,
            confidence,
            priority,
        }),
        crate::status_monitor::BlockerChange::Cleared => None,
    };
    state.refresh_workspace_status(transition.workspace_id);
    state.rebuild_tree();
}

fn handle_raw(state: &mut ClientState, runtime: &mut ClientRuntime, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    if state.mode == ClientMode::Authenticate {
        if let Some(host_id) = state.authentication_host {
            if let Err(error) = runtime.send_authentication_input(host_id, bytes) {
                state.set_output(error);
                state.mode = ClientMode::Manage;
            }
        }
        return;
    }

    let (filtered, matched) = state.input_decoder.consume_work_bytes(bytes);
    if state.mode == ClientMode::Work {
        if let Some(workspace_id) = state.active_workspace {
            state.mark_workspace_completions_seen(workspace_id);
        }
        let interrupted = filtered.contains(&0x03);
        let terminal_area = state.terminal_area;
        if let Some(terminal) = state.active_terminal_mut() {
            if let Err(error) = terminal.write(&filtered, terminal_area) {
                state.set_output(error.to_string());
                state.mode = ClientMode::Manage;
            }
        }
        if interrupted {
            mark_active_workspace_interrupted(state, runtime);
        }
        handle_matched_chord(state, runtime, matched);
        return;
    }

    for event in state.input_decoder.parse_manage_vec(&filtered, true) {
        handle_manage_event(state, runtime, event);
    }
    handle_matched_chord(state, runtime, matched);
}

fn flush_input(state: &mut ClientState, runtime: &mut ClientRuntime) {
    if state.mode == ClientMode::Authenticate {
        return;
    }
    if state.mode == ClientMode::Work {
        let bytes = state.input_decoder.flush_work();
        let interrupted = bytes.contains(&0x03);
        let terminal_area = state.terminal_area;
        if let Some(terminal) = state.active_terminal_mut() {
            if let Err(error) = terminal.write(&bytes, terminal_area) {
                state.set_output(format!("Terminal input could not be delivered: {error}"));
                state.mode = ClientMode::Manage;
            }
        }
        if interrupted {
            mark_active_workspace_interrupted(state, runtime);
        }
        return;
    }
    let buffered = state.input_decoder.flush_work();
    let mut events = state.input_decoder.parse_manage_vec(&buffered, false);
    events.extend(state.input_decoder.flush_manage_vec());
    for event in events {
        handle_manage_event(state, runtime, event);
    }
}

fn mark_active_workspace_interrupted(state: &mut ClientState, runtime: &mut ClientRuntime) {
    let Some(workspace_id) = state.active_workspace else {
        return;
    };
    // Zellij does not expose the focused pane for this specific client. Mark
    // every run in the active workspace conservatively instead of showing a
    // false completion for whichever pane received Ctrl-C.
    let host_id = state.host_for_workspace(workspace_id);
    let run_ids =
        if let Some(runs) = state.agent_runs.get_mut(&workspace_id) {
            let mut run_ids = Vec::new();
            for run in runs {
                if run.snapshot.as_ref().is_none_or(|snapshot| {
                    snapshot.state != crate::agent_status::AgentState::Exited
                }) {
                    run.mark_interrupted();
                    run_ids.push(run.run_id);
                }
            }
            state.refresh_workspace_status(workspace_id);
            run_ids
        } else {
            Vec::new()
        };
    let Some(host_id) = host_id else {
        return;
    };
    if !run_ids.is_empty() {
        queue_durable_state(
            state,
            runtime,
            host_id,
            "Persisting Ctrl-C agent status",
            vec![DeferredHostAction::MarkAgentsUnknown {
                workspace_id,
                run_ids,
            }],
        );
    }
}

fn queue_durable_state(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    host_id: crate::core::HostId,
    label: &str,
    actions: Vec<DeferredHostAction>,
) {
    match runtime.queue_durable_actions(host_id, label, actions, state.event_tx.clone()) {
        Ok(DurableActionQueue::Started { token, label }) => {
            state.host_operations.insert(host_id, (token, label));
        }
        Ok(DurableActionQueue::Queued { behind }) => state.set_output(format!(
            "Terminal input was handled; durable status is queued behind {behind}."
        )),
        Err(error) => {
            let workspace_ids = state
                .agent_runs
                .keys()
                .copied()
                .filter(|workspace_id| state.host_for_workspace(*workspace_id) == Some(host_id))
                .collect::<Vec<_>>();
            for workspace_id in workspace_ids {
                if let Some(runs) = state.agent_runs.get_mut(&workspace_id) {
                    for run in runs
                        .iter_mut()
                        .filter(|run| run.interrupted_after_sequence.is_some())
                    {
                        run.mark_snapshot_error(format!(
                            "Ctrl-C status is not yet durable: {error}"
                        ));
                    }
                }
                state.refresh_workspace_status(workspace_id);
            }
            state.set_detail("Terminal state persistence error", error);
            state.set_output(
                "Terminal input was handled, but durable status needs attention; review the error panel.",
            );
        }
    }
}

fn handle_matched_chord(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    matched: MatchedChord,
) {
    match matched {
        MatchedChord::Toggle => {
            state.mode = if state.mode == ClientMode::Work {
                ClientMode::Manage
            } else if state.active_workspace.is_some() {
                if let Some(workspace_id) = state.active_workspace {
                    state.mark_workspace_completions_seen(workspace_id);
                }
                ClientMode::Work
            } else {
                ClientMode::Manage
            };
        }
        MatchedChord::WorkspaceOverlay | MatchedChord::Switch => {
            state.mode = ClientMode::Manage;
            state.select_next(1);
            if matched == MatchedChord::Switch {
                attach_selected(state, runtime);
            }
        }
        MatchedChord::None => {}
    }
}

fn handle_manage_event(state: &mut ClientState, runtime: &mut ClientRuntime, event: InputEvent) {
    match event {
        InputEvent::Key(key) => manage::handle_key(state, runtime, key),
        InputEvent::Mouse(mouse) => manage::handle_mouse(state, runtime, mouse),
        InputEvent::Paste(value) if state.command_active => state.command_input.push_str(&value),
        _ => {}
    }
}

pub(super) use manage::{apply_attachment, attach_selected};
