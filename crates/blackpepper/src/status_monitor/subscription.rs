use std::io::{self, BufReader};
use std::process::{Command, Stdio};
use std::sync::{mpsc::Receiver, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::zellij::ZellijRuntime;

mod error;
pub use error::HostSubscriptionError;

use super::{
    consume_subscription_fallible, stream::consume_subscription_shared_fallible, BlockerTransition,
    StreamStats, ViewportBlockerMonitor,
};
use crate::agent_status::IntegrationHealth;

/// Spawn and reduce a Zellij subscription on the current machine.
///
/// `bp-host` should call this on the workspace host. Calling it on the client
/// with an SSH process would violate the viewport privacy boundary.
pub fn run_host_local_subscription<C, E>(
    runtime: &ZellijRuntime,
    session: &str,
    monitor: &mut ViewportBlockerMonitor,
    now_ms: C,
    mut emit: E,
) -> Result<StreamStats, HostSubscriptionError>
where
    C: FnMut() -> u64,
    E: FnMut(BlockerTransition),
{
    run_host_local_subscription_fallible(runtime, session, monitor, now_ms, |transition| {
        emit(transition);
        Ok(())
    })
}

/// Fallible form for `bp-host`, where emitting compact JSON can fail when the
/// SSH client disconnects.
pub fn run_host_local_subscription_fallible<C, E>(
    runtime: &ZellijRuntime,
    session: &str,
    monitor: &mut ViewportBlockerMonitor,
    now_ms: C,
    emit: E,
) -> Result<StreamStats, HostSubscriptionError>
where
    C: FnMut() -> u64,
    E: FnMut(BlockerTransition) -> io::Result<()>,
{
    let command = runtime.subscribe_command(session, monitor.zellij_pane_id())?;
    let mut process = Command::new(&command.program);
    process.args(&command.args).envs(&command.env);
    if let Some(cwd) = &command.cwd {
        process.current_dir(cwd);
    }
    let mut child = process
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // Zellij errors are intentionally reduced to the exit status. A
        // helper must never mix arbitrary process text into its JSON stream.
        .stderr(Stdio::null())
        .spawn()
        .map_err(HostSubscriptionError::Spawn)?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(HostSubscriptionError::MissingStdout);
    };
    let stats = match consume_subscription_fallible(BufReader::new(stdout), monitor, now_ms, emit) {
        Ok(stats) => stats,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(HostSubscriptionError::Stream(error));
        }
    };
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(HostSubscriptionError::Wait(error));
        }
    };
    if !status.success() {
        return Err(HostSubscriptionError::Exited(status.code()));
    }
    Ok(stats)
}

/// Cancellable host-helper form. The caller keeps the helper's stdin open and
/// signals this receiver when the client channel closes. The Zellij
/// subscription is then killed and reaped even if no viewport is changing.
pub fn run_host_local_subscription_cancellable<C, E>(
    runtime: &ZellijRuntime,
    session: &str,
    monitor: ViewportBlockerMonitor,
    now_ms: C,
    emit: E,
    cancelled: Receiver<()>,
) -> Result<StreamStats, HostSubscriptionError>
where
    C: FnMut() -> u64 + Send + 'static,
    E: FnMut(BlockerTransition) -> io::Result<()> + Send + 'static,
{
    run_host_local_subscription_cancellable_inner(
        runtime,
        session,
        monitor,
        now_ms,
        emit,
        cancelled,
        None,
        Duration::from_secs(1),
    )
}

