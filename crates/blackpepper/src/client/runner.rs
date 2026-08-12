//! Terminal setup, raw-input reader, and paced event loop.

mod connection_restore;
mod connection_update;
pub(super) mod operations;
mod periodic;
mod terminal_io;

use super::control::handle_event;
use super::runtime::{ClientRuntime, ConnectionRestoreReport, ConnectionUpdate};
use super::{render, ClientEvent, ClientState, HostConnection};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::error::Error;
use std::io;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use terminal_io::{flush_input_modes, spawn_input_thread};

use connection_update::apply as apply_connection_update;

pub fn run() -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let config = crate::client_config::load(&cwd)?;
    let (mut runtime, snapshot) = ClientRuntime::initialize(&cwd, &config)?;
    let (event_tx, event_rx) = mpsc::channel();
    let mut state = ClientState::new(config, snapshot, event_tx.clone());
    let startup_warnings = runtime.take_startup_warnings();
    for host in &state.snapshot.hosts {
        state.connections.insert(
            host.id,
            if host.id == runtime.local_host_id() {
                HostConnection::Local
            } else {
                HostConnection::Disconnected
            },
        );
    }
    let agent_recovery_error = match runtime
        .rediscover_agent_runs(runtime.local_host_id(), event_tx.clone())
    {
        Ok(runs) => {
            state.upsert_discovered_agent_runs(runtime.local_host_id(), runs);
            None
        }
        Err(error) => Some(format!(
            "Local agent status recovery is unavailable: {error}. Existing Zellij sessions remain usable."
        )),
    };
    let mut startup_messages = startup_warnings;
    if let Some(error) = agent_recovery_error {
        startup_messages.push(error);
    }
    if !startup_messages.is_empty() {
        state.set_detail("Startup warnings", startup_messages.join("\n\n"));
        state.set_output(
            "Some startup work was unavailable. Review the warning panel; Esc closes it.",
        );
    }
    state.rebuild_tree();

    let mut stdout = io::stdout();
    enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;
    let result = run_loop(&mut terminal, &mut state, &mut runtime, event_rx, event_tx);
    runtime.shutdown_host_operations();
    state.reset_input_modes();
    let mode_cleanup = flush_input_modes(&mut terminal, &mut state);
    let screen_cleanup = terminal
        .backend_mut()
        .execute(LeaveAlternateScreen)
        .map(|_| ());
    let raw_cleanup = disable_raw_mode();
    let cursor_cleanup = terminal.show_cursor();
    result?;
    mode_cleanup?;
    screen_cleanup?;
    raw_cleanup?;
    cursor_cleanup?;
    Ok(())
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    event_rx: mpsc::Receiver<ClientEvent>,
    event_tx: Sender<ClientEvent>,
) -> io::Result<()> {
    const FRAME_INTERVAL: Duration = Duration::from_millis(16);
    const CONNECTION_POLL: Duration = Duration::from_millis(100);
    const PERIODIC_POLL: Duration = Duration::from_secs(2);
    spawn_input_thread(event_tx.clone());
    state.update_input_modes();
    flush_input_modes(terminal, state)?;
    terminal.clear()?;
    terminal.draw(|frame| render(state, frame))?;
    let mut last_draw = Instant::now();
    let mut last_connection_poll = Instant::now();
    let mut last_periodic_poll = Instant::now() - PERIODIC_POLL;
    let mut periodic = periodic::Coordinator::default();
    let mut restores = connection_restore::Coordinator::default();
    let mut dirty = false;

    while !state.should_quit {
        let timeout = FRAME_INTERVAL
            .checked_sub(last_draw.elapsed())
            .unwrap_or_default();
        match event_rx.recv_timeout(timeout) {
            Ok(event) => {
                dispatch_event(state, runtime, &mut periodic, &mut restores, event);
                dirty = true;
                while let Ok(event) = event_rx.try_recv() {
                    dispatch_event(state, runtime, &mut periodic, &mut restores, event);
                }
                restores.cancel_disconnected(state);
                restores.reconcile_user_disconnects(state, runtime);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if last_connection_poll.elapsed() >= CONNECTION_POLL {
            for update in runtime.poll_connections() {
                match &update {
                    ConnectionUpdate::Ready { previous, host_id } => {
                        periodic::invalidate_host(state, &mut periodic, *previous);
                        periodic::invalidate_host(state, &mut periodic, *host_id);
                        restores.invalidate(*previous);
                        restores.invalidate(*host_id);
                    }
                    ConnectionUpdate::Failed { host_id, .. } => {
                        periodic::invalidate_host(state, &mut periodic, *host_id);
                        restores.invalidate(*host_id);
                    }
                }
                let ready_host = match &update {
                    ConnectionUpdate::Ready { host_id, .. } => Some(*host_id),
                    ConnectionUpdate::Failed { .. } => None,
                };
                apply_connection_update(state, update);
                if let Some(host_id) = ready_host {
                    restores.start(state, runtime, host_id, &event_tx);
                }
                dirty = true;
            }
            last_connection_poll = Instant::now();
        }
        if last_periodic_poll.elapsed() >= PERIODIC_POLL {
            periodic::schedule(state, runtime, &mut periodic, &event_tx);
            last_periodic_poll = Instant::now();
            dirty = true;
        }
        if state.expire_transient_output() {
            dirty = true;
        }
        if last_draw.elapsed() >= FRAME_INTERVAL {
            if dirty {
                flush_input_modes(terminal, state)?;
                terminal.draw(|frame| render(state, frame))?;
                dirty = false;
            }
            last_draw = Instant::now();
        }
    }
    restores.shutdown();
    periodic.shutdown();
    Ok(())
}

fn dispatch_event(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    periodic: &mut periodic::Coordinator,
    restores: &mut connection_restore::Coordinator,
    event: ClientEvent,
) {
    match event {
        ClientEvent::PeriodicRefreshComplete {
            token,
            host_id,
            result,
        } => {
            periodic::complete(state, runtime, periodic, token, host_id, result);
        }
        ClientEvent::PeriodicForwardCleanupComplete {
            token,
            host_id,
            outcomes,
        } => periodic::complete_forward_cleanup(state, runtime, periodic, token, host_id, outcomes),
        ClientEvent::ConnectionRestoreProgress {
            token,
            host_id,
            message,
        } => restores.progress(state, token, host_id, message),
        ClientEvent::ConnectionRestoreComplete { token, host_id } => {
            restores.complete(state, runtime, token, host_id)
        }
        ClientEvent::HostOperationProgress {
            token,
            host_id,
            message,
        } => operations::progress(state, token, host_id, message),
        ClientEvent::HostOperationComplete {
            token,
            host_id,
            generation,
        } => operations::complete(state, runtime, token, host_id, generation),
        ClientEvent::ManualRefreshRequested => {
            periodic::schedule(state, runtime, periodic, &state.event_tx.clone())
        }
        event => handle_event(state, runtime, event),
    }
    // Commands, Ctrl-C persistence, and terminal detach can all acquire a
    // host during event handling. Reject any observation that predates that
    // ownership even if the explicit operation finishes before its result is
    // eventually dequeued.
    periodic::invalidate_owned(state, runtime, periodic);
}
