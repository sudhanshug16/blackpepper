//! Host-local viewport blocker monitoring.
//!
//! Full Zellij viewports exist only while an NDJSON record is being matched.
//! The public wire types can carry identifiers and rule metadata, but have no
//! field capable of retaining terminal text.

mod model;
mod monitor;
mod stream;
mod subscription;

pub use model::{BlockerChange, BlockerSource, BlockerTransition, MonitorContext, StreamStats};
pub use monitor::ViewportBlockerMonitor;
pub use stream::{
    consume_subscription, consume_subscription_fallible, MAX_SUBSCRIPTION_LINE_BYTES,
};
pub use subscription::{
    run_host_local_subscription, run_host_local_subscription_cancellable,
    run_host_local_subscription_cancellable_with_health, run_host_local_subscription_fallible,
    HostSubscriptionError,
};

#[cfg(test)]
mod subscription_tests;
#[cfg(test)]
mod tests;
