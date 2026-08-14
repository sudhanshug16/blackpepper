use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crate::transport::{CommandCancellation, HostTransport};

use super::super::super::model::{ClientOperation, ZellijClient, ZellijError, ZellijTab};
use super::super::ZellijRuntime;

pub(super) const BACKGROUND_PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(2);
const BACKGROUND_PREFLIGHT_POLL: Duration = Duration::from_millis(25);

#[derive(Debug)]
pub(crate) enum BackgroundTabPreflight {
    Existing(u64),
    Create {
        preexisting_tab_ids: BTreeSet<u64>,
        restore_focus: Option<(u32, u64)>,
    },
}

impl ZellijRuntime {
    /// Read one coherent client/tab snapshot before a focus-affecting tab
    /// mutation. Zellij clients can detach between independent metadata calls;
    /// every retry remains read-only and shares one deadline.
    pub(crate) fn background_tab_preflight(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        name: &str,
        timeout: Duration,
    ) -> Result<BackgroundTabPreflight, ZellijError> {
        let started = Instant::now();
        loop {
            let remaining = preflight_remaining(started, timeout)?;
            let clients_before = self.list_clients_with_timeout(host, session, remaining)?;
            reject_multiple_clients(&clients_before)?;

            let remaining = preflight_remaining(started, timeout)?;
            let tabs = self.list_tabs_with_timeout(host, session, remaining)?;
            let matching = tabs
                .iter()
                .filter(|tab| tab.name == name)
                .collect::<Vec<_>>();
            match matching.as_slice() {
                [tab] => return Ok(BackgroundTabPreflight::Existing(tab.tab_id)),
                [] => {}
                _ => {
                    return Err(ZellijError::InvalidOutput(format!(
                        "found {} Zellij tabs named {name:?}; refusing to choose one",
                        matching.len()
                    )));
                }
            }

            let remaining = preflight_remaining(started, timeout)?;
            let clients_after = self.list_clients_with_timeout(host, session, remaining)?;
            reject_multiple_clients(&clients_after)?;
            let active_tab_ids = tabs
                .iter()
                .filter(|tab| tab.active)
                .map(|tab| tab.tab_id)
                .collect::<Vec<_>>();
            if clients_before == clients_after {
                match clients_after.as_slice() {
                    // With no controlling client there is no focus to steal or
                    // restore. A stale active marker can outlive a client
                    // between the independent server snapshots.
                    [] => {
                        preflight_remaining(started, timeout)?;
                        return Ok(creation_snapshot(tabs, None));
                    }
                    [client] if active_tab_ids.len() == 1 => {
                        preflight_remaining(started, timeout)?;
                        return Ok(creation_snapshot(
                            tabs,
                            Some((client.client_id, active_tab_ids[0])),
                        ));
                    }
                    _ => {}
                }
            }

            let remaining = preflight_remaining(started, timeout)?;
            std::thread::sleep(BACKGROUND_PREFLIGHT_POLL.min(remaining));
        }
    }
}

fn reject_multiple_clients(clients: &[ZellijClient]) -> Result<(), ZellijError> {
    if clients.len() <= 1 {
        return Ok(());
    }
    Err(ZellijError::ClientConflict {
        operation: ClientOperation::BackgroundMutation,
        clients: clients.to_vec(),
    })
}

fn creation_snapshot(
    tabs: Vec<ZellijTab>,
    restore_focus: Option<(u32, u64)>,
) -> BackgroundTabPreflight {
    BackgroundTabPreflight::Create {
        preexisting_tab_ids: tabs.iter().map(|tab| tab.tab_id).collect::<BTreeSet<_>>(),
        restore_focus,
    }
}

fn preflight_cancelled() -> ZellijError {
    ZellijError::InvalidOutput(
        "background tab preflight was cancelled; no new-tab was sent".to_string(),
    )
}

fn preflight_remaining(started: Instant, timeout: Duration) -> Result<Duration, ZellijError> {
    if CommandCancellation::scope_is_cancelled() {
        return Err(preflight_cancelled());
    }
    let remaining = timeout.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        return Err(ZellijError::InvalidOutput(format!(
            "Zellij client/tab state did not stabilize within {}ms; no new-tab was sent",
            timeout.as_millis()
        )));
    }
    Ok(remaining)
}
