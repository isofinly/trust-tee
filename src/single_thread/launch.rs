use crate::single_thread::Latch;
use crate::trust::Local;
use crate::util::fiber::enqueue_then;

// TODO: Must be implemented for remote as well
impl<U> Local<Latch<U>> {
    /// Execute `f` inside the latch's critical section on the local trustee.
    ///
    /// Zero-alloc;
    #[inline]
    pub fn lock_apply<R>(&self, f: impl FnOnce(&mut U) -> R) -> R {
        // Safety: local trustee shortcut; serialized by contract.
        let latch = unsafe { &mut *self.inner.inner.get() };
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
        let latch = unsafe { &mut *self.inner.inner.get() };
        let mut g = latch.lock();
        f(&mut *g, w)
    }

    /// Inspect immutably while holding the latch; handy for diagnostics/tests.
    #[inline]
    pub fn lock_with<R>(&self, f: impl FnOnce(&U) -> R) -> R {
        let latch = unsafe { &mut *self.inner.inner.get() };
        let g = latch.lock();
        f(&*g)
    }
}
