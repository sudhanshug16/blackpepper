use blackpepper::core::{serve_json_lines_with_extension, CorePaths, HostRegistry};
use blackpepper::host_services::{
    hold_session_lease, record_provider_hook, watch_blockers_cancellable, BlockerWatchArgs,
    HostServices, ProviderHookArgs, SessionLeaseArgs,
};
use std::{error::Error, fs, io, io::Read, process::ExitCode};

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .first()
        .is_some_and(|argument| argument == "agent-event")
    {
        run_provider_hook(arguments.into_iter().skip(1));
        return ExitCode::SUCCESS;
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "watch-blockers")
    {
        return match run_blocker_watch(arguments.into_iter().skip(1)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("bp-host: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "session-lease")
    {
        return match run_session_lease(arguments.into_iter().skip(1)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("bp-host: {error}");
                ExitCode::FAILURE
            }
        };
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("bp-host: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    if let Some(argument) = std::env::args().nth(1) {
        if argument == "--version" || argument == "-V" {
            println!("bp-host {}", blackpepper::BUILD_ID);
            return Ok(());
        }
        return Err(
            format!("unexpected argument {argument:?}; bp-host uses JSON lines on stdin").into(),
        );
    }

    let paths = CorePaths::discover()?;
    paths.prepare()?;
    let mut registry = HostRegistry::open(paths.registry_path())?;
    registry.ensure_local_host(&local_display_name())?;
    let mut services = HostServices::new(paths);

    let stdin = io::stdin();
    let stdout = io::stdout();
    serve_json_lines_with_extension(&registry, &mut services, stdin.lock(), stdout.lock())?;
    Ok(())
}

/// Provider hooks are intentionally fail-silent: malformed or unavailable
/// status reporting must never interfere with the user's agent process.
fn run_provider_hook(arguments: impl IntoIterator<Item = String>) {
    let Some(arguments) = ProviderHookArgs::parse(arguments) else {
        return;
    };
    let Ok(paths) = CorePaths::discover() else {
        return;
    };
    if paths.prepare().is_err() {
        return;
    }
    let Ok(mut registry) = HostRegistry::open(paths.registry_path()) else {
        return;
    };
    if registry.ensure_local_host(&local_display_name()).is_err() {
        return;
    }
    let stdin = io::stdin();
    record_provider_hook(&paths, &registry, arguments, stdin.lock());
}

fn run_blocker_watch(arguments: impl IntoIterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let arguments = BlockerWatchArgs::parse(arguments)
        .ok_or("invalid watch-blockers arguments; stable IDs, Zellij version, and Zellij pane ID are required")?;
    let paths = CorePaths::discover()?;
    paths.prepare()?;
    let mut registry = HostRegistry::open(paths.registry_path())?;
    registry.ensure_local_host(&local_display_name())?;
    let (cancel_tx, cancel_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut stdin = io::stdin();
        let mut buffer = [0_u8; 256];
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    let _ = cancel_tx.send(());
                    return;
                }
                Ok(_) => {}
            }
        }
    });
    watch_blockers_cancellable(&paths, &registry, &arguments, io::stdout(), cancel_rx)
        .map_err(Into::into)
}

fn run_session_lease(arguments: impl IntoIterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let arguments = SessionLeaseArgs::parse(arguments)
        .ok_or("invalid session-lease arguments; workspace and session IDs are required")?;
    let paths = CorePaths::discover()?;
    paths.prepare()?;
    let mut registry = HostRegistry::open(paths.registry_path())?;
    registry.ensure_local_host(&local_display_name())?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    hold_session_lease(&paths, &registry, &arguments, stdin.lock(), stdout.lock())
        .map_err(Into::into)
}

fn local_display_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| fs::read_to_string("/etc/hostname").ok())
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "local-host".to_owned())
}
