use super::{ports, ClientRuntime, HostSlot};
use crate::core::{HostId, RegistrySnapshot};
use crate::ports::{ForwardState, ForwardStatus};
use crate::transport::{ProcessSpec, RunningCommand, TransportError};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);

enum CleanupPlan {
    Local {
        forward_id: uuid::Uuid,
    },
    Ssh {
        forward_id: uuid::Uuid,
        spec: ProcessSpec,
    },
    Immediate {
        forward_id: uuid::Uuid,
        result: Result<(), String>,
    },
}

pub(crate) struct ForwardCleanupBatch {
    pub host_id: HostId,
    plans: Vec<CleanupPlan>,
}

impl ForwardCleanupBatch {
    pub fn is_empty(&self) -> bool {
        self.plans.is_empty()
    }

    pub fn forward_ids(&self) -> impl Iterator<Item = uuid::Uuid> + '_ {
        self.plans.iter().map(|plan| match plan {
            CleanupPlan::Local { forward_id }
            | CleanupPlan::Ssh { forward_id, .. }
            | CleanupPlan::Immediate { forward_id, .. } => *forward_id,
        })
    }

    pub fn wait_cancellable(self, cancellation: Receiver<()>) -> Vec<ForwardCleanupOutcome> {
        let mut outcomes = Vec::with_capacity(self.plans.len());
        for plan in self.plans {
            if cancellation_requested(&cancellation) {
                outcomes.push(ForwardCleanupOutcome::cancelled(plan.forward_id()));
                continue;
            }
            let outcome = match plan {
                CleanupPlan::Local { forward_id } => ForwardCleanupOutcome {
                    forward_id,
                    result: Ok(()),
                },
                CleanupPlan::Immediate { forward_id, result } => {
                    ForwardCleanupOutcome { forward_id, result }
                }
                CleanupPlan::Ssh { forward_id, spec } => ForwardCleanupOutcome {
                    forward_id,
                    result: run_cancel(spec, &cancellation),
                },
            };
            outcomes.push(outcome);
        }
        outcomes
    }
}

impl CleanupPlan {
    fn forward_id(&self) -> uuid::Uuid {
        match self {
            Self::Local { forward_id }
            | Self::Ssh { forward_id, .. }
            | Self::Immediate { forward_id, .. } => *forward_id,
        }
    }
}

#[derive(Debug)]
pub struct ForwardCleanupOutcome {
    pub forward_id: uuid::Uuid,
    pub result: Result<(), String>,
}

impl ForwardCleanupOutcome {
    fn cancelled(forward_id: uuid::Uuid) -> Self {
        Self {
            forward_id,
            result: Err("Forward cleanup was cancelled during shutdown or reconnect.".to_owned()),
        }
    }
}

impl ClientRuntime {
    pub(crate) fn prepare_orphan_forward_cleanup(
        &mut self,
        forwards: &[ForwardState],
        snapshot: &RegistrySnapshot,
        host_id: HostId,
    ) -> ForwardCleanupBatch {
        let mut plans = Vec::new();
        for forward in forwards.iter().filter(|forward| {
            forward.host_id == host_id
                && forward.status != ForwardStatus::Cancelling
                && !ports::forward_workspace_is_registered(snapshot, forward)
        }) {
            if host_id == self.local_host_id {
                plans.push(CleanupPlan::Local {
                    forward_id: forward.id,
                });
                continue;
            }
            let requested = ports::local_forward(forward);
            let plan = match self.hosts.get(&host_id) {
                None => CleanupPlan::Immediate {
                    forward_id: forward.id,
                    result: Ok(()),
                },
                Some(HostSlot::Local(_)) => unreachable!("remote host used a local transport"),
                Some(HostSlot::Ssh(host)) => {
                    match host.transport.background_cancel_spec(&requested) {
                        Ok(spec) => CleanupPlan::Ssh {
                            forward_id: forward.id,
                            spec,
                        },
                        Err(TransportError::ForwardNotOwned(_)) => CleanupPlan::Immediate {
                            forward_id: forward.id,
                            result: Ok(()),
                        },
                        Err(error) => CleanupPlan::Immediate {
                            forward_id: forward.id,
                            result: Err(error.to_string()),
                        },
                    }
                }
            };
            plans.push(plan);
        }
        ForwardCleanupBatch { host_id, plans }
    }

    pub(crate) fn confirm_orphan_forward_cleanup(&mut self, forward: &ForwardState) {
        if forward.host_id == self.local_host_id {
            self.local_port_proxies.remove(&forward.local_address);
            return;
        }
        if let Some(HostSlot::Ssh(host)) = self.hosts.get_mut(&forward.host_id) {
            host.transport
                .confirm_background_cancel(&ports::local_forward(forward));
        }
    }
}

fn run_cancel(spec: ProcessSpec, cancellation: &Receiver<()>) -> Result<(), String> {
    const POLL_INTERVAL: Duration = Duration::from_millis(10);
    let mut child = RunningCommand::spawn(&spec, false).map_err(|error| error.to_string())?;
    let started = Instant::now();
    loop {
        if child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_some()
        {
            let output = child
                .wait_with_output()
                .map_err(|error| error.to_string())?;
            return if output.success {
                Ok(())
            } else {
                Err(format!(
                    "SSH tunnel cancellation failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ))
            };
        }
        if cancellation_requested(cancellation) {
            let _ = child.cancel();
            return Err("Forward cleanup was cancelled during shutdown or reconnect.".to_owned());
        }
        if started.elapsed() >= CLEANUP_TIMEOUT {
            let _ = child.cancel();
            return Err(format!(
                "SSH tunnel cancellation exceeded its {}ms deadline.",
                CLEANUP_TIMEOUT.as_millis()
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn cancellation_requested(cancellation: &Receiver<()>) -> bool {
    matches!(
        cancellation.try_recv(),
        Ok(()) | Err(TryRecvError::Disconnected)
    )
}
