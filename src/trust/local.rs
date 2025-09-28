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
    fn apply<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let _guard = DelegatedScopeGuard::enter();
        unsafe { f(&mut *self.inner.get()) }
    }

    #[inline]
    /// Apply `f`, then enqueue `then` to run with the result.
    fn apply_then<R>(&self, f: impl FnOnce(&mut T) -> R, then: impl FnOnce(R)) {
        then(self.apply(f))
    }

    #[inline]
    /// Mutably apply `f` with an extra argument `w`, returning its result.
    fn apply_with<V, R>(&self, f: impl FnOnce(&mut T, V) -> R, w: V) -> R {
        let _guard = DelegatedScopeGuard::enter();
        unsafe { f(&mut *self.inner.get(), w) }
    }

    #[inline]
    /// Borrow the inner `T` immutably to compute `R`.
    fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let _guard = DelegatedScopeGuard::enter();
        unsafe { f(&*self.inner.get()) }
    }
}
