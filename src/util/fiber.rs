
//! Fiber abstraction to support Miri testing.
//!
//! Under normal compilation, this re-exports `may::coroutine` types.
//! Under `cfg(miri)`, this uses `std::thread` to simulate fibers, as `may` is not Miri-compatible.

#[cfg(not(miri))]
pub use may::coroutine::{Builder, scope, yield_now, JoinHandle, sleep};
#[cfg(not(miri))]
pub use may::sync::mpsc::{channel, Sender, Receiver};


use core::cell::Cell;

thread_local! {
    static IN_DELEGATED_CONTEXT: Cell<bool> = Cell::new(false);
}

/// Returns true if the current thread is executing within a delegated context.
pub fn is_in_delegated_context() -> bool {
    IN_DELEGATED_CONTEXT.with(|c| c.get())
}

/// Guard for delegated scopes.
pub struct DelegatedScopeGuard {
    _private: (),
}

impl DelegatedScopeGuard {
    /// Enter the delegated scope.
    pub fn enter() -> Self {
        IN_DELEGATED_CONTEXT.with(|c| c.set(true));
        Self { _private: () }
    }
}

impl Drop for DelegatedScopeGuard {
    fn drop(&mut self) {
        IN_DELEGATED_CONTEXT.with(|c| c.set(false));
    }
}




/// Enqueue a continuation to run after a result is available.
/// For now, this just runs the continuation immediately.
pub fn enqueue_then<R>(then: impl FnOnce(R), result: R) {
    then(result)
}

#[cfg(miri)]
/// Miri-compatible fiber implementation using system threads.
pub mod miri_impl {
    use std::thread;
    use std::time::Duration;
    use std::io;

    pub use std::thread::{yield_now, JoinHandle, Scope};
    pub use std::sync::mpsc::{channel, Sender, Receiver};

    /// Builder for spawning threads (fibers).
    pub struct Builder {
        inner: thread::Builder,
    }

    impl Builder {
        /// Create a new builder.
        pub fn new() -> Self {
            Self {
                inner: thread::Builder::new(),
            }
        }

        /// Set the name of the thread.
        pub fn name(self, name: String) -> Self {
            Self {
                inner: self.inner.name(name),
            }
        }

        /// Set the stack size (ignored by some implementations).
        pub fn stack_size(self, size: usize) -> Self {
            Self {
                inner: self.inner.stack_size(size),
            }
        }

        /// Spawn a new thread (simulating a fiber).
        pub unsafe fn spawn<F, T>(self, f: F) -> io::Result<JoinHandle<T>>
        where
            F: FnOnce() -> T + Send + 'static,
            T: Send + 'static,
        {
            self.inner.spawn(f)
        }
    }

    /// Create a scope for spawning scoped threads.
    pub fn scope<'env, F, T>(f: F) -> T
    where
        F: for<'scope> FnOnce(&'scope Scope<'scope, 'env>) -> T,
    {
        thread::scope(f)
    }

    /// Sleep for the given duration.
    pub fn sleep(dur: Duration) {
        thread::sleep(dur);
    }
}

#[cfg(miri)]
pub use miri_impl::*;
