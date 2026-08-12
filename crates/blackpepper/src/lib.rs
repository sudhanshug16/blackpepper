//! Blackpepper's remote-first workspace client and transient host helper.

/// Exact client/helper compatibility identity.
///
/// Release builds use the package version. The development installer supplies
/// a unique value so a dirty checkout can never reuse or overwrite a release
/// helper that happens to have the same package version.
pub const BUILD_ID: &str = match option_env!("BLACKPEPPER_BUILD_ID") {
    Some(value) => value,
    None => env!("CARGO_PKG_VERSION"),
};

/// Non-production builds compile with an explicit build identity. Installed
/// `bp-dev` and temporary source-watch builds use separate client singletons
/// and provider event stores while sharing the stable workspace/session
/// registry and host-side lifecycle locks.
pub const IS_DEVELOPMENT_BUILD: bool = option_env!("BLACKPEPPER_BUILD_ID").is_some();

/// Temporary source-watch builds are never installed as `bp-dev`.
///
/// The extra compile-time marker gives that short-lived channel its own
/// singleton and provider event database without allowing a runtime
/// environment variable to bypass the one-client-per-channel invariant.
pub const IS_SOURCE_WATCH_BUILD: bool = option_env!("BLACKPEPPER_SOURCE_WATCH_BUILD").is_some();

pub mod agent_status;
pub mod client;
pub mod client_config;
pub mod core;
pub mod host_services;
pub mod ports;
pub mod providers;
pub mod ssh_config;
pub mod status_monitor;
pub mod transport;
pub mod workspace_identity;
pub mod worktrunk;
pub mod zellij;

mod input;
mod keymap;
mod terminal;
#[cfg(test)]
mod test_utils;