/// Cancellable helper with a compact provider-health poll. The Zellij stream
/// and health poll share only the redacted matcher state, so OpenCode can
/// switch between plugin authority and screen fallback without restarting the
/// provider or sending viewport contents over SSH.
#[allow(clippy::too_many_arguments)]
pub fn run_host_local_subscription_cancellable_with_health<C, E, H>(
    runtime: &ZellijRuntime,
    session: &str,
    monitor: ViewportBlockerMonitor,
    now_ms: C,
    emit: E,
    cancelled: Receiver<()>,
    mut health_poll: H,
    health_poll_interval: Duration,
) -> Result<StreamStats, HostSubscriptionError>
where
    C: FnMut() -> u64 + Send + 'static,
    E: FnMut(BlockerTransition) -> io::Result<()> + Send + 'static,
    H: FnMut() -> IntegrationHealth,
{
    run_host_local_subscription_cancellable_inner(
        runtime,
        session,
        monitor,
        now_ms,
        emit,
        cancelled,
        Some(&mut health_poll),
        health_poll_interval,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_host_local_subscription_cancellable_inner<C, E>(
    runtime: &ZellijRuntime,
    session: &str,
    monitor: ViewportBlockerMonitor,
    now_ms: C,
    emit: E,
    cancelled: Receiver<()>,
    mut health_poll: Option<&mut dyn FnMut() -> IntegrationHealth>,
    health_poll_interval: Duration,
) -> Result<StreamStats, HostSubscriptionError>
where
    C: FnMut() -> u64 + Send + 'static,
    E: FnMut(BlockerTransition) -> io::Result<()> + Send + 'static,
{
    let command = runtime.subscribe_command(session, monitor.zellij_pane_id())?;
    let mut process = Command::new(&command.program);
    process.args(&command.args).envs(&command.env);
    if let Some(cwd) = &command.cwd {
        process.current_dir(cwd);
    }
    let mut child = process
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(HostSubscriptionError::Spawn)?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(HostSubscriptionError::MissingStdout);
    };
    let monitor = Arc::new(Mutex::new(monitor));
    let emit = Arc::new(Mutex::new(emit));
    let stream_monitor = Arc::clone(&monitor);
    let stream_emit = Arc::clone(&emit);
    let worker = std::thread::spawn(move || {
        consume_subscription_shared_fallible(
            BufReader::new(stdout),
            stream_monitor,
            now_ms,
            move |transition| {
                let mut emit = stream_emit
                    .lock()
                    .map_err(|_| io::Error::other("blocker output lock was poisoned"))?;
                emit(transition)
            },
        )
    });
    let mut last_health_poll = Instant::now()
        .checked_sub(health_poll_interval)
        .unwrap_or_else(Instant::now);

    loop {
        if cancelled.try_recv().is_ok() {
            let _ = child.kill();
            let _ = child.wait();
            return join_worker(worker);
        }
        if worker.is_finished() {
            let result = join_worker(worker);
            if result.is_err() {
                let _ = child.kill();
                let _ = child.wait();
                return result;
            }
            let status = child.wait().map_err(HostSubscriptionError::Wait)?;
            return if status.success() {
                result
            } else {
                Err(HostSubscriptionError::Exited(status.code()))
            };
        }
        if let Some(status) = child.try_wait().map_err(HostSubscriptionError::Wait)? {
            let result = join_worker(worker);
            return if status.success() {
                result
            } else {
                Err(HostSubscriptionError::Exited(status.code()))
            };
        }
        if let Some(poll) = health_poll.as_deref_mut() {
            if last_health_poll.elapsed() >= health_poll_interval {
                let health = poll();
                let transition = monitor
                    .lock()
                    .map_err(|_| HostSubscriptionError::WorkerPanicked)?
                    .set_integration_health(health, system_millis());
                if let Some(transition) = transition {
                    let result = emit
                        .lock()
                        .map_err(|_| HostSubscriptionError::WorkerPanicked)?(
                        transition
                    );
                    if let Err(error) = result {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = join_worker(worker);
                        return Err(HostSubscriptionError::Stream(error));
                    }
                }
                last_health_poll = Instant::now();
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn system_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn join_worker(
    worker: std::thread::JoinHandle<Result<StreamStats, io::Error>>,
) -> Result<StreamStats, HostSubscriptionError> {
    worker
        .join()
        .map_err(|_| HostSubscriptionError::WorkerPanicked)?
        .map_err(HostSubscriptionError::Stream)
}
