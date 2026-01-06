//! Terminal session management for embedded PTY.
//!
//! This module owns all terminal-related concerns:
//! - PTY spawning and lifecycle (pty.rs)
//! - VT100 parsing and screen state
//! - Rendering terminal content to ratatui (render.rs)
//! - Input encoding for keys and mouse events (input.rs)
//! - Provider-agnostic hooks for future extensions (hooks.rs)
//!
//! The terminal is intentionally generic: it can run any CLI program
//! (shells, AI agents, etc.) without provider-specific logic.

mod input;
mod pty;
mod render;

pub mod hooks;

pub use input::{key_event_to_bytes, mouse_event_to_bytes};
pub use pty::TerminalSession;
