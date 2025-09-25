use core::cell::UnsafeCell;

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

impl<U> Trust<crate::latch::Latch<U>> {
    /// Execute `f` inside the latch’s critical section on the local trustee.
    ///
    /// Zero-alloc; panics if re-entered while already holding the latch.
    #[inline]
    pub fn lock_apply<R>(&self, f: impl FnOnce(&mut U) -> R) -> R {
        let _guard = crate::fiber::DelegatedScopeGuard::enter();
        // Safety: local trustee shortcut; serialized by contract.
        let latch = unsafe { &mut *self.inner.get() };
        let mut g = latch.lock();
        f(&mut *g)
    }

    /// Lock + continuation variant; runs `then` immediately after `f`.
    #[inline]
    pub fn lock_apply_then<R>(&self, f: impl FnOnce(&mut U) -> R, then: impl FnOnce(R)) {
        let r = self.lock_apply(f);
        crate::fiber::enqueue_then(then, r);
    }

    /// Lock + an explicit out-of-band argument; no serialization on local path.
    #[inline]
    pub fn lock_apply_with<V, R>(&self, f: impl FnOnce(&mut U, V) -> R, w: V) -> R {
        let _guard = crate::fiber::DelegatedScopeGuard::enter();
        let latch = unsafe { &mut *self.inner.get() };
        let mut g = latch.lock();
        f(&mut *g, w)
    }

    /// Inspect immutably while holding the latch; handy for diagnostics/tests.
    #[inline]
    pub fn lock_with<R>(&self, f: impl FnOnce(&U) -> R) -> R {
        let _guard = crate::fiber::DelegatedScopeGuard::enter();
        let latch = unsafe { &mut *self.inner.get() };
        let g = latch.lock();
        f(&*g)
    }
}
