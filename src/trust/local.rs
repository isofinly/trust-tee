use crate::util::fiber::DelegatedScopeGuard;
use core::cell::UnsafeCell;

/// A trust over a property `T` executed by its local trustee.
pub struct Local<T> {
    pub(crate) inner: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for Local<T> {}

impl<T> super::common::TrustLike for Local<T> {
    type Value = T;
    #[inline]
    /// Create a `Trust<T>` (local) holding `value` managed by the local trustee.
    fn entrust(value: T) -> Self {
        Self {
            inner: UnsafeCell::new(value),
        }
    }

    #[inline]
    /// Mutably apply `f` to the inner `T` and return its result.
    fn apply<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R + Send + 'static,
        R: Send + 'static,
    {
        let _guard = DelegatedScopeGuard::enter();
        unsafe { f(&mut *self.inner.get()) }
    }

    #[inline]
    /// Apply `f`, then enqueue `then` to run with the result.
    fn apply_then<F, R>(&self, f: F, then: impl FnOnce(R))
    where
        F: FnOnce(&mut T) -> R + Send + 'static,
        R: Send + 'static,
    {
        then(self.apply(f))
    }

    #[inline]
    /// Mutably apply `f` with an extra argument `w`, returning its result.
    fn apply_with<F, V, R>(&self, f: F, w: V) -> R
    where
        F: FnOnce(&mut T, V) -> R + Send + 'static,
        V: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
        R: Send + 'static,
    {
        let _guard = DelegatedScopeGuard::enter();
        unsafe { f(&mut *self.inner.get(), w) }
    }

    #[inline]
    /// Borrow the inner `T` immutably to compute `R`.
    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R + Send + 'static,
        R: Send + 'static,
    {
        let _guard = DelegatedScopeGuard::enter();
        unsafe { f(&*self.inner.get()) }
    }

    #[inline]
    /// Apply a mutation `n` times on the inner value and wait for completion.
    fn apply_batch_mut(&self, f: fn(&mut T), n: u8) {
        let _guard = DelegatedScopeGuard::enter();
        unsafe {
            let ptr = self.inner.get();
            // TODO: Actually batch
            for _ in 0..n {
                f(&mut *ptr);
            }
        }
    }
}
