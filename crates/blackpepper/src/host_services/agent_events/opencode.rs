use super::{healthy_event, HostAgentEvents};
use crate::agent_status::{
    AgentEventKind, AgentEventSource, DeliveryContinuity, IntegrationHealth, Provider,
};

impl HostAgentEvents {
    /// Record one successful delivery from the managed OpenCode plugin.
    /// Heartbeats only advance a single freshness row; the semantic log grows
    /// only when health changes or the provider reports a real state edge.
    pub fn record_opencode_delivery(
        &mut self,
        context: super::AgentRunContext,
        kinds: &[AgentEventKind],
        semantic_sequence: u64,
    ) -> Result<(), String> {
        self.record_opencode_delivery_at(context, kinds, semantic_sequence, super::now_millis())
    }

    pub(in crate::host_services) fn record_opencode_delivery_at(
        &mut self,
        context: super::AgentRunContext,
        kinds: &[AgentEventKind],
        semantic_sequence: u64,
        observed_at_ms: u64,
    ) -> Result<(), String> {
        if context.provider != Provider::OpenCode {
            return Err("Integration heartbeats are only valid for OpenCode.".to_owned());
        }
        let _mutation_lock = super::lock_mutations(&self.path)?;
        let stored = self
            .active_context(context.run_id)?
            .ok_or_else(|| "Agent run is not registered or is stale.".to_owned())?;
        if stored != context {
            return Err("Agent event context does not match the registered run.".to_owned());
        }
        let current_health = self
            .store
            .snapshot(context.run_id)
            .map_err(|error| error.to_string())?
            .map_or(IntegrationHealth::Unknown, |snapshot| {
                snapshot.integration_health
            });

        if kinds.is_empty() {
            // A heartbeat has no provider state to lose. Advance its compact
            // cursor first; if the recovery edge then fails, durable health
            // remains stale and a later snapshot safely retries that edge.
            let continuity = self
                .store
                .touch_integration(
                    context.run_id,
                    context.provider,
                    observed_at_ms,
                    Some(1),
                    semantic_sequence,
                )
                .map_err(|error| error.to_string())?;
            if continuity == DeliveryContinuity::Gap {
                self.mark_opencode_gap_locked(context, observed_at_ms)?;
                return Err(
                    "OpenCode integration delivery has a missing semantic event.".to_owned(),
                );
            }
            if !current_health.is_healthy() {
                self.append_many_locked(
                    context,
                    &[healthy_event()],
                    AgentEventSource::ProviderIntegration,
                    observed_at_ms,
                )?;
            }
            return Ok(());
        }

        let handshake = semantic_sequence == 0 && kinds == [healthy_event()];
        if semantic_sequence == 0 && !handshake {
            self.mark_opencode_gap_locked(context, observed_at_ms)?;
            return Err("OpenCode semantic delivery cursor must be positive.".to_owned());
        }
        if handshake
            && self
                .store
                .integration_freshness(context.run_id)
                .map_err(|error| error.to_string())?
                .is_some()
        {
            self.mark_opencode_gap_locked(context, observed_at_ms)?;
            return Err("OpenCode integration handshake was duplicated.".to_owned());
        }

        if !handshake {
            let freshness = self
                .store
                .integration_freshness(context.run_id)
                .map_err(|error| error.to_string())?;
            let expected = freshness.as_ref().is_some_and(|freshness| {
                !freshness.delivery_gap
                    && semantic_sequence <= i64::MAX as u64
                    && freshness
                        .semantic_sequence
                        .checked_add(1)
                        .is_some_and(|next| next == semantic_sequence)
            });
            if !expected {
                self.mark_opencode_gap_locked(context, observed_at_ms)?;
                return Err(
                    "OpenCode integration delivery has a missing, duplicate, or out-of-order semantic event."
                        .to_owned(),
                );
            }
        }

        let mut semantic = Vec::with_capacity(kinds.len() + 1);
        if !current_health.is_healthy() && !kinds.contains(&healthy_event()) {
            semantic.push(healthy_event());
        }
        semantic.extend_from_slice(kinds);
        self.append_many_locked(
            context,
            &semantic,
            AgentEventSource::ProviderIntegration,
            observed_at_ms,
        )?;
        // Never let a failed semantic delivery refresh full plugin authority.
        // The host mutation lock remains held across both commits, so another
        // watcher cannot observe the new state before its freshness cursor.
        let continuity = if handshake {
            self.store.touch_integration(
                context.run_id,
                context.provider,
                observed_at_ms,
                Some(1),
                0,
            )
        } else {
            self.store.advance_integration(
                context.run_id,
                context.provider,
                observed_at_ms,
                Some(1),
                semantic_sequence,
            )
        }
        .map_err(|error| error.to_string())?;
        if continuity == DeliveryContinuity::Gap {
            self.mark_opencode_gap_locked(context, observed_at_ms)?;
            return Err("OpenCode integration delivery continuity was lost.".to_owned());
        }
        Ok(())
    }

    fn mark_opencode_gap_locked(
        &mut self,
        context: super::AgentRunContext,
        observed_at_ms: u64,
    ) -> Result<(), String> {
        self.store
            .mark_integration_gap(context.run_id, context.provider, Some(1))
            .map_err(|error| error.to_string())?;
        let health = self
            .store
            .snapshot(context.run_id)
            .map_err(|error| error.to_string())?
            .map_or(IntegrationHealth::Unknown, |snapshot| {
                snapshot.integration_health
            });
        if health != IntegrationHealth::Stale {
            self.append_many_locked(
                context,
                &[AgentEventKind::IntegrationHealthChanged {
                    health: IntegrationHealth::Stale,
                }],
                AgentEventSource::IntegrationSupervisor,
                observed_at_ms,
            )?;
        }
        Ok(())
    }
}
