#![forbid(unsafe_op_in_unsafe_fn)]
#![deny(missing_docs)]
//! Trust<T> MVP with local trustee shortcut and an explicit remote runtime handle.
//!
//! - Local path: zero per-operation allocation, no TLS, closures run inline.
//! - Remote path: RemoteRuntime spawns a trustee thread with prebuilt SPSC
//!   queues; requests/responses are FIFO so apply-style round trips need no IDs.

mod affinity;
mod fiber;
mod latch;
mod slots;
mod trustee;

pub use affinity::PinConfig;
pub use latch::Latch;
pub use trustee::{LocalTrustee, RemoteRuntime, RemoteTrust};

use core::cell::UnsafeCell;

/// A trust over a property `T` executed by its local trustee.
///
/// Local path: closures run inline, must not block/yield.
pub struct Trust<T> {
    inner: UnsafeCell<T>,
}

// Trust is Send if T: Send; !Sync to prevent accidental sharing.
unsafe impl<T: Send> Send for Trust<T> {}

impl<T> Trust<T> {
    /// Create a new local trust.
    #[inline]
    pub fn new_local(value: T) -> Self {
        Self {
            inner: UnsafeCell::new(value),
        }
    }

    /// Synchronous, non-blocking delegated apply (local shortcut).
    #[inline]
    pub fn apply<U>(&self, f: impl FnOnce(&mut T) -> U) -> U {
        let _guard = crate::fiber::DelegatedScopeGuard::enter();
        // Safety: serialized by contract (local trustee executes inline).
        unsafe { f(&mut *self.inner.get()) }
    }

    /// Non-blocking style with an immediate continuation (local shortcut).
    #[inline]
    pub fn apply_then<U>(&self, f: impl FnOnce(&mut T) -> U, then: impl FnOnce(U)) {
        let out = self.apply(f);
        crate::fiber::enqueue_then(then, out);
    }

    /// Apply with an explicit additional argument (local shortcut).
    #[inline]
    pub fn apply_with<V, U>(&self, f: impl FnOnce(&mut T, V) -> U, w: V) -> U {
        let _guard = crate::fiber::DelegatedScopeGuard::enter();
        unsafe { f(&mut *self.inner.get(), w) }
    }

    /// Read-only projection helper for tests/diagnostics.
    #[inline]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let _guard = crate::fiber::DelegatedScopeGuard::enter();
        unsafe { f(&*self.inner.get()) }
    }
}
