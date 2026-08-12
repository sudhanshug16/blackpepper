use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Blackpepper could not start: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args();
    let program = arguments
        .next()
        .and_then(|value| {
            std::path::Path::new(&value)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "bp".to_owned());
    match arguments.next().as_deref() {
        Some("--version" | "-V" | "-v") if arguments.next().is_none() => {
            println!("blackpepper {}", blackpepper::BUILD_ID);
            Ok(())
        }
        Some("--help" | "-h") if arguments.next().is_none() => {
            println!("Blackpepper {}", blackpepper::BUILD_ID);
            println!("Remote-first local and SSH agent workspaces backed by Zellij.");
            println!();
            println!("Usage: {program}");
            println!();
            println!("Commands are entered inside the client. Use :help to list them.");
            Ok(())
        }
        Some(argument) => Err(format!(
            "unexpected argument {argument:?}; {program} has no command-line subcommands"
        )
        .into()),
        None => blackpepper::client::run(),
    }
}
