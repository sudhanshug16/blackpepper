//! Best-effort listener discovery and client-local forwarding state.

mod model;
mod probe;

pub use model::{
    choose_initial_local_port, port_is_available, resolve_forward_target, target_is_ambiguous,
    AttributionConfidence, ForwardState, ForwardStatus, PortListener, PortSnapshot,
    ProbeCompleteness, RemotePortTarget,
};
pub use probe::{
    attribute_linux_cwds, failed_probe, parse_linux_ss, parse_macos_lsof, platform_probe,
    ProbeCommand,
};

#[cfg(test)]
mod tests;
