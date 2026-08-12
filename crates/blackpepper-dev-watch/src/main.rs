mod bundle;
mod config;
mod process;
mod source;
mod supervisor;

const USAGE: &str = "\
usage: scripts/dev-watch.sh

Build and run a temporary Blackpepper source client, then rebuild it after
Rust source or manifest changes. Nothing is installed and bp-dev is untouched.
The current TUI keeps running on build failure and relaunches only after a
successful immutable bundle is staged under target/. The rebuild log is stored
under target/blackpepper-dev-watch.log.
";

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if matches!(arguments.as_slice(), [value] if value == "-h" || value == "--help") {
        print!("{USAGE}");
        return;
    }
    if !arguments.is_empty() {
        eprintln!("Error: usage: scripts/dev-watch.sh (try --help)");
        std::process::exit(2);
    }
    let result = config::Config::load().and_then(supervisor::run);
    match result {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("Error: {error}");
            std::process::exit(1);
        }
    }
}
