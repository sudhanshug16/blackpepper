//! Command system for the `:command` interface.
//!
//! This module handles all aspects of the command-mode UI:
//! - Registry: command specs and metadata (registry.rs)
//! - Parsing: tokenizing and validating command input (parse.rs)
//! - Execution: running commands and returning results (exec.rs)
//!
//! Commands follow a `:name subcommand [args]` pattern similar to Vim.
//! The registry provides autocompletion and help text.

mod exec;
mod parse;
mod registry;

pub use exec::{run_command, CommandContext, CommandResult};
pub use parse::{complete_command_input, parse_command};
pub use registry::{command_help_lines_cli, command_hint_lines};

// Re-exports for external use (kept for future API stability)
#[allow(unused_imports)]
pub use parse::{CommandError, CommandMatch, CommandParseResult};
#[allow(unused_imports)]
pub use registry::{command_help_lines, CommandSpec, COMMANDS, TOP_LEVEL_COMMANDS};
