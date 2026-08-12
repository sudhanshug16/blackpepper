use super::{build_tree, ClientEvent, DisplayStatus, EmbeddedTerminal, HostConnection, HostNode};
use crate::client_config::ClientConfig;
mod agent_run;
mod input_modes;
mod view;

use crate::core::{HostAgentRun, HostId, RegistrySnapshot, WorkspaceId};
use crate::input::InputDecoder;
use crate::keymap::{parse_key_chord, KeyChord};
use crate::ports::{ForwardState, PortSnapshot};
use crate::terminal::InputModes;
use ratatui::layout::Rect;
use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

pub use agent_run::AgentRunView;
pub use view::{
    ClientMode, DetailView, HelpView, PendingWorktrunkApproval, PortClickTarget, WorkspacePicker,
};

pub struct ClientState {
    pub mode: ClientMode,
    pub config: ClientConfig,
    pub snapshot: RegistrySnapshot,
    pub tree: Vec<HostNode>,
    pub connections: BTreeMap<HostId, HostConnection>,
    pub statuses: BTreeMap<WorkspaceId, DisplayStatus>,
    pub agent_runs: BTreeMap<WorkspaceId, Vec<AgentRunView>>,
    pub selected_host: Option<HostId>,
    pub selected_workspace: Option<WorkspaceId>,
    pub active_workspace: Option<WorkspaceId>,
    pub terminals: HashMap<WorkspaceId, EmbeddedTerminal>,
    pub ports: BTreeMap<HostId, PortSnapshot>,
    pub show_all_host_ports: bool,
    pub forwards: Vec<ForwardState>,
    /// Scroll offset for the compact ports panel. The full `:ports` detail
    /// remains independently scrollable.
    pub ports_scroll: u16,
    pub ports_area: Option<Rect>,
    pub port_click_targets: Vec<PortClickTarget>,
    pub connected_clients: BTreeMap<WorkspaceId, usize>,
    /// Host-computed repository and tab context, keyed by workspace.
    pub overviews: BTreeMap<WorkspaceId, crate::core::WorkspaceOverview>,
    /// Explicit host work remains visible even if terminal output replaces
    /// the transient footer message while its worker is running.
    pub host_operations: BTreeMap<HostId, (uuid::Uuid, String)>,
    pub authentication_host: Option<HostId>,
    pub authentication_output: Vec<u8>,
    pub command_active: bool,
    pub command_input: String,
    /// Highlighted completion candidate, as an index into the grounded list
    /// rebuilt on every keystroke.
    pub command_selection: usize,
    /// Open workspace picker, if any.
    pub picker: Option<WorkspacePicker>,
    /// Open grouped help, if any.
    pub help: Option<HelpView>,
    pub pending_approval: Option<PendingWorktrunkApproval>,
    pub approval_scroll: u16,
    pub detail: Option<DetailView>,
    pub detail_scroll: u16,
    pub output: Option<String>,
    transient_output: Option<(String, Instant)>,
    pub should_quit: bool,
    pub terminal_area: Option<Rect>,
    pub event_tx: Sender<ClientEvent>,
    pub input_decoder: InputDecoder,
    pub toggle_chord: Option<KeyChord>,
    pub switch_chord: Option<KeyChord>,
    pub workspace_overlay_chord: Option<KeyChord>,
    pub input_modes_applied: InputModes,
    pub pending_input_mode_bytes: Vec<u8>,
    /// Client start instant. Animation phase is derived from this so restarting
    /// the client, not the passage of a frame counter, is what resets motion.
    started: Instant,
}

/// Compact age of the most recent provider event, in the same two-character
/// vocabulary the design uses (`8s`, `2m`, `3h`, `4d`). Returns `None` when the
/// host clock and the client clock disagree enough that any number would be a
/// guess.
fn elapsed_label(event_at_ms: u64) -> Option<String> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    let seconds = u128::from(event_at_ms)
        .le(&now_ms)
        .then(|| (now_ms - u128::from(event_at_ms)) / 1000)?;
    Some(match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m", seconds / 60),
        3600..=86_399 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    })
}

