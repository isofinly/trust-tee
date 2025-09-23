use std::cell::UnsafeCell;

/// A trust over a property `T` executed by its local trustee.
pub struct Trust<T> {
    inner: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for Trust<T> {}

impl<T> Trust<T> {
    #[inline]
    /// Create a `Trust<T>` holding `value` managed by a local trustee.
    pub fn new_local(value: T) -> Self {
        Self {
            inner: UnsafeCell::new(value),
        }
    }

    #[inline]
    /// Mutably apply `f` to the inner `T` and return its result.
    pub fn apply<U>(&self, f: impl FnOnce(&mut T) -> U) -> U {
        let _guard = crate::fiber::DelegatedScopeGuard::enter();
        unsafe { f(&mut *self.inner.get()) }
    }

    #[inline]
    /// Apply `f`, then enqueue `then` to run with the result.
    pub fn apply_then<U>(&self, f: impl FnOnce(&mut T) -> U, then: impl FnOnce(U)) {
        let out = self.apply(f);
        crate::fiber::enqueue_then(then, out);
    }

    #[inline]
    /// Mutably apply `f` with an extra argument `w`, returning its result.
    pub fn apply_with<V, U>(&self, f: impl FnOnce(&mut T, V) -> U, w: V) -> U {
        let _guard = crate::fiber::DelegatedScopeGuard::enter();
        unsafe { f(&mut *self.inner.get(), w) }
    }

    #[inline]
    /// Borrow the inner `T` immutably to compute `R`.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let _guard = crate::fiber::DelegatedScopeGuard::enter();
        unsafe { f(&*self.inner.get()) }
    }
}
