mod animals;
mod app;
mod commands;
mod config;
mod events;
mod git;
mod keymap;
mod state;
mod terminal;
mod updater;
mod workspaces;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args();
    let program = args.next().unwrap_or_else(|| "blackpepper".to_string());

    updater::apply_staged_update();

    let mut rest: Vec<String> = args.collect();
    if !rest.is_empty() {
        if rest.first().map(String::as_str) == Some("--help")
            || rest.first().map(String::as_str) == Some("-h")
        {
            println!("Usage: {program} [command]");
            println!();
            println!("Commands:");
            for line in commands::command_help_lines_cli() {
                println!("  {line}");
            }
            return Ok(());
        }
        if rest.first().map(String::as_str) == Some("--version")
            || rest.first().map(String::as_str) == Some("-v")
        {
            let result = commands::run_command(
                "version",
                &[],
                &commands::CommandContext {
                    cwd: std::env::current_dir()
                        .unwrap_or_else(|_| std::path::PathBuf::from(".")),
                    repo_root: None,
                    workspace_root: std::path::PathBuf::from(".blackpepper/workspaces"),
                },
            );
            println!("{}", result.message);
            return Ok(());
        }

        let command = rest.remove(0);
        let command = command.strip_prefix(':').unwrap_or(&command).to_string();
        let tokens: Vec<String> = std::iter::once(command)
            .chain(rest.into_iter())
            .collect();
        let input = format!(":{}", tokens.join(" "));
        let parsed = match commands::parse_command(&input) {
            Ok(parsed) => parsed,
            Err(err) => {
                eprintln!("{}", err.error);
                return Ok(());
            }
        };
        let is_cli_exposed = if parsed.args.is_empty() {
            commands::COMMANDS.iter().any(|spec| {
                spec.cli_exposed
                    && (spec.name == parsed.name
                        || spec
                            .name
                            .starts_with(&format!("{} ", parsed.name)))
            })
        } else {
            let full = format!("{} {}", parsed.name, parsed.args[0]);
            commands::COMMANDS
                .iter()
                .any(|spec| spec.name == full && spec.cli_exposed)
        };
        if !is_cli_exposed {
            eprintln!("Command not available in CLI: {}", parsed.name);
            return Ok(());
        }

        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let repo_root = git::resolve_repo_root(&cwd);
        let config_root = repo_root.as_deref().unwrap_or(&cwd);
        let config = config::load_config(config_root);
        if parsed.name == "help" {
            for line in commands::command_help_lines_cli() {
                println!("{}", line);
            }
            return Ok(());
        }

        let result = commands::run_command(
            parsed.name.as_str(),
            &parsed.args,
            &commands::CommandContext {
                cwd,
                repo_root,
                workspace_root: config.workspace.root,
            },
        );
        if result.ok {
            println!("{}", result.message);
        } else {
            eprintln!("{}", result.message);
        }
        return Ok(());
    }

    let _ = updater::check_for_update();
    app::run()
}
