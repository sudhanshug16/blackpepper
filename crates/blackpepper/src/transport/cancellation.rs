use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

thread_local! {
    static ACTIVE: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// Cooperative cancellation inherited by ordinary command waits on the
/// current thread. It is deliberately scoped: interactive mutations keep
/// their existing unknown-result rules, while idempotent reconnect recovery
/// can cancel every child it starts when its connection generation is stale.
#[derive(Clone, Default)]
pub(crate) struct CommandCancellation {
    cancelled: Arc<AtomicBool>,
}

impl CommandCancellation {
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn scoped<T>(&self, work: impl FnOnce() -> T) -> T {
        let previous = ACTIVE.with(|active| active.replace(Some(Arc::clone(&self.cancelled))));
        let _guard = CancellationScope { previous };
        work()
    }

    pub(crate) fn scope_is_active() -> bool {
        active()
    }

    pub(crate) fn scope_is_cancelled() -> bool {
        requested()
    }

    /// Complete a lease-protected idempotent mutation once it starts. A
    /// Zellij tab creation and its focus compensation are one logical action;
    /// interrupting between them would leave visible partial state.
    pub(crate) fn mask_current<T>(work: impl FnOnce() -> T) -> T {
        let previous = ACTIVE.with(|active| active.replace(None));
        let _guard = CancellationScope { previous };
        work()
    }
}

struct CancellationScope {
    previous: Option<Arc<AtomicBool>>,
}

impl Drop for CancellationScope {
    fn drop(&mut self) {
        ACTIVE.with(|active| {
            active.replace(self.previous.take());
        });
    }
}

pub(super) fn requested() -> bool {
    ACTIVE.with(|active| {
        active
            .borrow()
            .as_ref()
            .is_some_and(|cancelled| cancelled.load(Ordering::Acquire))
    })
}

pub(super) fn active() -> bool {
    ACTIVE.with(|active| active.borrow().is_some())
}
