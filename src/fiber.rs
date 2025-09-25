//! Minimal fiber scaffolding with no TLS; local path runs inline.

pub struct DelegatedScopeGuard {
    _priv: (),
}

/// Enter a delegated scope. Used for testing.
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

#[inline]
pub fn enqueue_then<U>(then: impl FnOnce(U), value: U) {
    then(value)
}
