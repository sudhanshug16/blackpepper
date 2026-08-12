use crate::agent_status::{
    BlockerExplain, BlockerInput, BlockerManifestError, BlockerOverlay, IntegrationHealth, Provider,
};

use super::{BlockerChange, BlockerSource, BlockerTransition, MonitorContext};

/// Stateful matcher for one agent run in one Zellij pane.
pub struct ViewportBlockerMonitor {
    context: MonitorContext,
    zellij_pane_id: String,
    overlay: BlockerOverlay,
    /// Last redacted rule result for the current viewport. Keeping the rule
    /// metadata (never the viewport) lets a long-lived watcher react when
    /// OpenCode authority becomes stale without waiting for another repaint.
    last_match: Option<BlockerExplain>,
    current: Option<BlockerExplain>,
    sequence: u64,
}

impl ViewportBlockerMonitor {
    pub fn bundled(
        context: MonitorContext,
        zellij_pane_id: impl Into<String>,
    ) -> Result<Self, BlockerManifestError> {
        Self::bundled_after(context, zellij_pane_id, 0)
    }

    /// Resume after the last sequence known by a still-live client tracker.
    /// A fresh transient helper can therefore preserve monotonic ordering.
    pub fn bundled_after(
        context: MonitorContext,
        zellij_pane_id: impl Into<String>,
        last_sequence: u64,
    ) -> Result<Self, BlockerManifestError> {
        let overlay = BlockerOverlay::bundled(context.provider)?;
        Ok(Self {
            context,
            zellij_pane_id: zellij_pane_id.into(),
            overlay,
            last_match: None,
            current: None,
            sequence: last_sequence,
        })
    }

    pub fn zellij_pane_id(&self) -> &str {
        &self.zellij_pane_id
    }

    /// Update provider health without inspecting terminal contents.
    ///
    /// A healthy OpenCode plugin is the full authority, so becoming healthy
    /// clears any existing screen overlay immediately.
    pub fn set_integration_health(
        &mut self,
        health: IntegrationHealth,
        observed_at_ms: u64,
    ) -> Option<BlockerTransition> {
        self.context.integration_health = health;
        let next = self
            .overlay_allowed()
            .then(|| self.last_match.clone())
            .flatten();
        self.apply_match(next, observed_at_ms)
    }

    /// Match one complete viewport. `viewport` is borrowed only for this call.
    pub fn observe(
        &mut self,
        viewport: &str,
        terminal_title: Option<&str>,
        observed_at_ms: u64,
    ) -> Option<BlockerTransition> {
        self.last_match = self.overlay.evaluate(BlockerInput {
            viewport,
            terminal_title,
        });
        let next = self
            .overlay_allowed()
            .then(|| self.last_match.clone())
            .flatten();
        self.apply_match(next, observed_at_ms)
    }

    /// Pane closure removes a visible blocker. Exited state remains the
    /// process supervisor's responsibility.
    pub fn pane_closed(&mut self, observed_at_ms: u64) -> Option<BlockerTransition> {
        self.last_match = None;
        self.apply_match(None, observed_at_ms)
    }

    fn overlay_allowed(&self) -> bool {
        self.context.provider != Provider::OpenCode || !self.context.integration_health.is_healthy()
    }

    fn apply_match(
        &mut self,
        next: Option<BlockerExplain>,
        observed_at_ms: u64,
    ) -> Option<BlockerTransition> {
        if self.current == next {
            return None;
        }
        let state = match &next {
            Some(blocker) => BlockerChange::NeedsInput {
                rule_id: blocker.rule_id.clone(),
                confidence: blocker.confidence,
                priority: blocker.priority,
            },
            None => BlockerChange::Cleared,
        };
        self.current = next;
        Some(self.transition(observed_at_ms, state))
    }

    fn transition(&mut self, observed_at_ms: u64, state: BlockerChange) -> BlockerTransition {
        self.sequence = self.sequence.saturating_add(1);
        BlockerTransition {
            host_id: self.context.host_id,
            workspace_id: self.context.workspace_id,
            run_id: self.context.run_id,
            pane_id: self.context.pane_id,
            provider: self.context.provider,
            sequence: self.sequence,
            observed_at_ms,
            source: BlockerSource::ZellijViewport,
            manifest_version: self.overlay.version().to_string(),
            state,
        }
    }
}
