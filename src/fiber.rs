//! Fiber scaffolding with no TLS and zero heap on the local path.
//!
//! We avoid TLS entirely. Delegated context is represented by a scope guard
//! that uses a thread-bounded, compile-time–only marker. For a full runtime,
//! this module would integrate with a coroutine scheduler explicitly passed
//! around via handles.

/// A zero-sized scope guard marking “delegated context” lexically.
///
/// This is purely structural on the local path and imposes no runtime cost.
/// It prevents accidental nested blocking in debug builds if you add checks.
///
/// Zero-sized lexical guard for “delegated context”.
pub struct DelegatedScopeGuard {
    _priv: (),
}

impl DelegatedScopeGuard {
    #[inline]
    pub fn enter() -> Self {
        Self { _priv: () }
    }
}

impl Drop for DelegatedScopeGuard {
    #[inline]
    fn drop(&mut self) {}
}

/// Inline continuation runner for local path.
#[inline]
pub fn enqueue_then<U>(then: impl FnOnce(U), value: U) {
    then(value)
}
