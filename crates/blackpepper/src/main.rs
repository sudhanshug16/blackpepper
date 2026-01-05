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
    app::run()
}
