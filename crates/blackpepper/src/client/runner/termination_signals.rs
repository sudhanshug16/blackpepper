//! Signal-safe termination intent observed by the main event loop.

use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

pub(super) struct TerminationSignals {
    pending: Arc<AtomicUsize>,
    shutdown_started: Arc<AtomicBool>,
    // Retain every action for this single-run process. Registration errors
    // roll back only the actions installed by the incomplete setup.
    _registrations: Vec<signal_hook::SigId>,
}

impl TerminationSignals {
    #[cfg(unix)]
    pub(super) fn register() -> io::Result<Self> {
        use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};

        let pending = Arc::new(AtomicUsize::new(0));
        let shutdown_started = Arc::new(AtomicBool::new(false));
        let mut registrations = Vec::new();
        for signal in [SIGHUP, SIGINT, SIGQUIT, SIGTERM] {
            // Registration order is significant. The first signal observes
            // false, records itself, and starts graceful shutdown. A second
            // signal observes true here and terminates before the setters.
            if let Err(error) = retain(
                &mut registrations,
                signal_hook::flag::register_conditional_default(
                    signal,
                    Arc::clone(&shutdown_started),
                ),
            ) {
                rollback(&mut registrations);
                return Err(error);
            }
            if let Err(error) = retain(
                &mut registrations,
                signal_hook::flag::register_usize(signal, Arc::clone(&pending), signal as usize),
            ) {
                rollback(&mut registrations);
                return Err(error);
            }
            if let Err(error) = retain(
                &mut registrations,
                signal_hook::flag::register(signal, Arc::clone(&shutdown_started)),
            ) {
                rollback(&mut registrations);
                return Err(error);
            }
        }
        Ok(Self {
            pending,
            shutdown_started,
            _registrations: registrations,
        })
    }

    #[cfg(not(unix))]
    pub(super) fn register() -> io::Result<Self> {
        Ok(Self {
            pending: Arc::new(AtomicUsize::new(0)),
            shutdown_started: Arc::new(AtomicBool::new(false)),
            _registrations: Vec::new(),
        })
    }

    pub(super) fn pending(&self) -> &AtomicUsize {
        &self.pending
    }

    pub(super) fn signal(&self) -> Option<i32> {
        i32::try_from(self.pending.load(Ordering::SeqCst))
            .ok()
            .filter(|signal| *signal != 0)
    }

    pub(super) fn complete_cleanup(&self) {
        self.shutdown_started.store(true, Ordering::SeqCst);
    }
}

#[cfg(unix)]
fn retain(
    registrations: &mut Vec<signal_hook::SigId>,
    registration: io::Result<signal_hook::SigId>,
) -> io::Result<()> {
    registrations.push(registration?);
    Ok(())
}

#[cfg(unix)]
fn rollback(registrations: &mut Vec<signal_hook::SigId>) {
    for registration in registrations.drain(..) {
        signal_hook::low_level::unregister(registration);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_ignores_zero_and_preserves_signal_number() {
        let signal = TerminationSignals {
            pending: Arc::new(AtomicUsize::new(0)),
            shutdown_started: Arc::new(AtomicBool::new(false)),
            _registrations: Vec::new(),
        };
        assert_eq!(signal.signal(), None);
        signal.pending.store(15, Ordering::SeqCst);
        assert_eq!(signal.signal(), Some(15));
    }

    #[cfg(unix)]
    #[test]
    fn first_signal_defers_and_second_terminates_during_cleanup() {
        use std::os::unix::process::ExitStatusExt;
        use std::process::Command;

        const CHILD_ENV: &str = "BLACKPEPPER_TEST_DEFAULT_SIGNAL";
        if std::env::var_os(CHILD_ENV).is_some() {
            let signals = TerminationSignals::register().unwrap();
            // The first signal only records intent while cleanup is active.
            unsafe {
                libc::raise(libc::SIGTERM);
            }
            assert_eq!(signals.signal(), Some(libc::SIGTERM));
            assert!(signals.shutdown_started.load(Ordering::SeqCst));
            // SAFETY: SIGTERM is valid and intentionally terminates only this
            // isolated child process.
            unsafe {
                libc::raise(libc::SIGTERM);
            }
            std::process::exit(99);
        }

        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "client::runner::termination_signals::tests::first_signal_defers_and_second_terminates_during_cleanup",
            ])
            .env(CHILD_ENV, "1")
            .status()
            .unwrap();
        assert_eq!(status.signal(), Some(libc::SIGTERM));
    }
}