impl ClientState {
    pub fn new(
        config: ClientConfig,
        snapshot: RegistrySnapshot,
        event_tx: Sender<ClientEvent>,
    ) -> Self {
        let toggle_chord = parse_key_chord(&config.keymap.toggle_mode);
        let switch_chord = parse_key_chord(&config.keymap.switch_workspace);
        let workspace_overlay_chord = parse_key_chord(&config.keymap.workspace_overlay);
        let input_decoder = InputDecoder::new(
            toggle_chord.clone(),
            workspace_overlay_chord.clone(),
            switch_chord.clone(),
        );
        let mut state = Self {
            mode: ClientMode::Manage,
            config,
            snapshot,
            tree: Vec::new(),
            connections: BTreeMap::new(),
            statuses: BTreeMap::new(),
            agent_runs: BTreeMap::new(),
            selected_host: None,
            selected_workspace: None,
            active_workspace: None,
            terminals: HashMap::new(),
            ports: BTreeMap::new(),
            show_all_host_ports: false,
            forwards: Vec::new(),
            ports_scroll: 0,
            ports_area: None,
            port_click_targets: Vec::new(),
            connected_clients: BTreeMap::new(),
            overviews: BTreeMap::new(),
            host_operations: BTreeMap::new(),
            authentication_host: None,
            authentication_output: Vec::new(),
            command_active: false,
            command_input: String::new(),
            command_selection: 0,
            picker: None,
            help: None,
            pending_approval: None,
            approval_scroll: 0,
            detail: None,
            detail_scroll: 0,
            output: None,
            transient_output: None,
            should_quit: false,
            terminal_area: None,
            event_tx,
            input_decoder,
            toggle_chord,
            switch_chord,
            workspace_overlay_chord,
            input_modes_applied: InputModes::default(),
            pending_input_mode_bytes: Vec::new(),
            started: Instant::now(),
        };
        state.rebuild_tree();
        state.selected_workspace = state.workspace_ids().first().copied();
        state.selected_host = state
            .selected_workspace
            .and_then(|id| state.host_for_workspace(id))
            .or_else(|| state.snapshot.hosts.first().map(|host| host.id));
        state
    }

    pub fn rebuild_tree(&mut self) {
        self.tree = build_tree(&self.snapshot, &self.connections, &self.statuses);
        if self
            .selected_workspace
            .is_some_and(|id| !self.workspace_ids().contains(&id))
        {
            self.selected_workspace = self.workspace_ids().first().copied();
        }
    }

    pub fn workspace_ids(&self) -> Vec<WorkspaceId> {
        self.tree
            .iter()
            .flat_map(|host| &host.repositories)
            .flat_map(|repo| &repo.workspaces)
            .map(|workspace| workspace.id)
            .collect()
    }

    pub fn select_next(&mut self, direction: i32) {
        let ids = self.workspace_ids();
        if ids.is_empty() {
            self.selected_workspace = None;
            return;
        }
        let current = self
            .selected_workspace
            .and_then(|selected| ids.iter().position(|id| *id == selected))
            .unwrap_or(0);
        let next = (current as i32 + direction).rem_euclid(ids.len() as i32) as usize;
        self.selected_workspace = Some(ids[next]);
        self.selected_host = self.host_for_workspace(ids[next]);
    }

    pub fn active_terminal_mut(&mut self) -> Option<&mut EmbeddedTerminal> {
        self.terminals.get_mut(&self.active_workspace?)
    }

