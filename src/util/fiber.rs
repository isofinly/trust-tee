//! Minimal fiber scaffolding with no TLS; local path runs inline.

/// Guard for entering a delegated scope in fiber execution.
pub struct DelegatedScopeGuard {
    _priv: (),
    _marker: std::marker::PhantomData<*const ()>,
}

/// Enter a delegated scope. Used for testing.
impl DelegatedScopeGuard {
    /// Enter a delegated scope. Used for testing.
    #[inline]
    pub fn enter() -> Self {
        Self {
            _priv: (),
            _marker: std::marker::PhantomData,
        }
    }
}

impl Drop for DelegatedScopeGuard {
    #[inline]
    fn drop(&mut self) {}
}

/// Enqueue a continuation to run after the current operation.
#[inline]
pub fn enqueue_then<U>(then: impl FnOnce(U), value: U) {
    then(value)
}
