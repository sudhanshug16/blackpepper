use crate::bundle::{bundle_count, publish_bundle, remove_stopped_client};
use crate::config::Config;
use crate::process::{append_log, reset_log, BuildProcess, ClientProcess};
use crate::source::{SourceEvent, SourceWatch};
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub(crate) fn run(config: Config) -> Result<i32, String> {
    let stopping = Arc::new(AtomicBool::new(false));
    for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGTERM] {
        signal_hook::flag::register(signal, Arc::clone(&stopping))
            .map_err(|error| format!("could not register termination signal: {error}"))?;
    }

    reset_log(&config.log)?;
    let source = SourceWatch::start(&config.root)?;
    println!(
        "Building a temporary Blackpepper source client. Build log: {}",
        config.log.display()
    );
    let build = BuildProcess::start(&config, 1)?;
    set_terminal_title("Blackpepper — building temporary source client");
    let mut supervisor = Supervisor {
        config,
        stopping,
        source,
        binary: None,
        client: None,
        build: Some(build),
        changed_at: None,
        next_sequence: 2,
        storage_warning_shown: false,
    };
    let outcome = supervisor.event_loop();
    let cleanup = supervisor.shutdown();
    match (outcome, cleanup) {
        (Ok(code), Ok(())) => Ok(code),
        (Err(error), _) | (Ok(_), Err(error)) => Err(error),
    }
}

struct Supervisor {
    config: Config,
    stopping: Arc<AtomicBool>,
    source: SourceWatch,
    binary: Option<std::path::PathBuf>,
    client: Option<ClientProcess>,
    build: Option<BuildProcess>,
    changed_at: Option<Instant>,
    next_sequence: u64,
    storage_warning_shown: bool,
}

impl Supervisor {
    fn event_loop(&mut self) -> Result<i32, String> {
        loop {
            if self.stopping.load(Ordering::Relaxed) {
                return Ok(0);
            }
            if let Some(status) = self
                .client
                .as_mut()
                .map(ClientProcess::try_wait)
                .transpose()?
                .flatten()
            {
                return Ok(status.code().unwrap_or(1));
            }
            self.receive_source_events()?;
            self.finish_build()?;
            self.start_build_if_ready()?;
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn receive_source_events(&mut self) -> Result<(), String> {
        while let Some(event) = self.source.try_recv()? {
            match event {
                SourceEvent::Changed => self.changed_at = Some(Instant::now()),
                SourceEvent::Failed(error) => {
                    append_log(
                        &self.config.log,
                        &format!("Error: source watch failed: {error}"),
                    )?;
                    set_terminal_title("Blackpepper — source watch failed; see build log");
                    ring_bell();
                }
            }
        }
        Ok(())
    }

    fn start_build_if_ready(&mut self) -> Result<(), String> {
        if self.build.is_some()
            || self
                .changed_at
                .is_none_or(|changed| changed.elapsed() < self.config.debounce)
        {
            return Ok(());
        }
        self.changed_at = None;
        self.build = Some(BuildProcess::start(&self.config, self.next_sequence)?);
        self.next_sequence += 1;
        set_terminal_title("Blackpepper — rebuilding temporary source client");
        Ok(())
    }

    fn finish_build(&mut self) -> Result<(), String> {
        let Some(status) = self
            .build
            .as_mut()
            .map(BuildProcess::try_wait)
            .transpose()?
            .flatten()
        else {
            return Ok(());
        };
        let completed = self.build.take().expect("completed build remains present");
        if !status.success() {
            return self.build_failed(format!("build exited {status}"));
        }
        if self.changed_at.is_some() {
            append_log(
                &self.config.log,
                "Source changed during the build; the stale output was not launched.",
            )?;
            set_terminal_title("Blackpepper — source changed; rebuilding again");
            return Ok(());
        }
        let next = match publish_bundle(&self.config, completed.build_id()) {
            Ok(binary) => binary,
            Err(error) => return self.build_failed(error),
        };
        if let Some(client) = self.client.as_mut() {
            client.stop(self.config.grace, true)?;
        }
        let reloading = self.client.is_some();
        let next_client = ClientProcess::launch(&next, &self.config.launch_cwd)?;
        if let Some(previous) = self.binary.take() {
            remove_stopped_client(&self.config, &previous)?;
        }
        self.binary = Some(next.clone());
        self.client = Some(next_client);
        if reloading {
            println!("Source build succeeded; temporary client reloaded.");
        } else {
            println!(
                "Running temporary source client from {}. bp-dev was not changed.",
                next.display()
            );
        }
        self.warn_storage_if_needed()?;
        set_terminal_title("Blackpepper — temporary source run");
        Ok(())
    }

    fn build_failed(&self, error: String) -> Result<(), String> {
        append_log(&self.config.log, &format!("Error: {error}"))?;
        if self.client.is_some() {
            set_terminal_title("Blackpepper — build failed; old TUI still running");
        } else {
            eprintln!(
                "Source build failed; waiting for another edit. See {}",
                self.config.log.display()
            );
            set_terminal_title("Blackpepper — initial build failed; waiting for edit");
        }
        ring_bell();
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), String> {
        if let Some(build) = self.build.as_mut() {
            build.cancel(self.config.grace)?;
        }
        if let Some(client) = self.client.as_mut() {
            client.stop(self.config.grace, false)?;
        }
        if let Some(binary) = self.binary.take() {
            remove_stopped_client(&self.config, &binary)?;
        }
        Ok(())
    }

    fn warn_storage_if_needed(&mut self) -> Result<(), String> {
        if self.storage_warning_shown {
            return Ok(());
        }
        let count = bundle_count(&self.config)?;
        if count > 5 {
            eprintln!(
                "Warning: {count} temporary helper bundles are retained under {} for exact provider-hook paths. Remove that directory only after its source-run agents have stopped.",
                self.config.bundle_root.display()
            );
            self.storage_warning_shown = true;
        }
        Ok(())
    }
}

fn set_terminal_title(title: &str) {
    if std::io::stderr().is_terminal() {
        let _ = write!(std::io::stderr(), "\x1b]2;{title}\x07");
        let _ = std::io::stderr().flush();
    }
}

fn ring_bell() {
    if std::io::stderr().is_terminal() {
        let _ = write!(std::io::stderr(), "\x07");
        let _ = std::io::stderr().flush();
    }
}