    pub fn active_workspace_record(&self) -> Option<&crate::core::WorkspaceRecord> {
        let id = self.active_workspace?;
        self.snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.id == id)
    }

    pub fn host_for_workspace(&self, workspace_id: WorkspaceId) -> Option<HostId> {
        self.snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .map(|workspace| workspace.host_id)
    }

    pub fn set_output(&mut self, message: impl Into<String>) {
        self.transient_output = None;
        self.output = Some(message.into());
    }

    pub(in crate::client) fn set_transient_output(
        &mut self,
        message: impl Into<String>,
        duration: Duration,
    ) {
        self.transient_output = Some((message.into(), Instant::now() + duration));
    }

    pub(in crate::client) fn expire_transient_output(&mut self) -> bool {
        if self
            .transient_output
            .as_ref()
            .is_some_and(|(_, deadline)| Instant::now() >= *deadline)
        {
            self.transient_output = None;
            return true;
        }
        false
    }

    pub(in crate::client) fn visible_output(&self) -> Option<&str> {
        self.transient_output
            .as_ref()
            .map(|(message, _)| message.as_str())
            .or(self.output.as_deref())
    }

    pub(in crate::client) fn clear_output(&mut self) {
        self.transient_output = None;
        self.output = None;
    }

    pub fn set_detail(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.detail = Some(DetailView {
            title: title.into(),
            body: body.into(),
        });
        self.detail_scroll = 0;
    }

    pub fn close_detail(&mut self) -> bool {
        let was_open = self.detail.take().is_some();
        if was_open {
            self.detail_scroll = 0;
            self.clear_output();
        }
        was_open
    }

    /// A completion badge is per client. It becomes seen only when this client
    /// explicitly returns to or interacts with that workspace, never because a
    /// diagnostic command happened to inspect it.
    pub fn mark_workspace_completions_seen(&mut self, workspace_id: WorkspaceId) -> bool {
        let mut changed = false;
        if let Some(runs) = self.agent_runs.get_mut(&workspace_id) {
            for run in runs {
                if let Some(snapshot) = &run.snapshot {
                    if run.seen_completion_revision < snapshot.completion_revision {
                        run.seen_completion_revision = snapshot.completion_revision;
                        changed = true;
                    }
                }
            }
        }
        if changed {
            self.refresh_workspace_status(workspace_id);
            self.rebuild_tree();
        }
        changed
    }

    /// Workspaces matching the open picker's filter, in sidebar order, each
    /// with the host it lives on. Filtering is a plain case-insensitive
    /// substring match over the label so what you type is what you get.
    pub fn picker_matches(&self) -> Vec<(WorkspaceId, String, String, DisplayStatus)> {
        let Some(picker) = self.picker.as_ref() else {
            return Vec::new();
        };
        let filter = picker.filter.to_lowercase();
        self.tree
            .iter()
            .flat_map(|host| {
                host.repositories
                    .iter()
                    .flat_map(|repository| &repository.workspaces)
                    .map(|workspace| {
                        (
                            workspace.id,
                            workspace.label.clone(),
                            host.label.clone(),
                            workspace.status,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|(_, label, _, _)| filter.is_empty() || label.to_lowercase().contains(&filter))
            .collect()
    }

    pub fn open_picker(&mut self) {
        let selected = self.selected_workspace;
        self.picker = Some(WorkspacePicker::default());
        // Land on the current workspace so the picker opens where the eye
        // already is, rather than resetting to the top of the list.
        if let Some(index) = selected.and_then(|id| {
            self.picker_matches()
                .iter()
                .position(|(candidate, _, _, _)| *candidate == id)
        }) {
            if let Some(picker) = self.picker.as_mut() {
                picker.selected = index;
            }
        }
    }

    /// Move the picker cursor, clamping to the filtered list rather than
    /// wrapping past its ends.
    pub fn move_picker(&mut self, direction: i32) {
        let count = self.picker_matches().len();
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        if count == 0 {
            picker.selected = 0;
            return;
        }
        let next = (picker.selected as i32 + direction).clamp(0, count as i32 - 1);
        picker.selected = next as usize;
    }

    pub fn picker_choice(&self) -> Option<WorkspaceId> {
        let picker = self.picker.as_ref()?;
        self.picker_matches()
            .get(picker.selected)
            .map(|(id, _, _, _)| *id)
    }

    /// Rotation phase for in-flight indicators, derived from wall time so every
    /// spinner on screen advances together without a per-widget animation
    /// clock.
    pub fn spinner_phase(&self) -> usize {
        (self.started.elapsed().as_millis() / 100) as usize
    }

    /// What the status column says instead of the vocabulary word. A running
    /// agent shows the provider and how long ago it last reported, which is the
    /// only status where the age of the evidence changes what you would do.
    /// Everything else keeps its word.
    pub fn status_detail(
        &self,
        workspace_id: WorkspaceId,
        status: DisplayStatus,
    ) -> Option<String> {
        let (provider, elapsed) = self.running_agent(workspace_id, status)?;
        Some(format!("{provider} {elapsed}"))
    }

    /// The elapsed time without the provider. A narrow list column already
    /// names the workspace, and the status row names the provider, so
    /// repeating it on every row spends columns for nothing.
    pub fn status_elapsed(
        &self,
        workspace_id: WorkspaceId,
        status: DisplayStatus,
    ) -> Option<String> {
        self.running_agent(workspace_id, status)
            .map(|(_, elapsed)| elapsed)
    }

    fn running_agent(
        &self,
        workspace_id: WorkspaceId,
        status: DisplayStatus,
    ) -> Option<(crate::agent_status::Provider, String)> {
        if status != DisplayStatus::Working {
            return None;
        }
        let run = self
            .agent_runs
            .get(&workspace_id)?
            .iter()
            .find(|run| run.display_status() == DisplayStatus::Working)?;
        let elapsed = run
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.last_event_at_ms)
            .and_then(elapsed_label)?;
        Some((run.provider, elapsed))
    }

    pub fn refresh_workspace_status(&mut self, workspace_id: WorkspaceId) {
        let status = self
            .agent_runs
            .get(&workspace_id)
            .into_iter()
            .flatten()
            .map(AgentRunView::display_status)
            .max_by_key(|status| match status {
                DisplayStatus::NeedsInput => 5,
                DisplayStatus::Done => 4,
                DisplayStatus::Working => 3,
                DisplayStatus::Ready => 2,
                DisplayStatus::Unknown => 1,
                DisplayStatus::Exited => 0,
                DisplayStatus::Idle => 0,
            })
            .unwrap_or(DisplayStatus::Idle);
        self.statuses.insert(workspace_id, status);
    }

    /// Merge host-authoritative run descriptors without clearing client-local
    /// completion cursors or blocker observations for runs already displayed.
    pub fn upsert_discovered_agent_runs(
        &mut self,
        host_id: HostId,
        discovered: Vec<HostAgentRun>,
    ) -> usize {
        let mut updated = 0;
        let mut touched = Vec::new();
        for run in discovered {
            if self.host_for_workspace(run.workspace_id) != Some(host_id) {
                continue;
            }
            let workspace_id = run.workspace_id;
            let runs = self.agent_runs.entry(workspace_id).or_default();
            if let Some(existing) = runs
                .iter_mut()
                .find(|existing| existing.run_id == run.run_id)
            {
                existing.pane_id = run.pane_id;
                existing.tab_id = run.binding.tab_id;
                existing.provider = run.provider;
                existing.zellij_pane_id = run.binding.zellij_pane_id;
                existing.apply_snapshot(run.snapshot);
            } else {
                runs.push(AgentRunView::from_host_run(run));
            }
            touched.push(workspace_id);
            updated += 1;
        }
        touched.sort_unstable();
        touched.dedup();
        for workspace_id in touched {
            self.refresh_workspace_status(workspace_id);
        }
        updated
    }
}

#[cfg(test)]
#[path = "state/input_mode_tests.rs"]
mod input_mode_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_status::{
        AgentSnapshot, AgentState, BlockerConfidence, BlockerExplain, IntegrationHealth,
        NeedsInputCapability, Provider,
    };
    use crate::core::{AgentRunId, PaneId};

    #[test]
    fn detail_view_opens_at_the_top_and_closes_cleanly() {
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let mut state = ClientState::new(
            crate::client_config::load_contents(None, None, None).unwrap(),
            RegistrySnapshot::default(),
            event_tx,
        );

        state.set_detail("Command reference", "line one\nline two");
        state.set_output("Command reference open.");
        state.detail_scroll = 7;

        assert_eq!(state.detail.as_ref().unwrap().title, "Command reference");
        assert!(state.close_detail());
        assert!(state.detail.is_none());
        assert_eq!(state.detail_scroll, 0);
        assert!(state.output.is_none());
        assert!(!state.close_detail());
    }

    #[test]
    fn transient_output_temporarily_overlays_without_destroying_persistent_output() {
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let mut state = ClientState::new(
            crate::client_config::load_contents(None, None, None).unwrap(),
            RegistrySnapshot::default(),
            event_tx,
        );

        state.set_output("Persistent warning");
        state.set_transient_output("Copied.", Duration::from_secs(30));
        assert_eq!(state.visible_output(), Some("Copied."));
        assert!(!state.expire_transient_output());

        state.set_transient_output("Copied.", Duration::ZERO);
        assert!(state.expire_transient_output());
        assert_eq!(state.visible_output(), Some("Persistent warning"));
    }

    fn snapshot(run_id: AgentRunId, state: AgentState, sequence: u64) -> AgentSnapshot {
        AgentSnapshot {
            run_id,
            provider: Provider::Codex,
            state,
            revision: sequence,
            completion_revision: u64::from(state == AgentState::Done),
            seen_completion_revision: 0,
            last_event_sequence: Some(sequence),
            last_event_at_ms: Some(sequence),
            integration_health: IntegrationHealth::Healthy {
                integration_version: Some(1),
            },
            needs_input_capability: NeedsInputCapability::ProviderEventsWithOverlay,
            completion_suppressed: false,
        }
    }

    fn run_view() -> AgentRunView {
        let run_id = AgentRunId::new();
        AgentRunView {
            run_id,
            pane_id: PaneId::new(),
            tab_id: 1,
            provider: Provider::Codex,
            zellij_pane_id: "terminal_1".to_string(),
            needs_input_capability: "partial".to_string(),
            snapshot: Some(snapshot(run_id, AgentState::Working, 10)),
            explain: None,
            snapshot_error: None,
            seen_completion_revision: 0,
            blocker: None,
            blocker_watcher_instance: None,
            blocker_sequence: 0,
            blocker_observed_at_ms: None,
            interrupted_after_sequence: None,
        }
    }

    #[test]
    fn failed_status_refresh_cannot_leave_old_state_authoritative() {
        let mut run = run_view();
        assert_eq!(run.display_status(), DisplayStatus::Working);

        assert!(run.mark_snapshot_error("remote snapshot timed out".to_owned()));
        assert!(!run.mark_snapshot_error("remote snapshot timed out".to_owned()));
        assert_eq!(run.display_status(), DisplayStatus::Unknown);
        assert_eq!(
            run.snapshot_error.as_deref(),
            Some("remote snapshot timed out")
        );

        run.apply_snapshot(snapshot(run.run_id, AgentState::Working, 11));
        assert_eq!(run.display_status(), DisplayStatus::Working);
        assert!(run.snapshot_error.is_none());
    }

    #[test]
    fn interrupted_turn_stays_unknown_across_ambiguous_completion() {
        let mut run = run_view();
        run.mark_interrupted();
        run.apply_snapshot(snapshot(run.run_id, AgentState::Done, 11));

        assert_eq!(run.display_status(), DisplayStatus::Unknown);
        assert_eq!(run.interrupted_after_sequence, Some(10));
    }

    #[test]
    fn returning_to_a_workspace_marks_its_completion_seen() {
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let mut state = ClientState::new(
            crate::client_config::load_contents(None, None, None).unwrap(),
            RegistrySnapshot::default(),
            event_tx,
        );
        let workspace_id = WorkspaceId::new();
        let mut run = run_view();
        run.apply_snapshot(snapshot(run.run_id, AgentState::Done, 11));
        state.agent_runs.insert(workspace_id, vec![run]);

        assert!(state.mark_workspace_completions_seen(workspace_id));
        assert_eq!(
            state.agent_runs[&workspace_id][0].display_status(),
            DisplayStatus::Ready
        );
        assert!(!state.mark_workspace_completions_seen(workspace_id));
    }

    #[test]
    fn authoritative_activity_clears_interrupted_state() {
        let mut run = run_view();
        run.mark_interrupted();
        run.apply_snapshot(snapshot(run.run_id, AgentState::Working, 12));

        assert_eq!(run.display_status(), DisplayStatus::Working);
        assert_eq!(run.interrupted_after_sequence, None);
    }

    #[test]
    fn process_exit_clears_a_racing_screen_blocker() {
        let mut run = run_view();
        run.blocker = Some(BlockerExplain {
            provider: Provider::Codex,
            manifest_version: "test".to_owned(),
            rule_id: "approval".to_owned(),
            confidence: BlockerConfidence::High,
            priority: 10,
        });
        run.blocker_observed_at_ms = Some(10);

        run.apply_snapshot(snapshot(run.run_id, AgentState::Exited, 11));

        assert_eq!(run.display_status(), DisplayStatus::Exited);
        assert!(run.blocker.is_none());
        assert_eq!(run.blocker_observed_at_ms, None);
    }

    #[test]
    fn restarted_blocker_watcher_gets_an_independent_sequence_cursor() {
        let mut run = run_view();
        let old_instance = uuid::Uuid::new_v4();
        run.blocker_watcher_instance = Some(old_instance);
        run.blocker_sequence = 99;
        run.blocker_observed_at_ms = Some(99);

        run.begin_blocker_watcher(uuid::Uuid::new_v4());

        assert_eq!(run.blocker_sequence, 0);
        assert_eq!(run.blocker_observed_at_ms, None);
    }

    #[test]
    fn healthy_opencode_snapshot_clears_a_degraded_screen_overlay() {
        let mut run = run_view();
        run.provider = Provider::OpenCode;
        run.blocker = Some(BlockerExplain {
            provider: Provider::OpenCode,
            manifest_version: "test".to_owned(),
            rule_id: "approval".to_owned(),
            confidence: BlockerConfidence::High,
            priority: 10,
        });
        run.blocker_observed_at_ms = Some(9);
        let mut recovered = snapshot(run.run_id, AgentState::Working, 10);
        recovered.provider = Provider::OpenCode;

        run.apply_snapshot(recovered);

        assert_eq!(run.display_status(), DisplayStatus::Working);
        assert!(run.blocker.is_none());
        assert_eq!(run.blocker_observed_at_ms, None);
        assert_eq!(run.displayed_needs_input_capability(), "full");
    }

    #[test]
    fn opencode_needs_input_coverage_tracks_live_health() {
        let mut run = run_view();
        run.provider = Provider::OpenCode;
        let mut stale = snapshot(run.run_id, AgentState::Unknown, 11);
        stale.provider = Provider::OpenCode;
        stale.integration_health = IntegrationHealth::Stale;
        run.apply_snapshot(stale);
        assert_eq!(run.displayed_needs_input_capability(), "partial");

        let mut healthy = snapshot(run.run_id, AgentState::Working, 12);
        healthy.provider = Provider::OpenCode;
        run.apply_snapshot(healthy);
        assert_eq!(run.displayed_needs_input_capability(), "full");

        let mut degraded = snapshot(run.run_id, AgentState::Unknown, 13);
        degraded.provider = Provider::OpenCode;
        degraded.integration_health = IntegrationHealth::Degraded {
            issue: crate::agent_status::IntegrationIssue::TransportUnavailable,
        };
        run.apply_snapshot(degraded);
        assert_eq!(run.displayed_needs_input_capability(), "partial");
    }

    #[test]
    fn opencode_accepts_newer_expiry_but_rejects_overlay_after_recovery() {
        let mut run = run_view();
        run.provider = Provider::OpenCode;
        let mut healthy = snapshot(run.run_id, AgentState::Working, 10);
        healthy.provider = Provider::OpenCode;
        healthy.last_event_at_ms = Some(100);
        run.snapshot = Some(healthy);

        assert!(!run.healthy_snapshot_supersedes_blocker(101));
        assert!(run.healthy_snapshot_supersedes_blocker(100));
        assert!(run.healthy_snapshot_supersedes_blocker(99));

        run.snapshot.as_mut().unwrap().integration_health = IntegrationHealth::Stale;
        assert!(!run.healthy_snapshot_supersedes_blocker(99));
    }

    #[test]
    fn port_click_target_keeps_address_when_ports_match() {
        let workspace_id = WorkspaceId::new();
        let ipv4 = PortClickTarget {
            workspace_id,
            target: crate::ports::RemotePortTarget::from_bind_address("127.0.0.1", 3000).unwrap(),
            x_start: 1,
            x_end: 20,
            y: 4,
        };
        let ipv6 = PortClickTarget {
            target: crate::ports::RemotePortTarget::from_bind_address("::1", 3000).unwrap(),
            ..ipv4.clone()
        };

        assert!(ipv4.contains(5, 4));
        assert_ne!(ipv4.target, ipv6.target);
    }
}
