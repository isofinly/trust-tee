//! Latch<T>: single-threaded mutual exclusion without atomics or heap allocation.
//!
//! Intended for trustee-local use only. This type does not implement Sync and
//! must only be accessed by fibers running on the same OS thread that owns it.
//! It is analogous to a Mutex<T> but is strictly single-threaded; no atomics,
//! no parking, no poisoning. Re-entrant locking is not allowed.

use core::cell::{Cell, UnsafeCell};
use core::ops::{Deref, DerefMut};

/// Single-threaded latch protecting `T`.
///
/// - !Sync: must not be shared across threads.
/// - Send if `T: Send`.
/// - Lock acquisition is non-blocking; re-entrant locking panics.
pub struct Latch<T> {
    locked: Cell<bool>,
    inner: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for Latch<T> {}

impl<T> Latch<T> {
    /// Create a new latch wrapping `value`.
    #[inline]
    pub const fn new(value: T) -> Self {
        Self {
            locked: Cell::new(false),
            inner: UnsafeCell::new(value),
        }
    }

    /// Attempt to acquire the latch; returns `None` if already locked.
    #[inline]
    pub fn try_lock(&self) -> Option<LatchGuard<'_, T>> {
        if self.locked.get() {
            return None;
        }
        // Single-threaded discipline: no atomics needed.
        self.locked.set(true);
        Some(LatchGuard {
            latch: self,
            _marker: std::marker::PhantomData,
        })
    }

    /// Acquire the latch or panic if already locked (non-reentrant).
    #[inline]
    pub fn lock(&self) -> LatchGuard<'_, T> {
        match self.try_lock() {
            Some(g) => g,
            None => panic!("Latch::lock: already locked (non-reentrant)"),
        }
    }

    /// Get a mutable reference to the inner value without locking.
    ///
    /// Safe because `&mut self` guarantees exclusive access.
    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        // Safety: exclusive borrow of self guarantees exclusivity of inner.
        unsafe { &mut *self.inner.get() }
    }

    /// Consume the latch and return the inner value.
    #[inline]
    pub fn into_inner(self) -> T {
        // Safety: consuming self ensures no outstanding borrows/guards.
        self.inner.into_inner()
    }

    /// Is the latch currently locked?
    #[inline]
    pub fn is_locked(&self) -> bool {
        self.locked.get()
    }
}

/// Guard returned from `Latch::lock`/`try_lock`, releases on drop.
///
/// The guard is !Send and !Sync to keep it on the trustee thread.
pub struct LatchGuard<'a, T> {
    latch: &'a Latch<T>,
    _marker: std::marker::PhantomData<*const ()>,
}

impl<'a, T> Deref for LatchGuard<'a, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &Self::Target {
        // Safety: guard holds exclusive logical access.
        unsafe { &*self.latch.inner.get() }
    }
}

impl<'a, T> DerefMut for LatchGuard<'a, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // Safety: guard holds exclusive logical access.
        unsafe { &mut *self.latch.inner.get() }
    }
}

impl<'a, T> Drop for LatchGuard<'a, T> {
    #[inline]
    fn drop(&mut self) {
        debug_assert!(
            self.latch.locked.get(),
            "LatchGuard dropped when not locked"
        );
        self.latch.locked.set(false);
    }
}
