//! Provider-agnostic hooks for future extensions.
//!
//! This module provides extension points for provider-specific behavior
//! without adding provider logic to the core terminal implementation.
//!
//! Currently a placeholder. Future capabilities may include:
//! - Environment variable injection per provider
//! - Output event extraction (e.g., detecting tool calls)
//! - Custom escape sequence handling
//!
//! The terminal stays generic; provider-specific adapters would implement
//! traits defined here and be wired in optionally.

// Allow unused code in this module - these are future extension points
#![allow(dead_code)]

/// Marker trait for provider adapters.
///
/// Providers like Claude, Codex, or OpenCode could implement this
/// to inject custom environment variables or handle special events.
/// Not yet used; exists to document the extension point.
pub trait ProviderAdapter: Send + Sync {
    /// Provider name for identification.
    fn name(&self) -> &str;

    /// Additional environment variables to set when spawning.
    fn extra_env(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    /// Called when output is received; can extract events.
    /// Returns true if the output was consumed (not displayed).
    fn on_output(&mut self, _bytes: &[u8]) -> bool {
        false
    }
}

/// Default no-op adapter for generic shell sessions.
pub struct GenericShell;

impl ProviderAdapter for GenericShell {
    fn name(&self) -> &str {
        "shell"
    }
}
