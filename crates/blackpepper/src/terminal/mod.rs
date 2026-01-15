//! Terminal session management for embedded PTY.
//!
//! This module owns all terminal-related concerns:
//! - PTY spawning and lifecycle (pty.rs)
//! - VT100 parsing and screen state
//! - Rendering terminal content to ratatui (render.rs)
//! - Provider-agnostic hooks for future extensions (hooks.rs)
//!
//! The terminal is intentionally generic: it can run any CLI program
//! (shells, AI agents, etc.) without provider-specific logic.

mod input_modes;
mod pty;
mod render;

pub mod hooks;

pub use input_modes::InputModes;
pub use pty::TerminalSession;
