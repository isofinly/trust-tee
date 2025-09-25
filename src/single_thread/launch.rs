use crate::single_thread::Latch;
use crate::trust::Trust;
use crate::util::fiber::{DelegatedScopeGuard, enqueue_then};

impl<U> Trust<Latch<U>> {
    /// Execute `f` inside the latch's critical section on the local trustee.
    ///
    /// Zero-alloc;
    /// TODO: panics if re-entered while already holding the latch.
    #[inline]
    pub fn lock_apply<R>(&self, f: impl FnOnce(&mut U) -> R) -> R {
        let _guard = DelegatedScopeGuard::enter();
        // Safety: local trustee shortcut; serialized by contract.
        let latch = unsafe { &mut *self.inner.get() };
        let mut g = latch.lock();
        f(&mut *g)
    }

    /// Lock + continuation variant; runs `then` immediately after `f`.
    #[inline]
    pub fn lock_apply_then<R>(&self, f: impl FnOnce(&mut U) -> R, then: impl FnOnce(R)) {
        let r = self.lock_apply(f);
        enqueue_then(then, r);
    }

    /// Lock + an explicit out-of-band argument; no serialization on local path.
    #[inline]
    pub fn lock_apply_with<V, R>(&self, f: impl FnOnce(&mut U, V) -> R, w: V) -> R {
        let _guard = DelegatedScopeGuard::enter();
        let latch = unsafe { &mut *self.inner.get() };
        let mut g = latch.lock();
        f(&mut *g, w)
    }

    /// Inspect immutably while holding the latch; handy for diagnostics/tests.
    #[inline]
    pub fn lock_with<R>(&self, f: impl FnOnce(&U) -> R) -> R {
        let _guard = DelegatedScopeGuard::enter();
        let latch = unsafe { &mut *self.inner.get() };
        let g = latch.lock();
        f(&*g)
    }

    /// Launch a function inside the latch's critical section on the local trustee.
    #[inline]
    pub fn launch<R>(&self, f: impl FnOnce(&mut U) -> R) -> R {
        unimplemented!("launch is not implemented")
    }

    /// Launch + continuation variant; runs `then` immediately after `f`.
    #[inline]
    pub fn launch_then<R>(&self, f: impl FnOnce(&mut U) -> R, then: impl FnOnce(R)) {
        unimplemented!("launch is not implemented")
    }
}
