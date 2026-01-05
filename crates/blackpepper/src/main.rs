mod app;
mod commands;
mod config;
mod events;
mod git;
mod keymap;
mod state;
mod terminal;
mod workspaces;
mod animals;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args();
    let program = args.next().unwrap_or_else(|| "blackpepper".to_string());
    if let Some(command) = args.next() {
        let rest: Vec<String> = args.collect();
        if command == "init" {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let repo_root = git::resolve_repo_root(&cwd);
            let config_root = repo_root.as_deref().unwrap_or(&cwd);
            let config = config::load_config(config_root);
            let result = commands::run_command(
                "init",
                &rest,
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
        eprintln!("Usage: {} init", program);
        return Ok(());
    }

    app::run()
}
