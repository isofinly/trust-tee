//! Latch<T>: single-threaded mutual exclusion without atomics or heap allocation.

use core::cell::{Cell, UnsafeCell};
use core::ops::{Deref, DerefMut};

pub struct Latch<T> {
    locked: Cell<bool>,
    inner: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for Latch<T> {}

impl<T> Latch<T> {
    #[inline]
    pub const fn new(value: T) -> Self {
        Self {
            locked: Cell::new(false),
            inner: UnsafeCell::new(value),
        }
    }

    #[inline]
    pub fn try_lock(&self) -> Option<LatchGuard<'_, T>> {
        if self.locked.get() {
            return None;
        }
        self.locked.set(true);
        Some(LatchGuard {
            latch: self,
            _marker: std::marker::PhantomData,
        })
    }

    #[inline]
    pub fn lock(&self) -> LatchGuard<'_, T> {
        self.try_lock().expect("Latch::lock: already locked")
    }

    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        unsafe { &mut *self.inner.get() }
    }

    #[inline]
    pub fn into_inner(self) -> T {
        self.inner.into_inner()
    }

    #[inline]
    pub fn is_locked(&self) -> bool {
        self.locked.get()
    }
}

pub struct LatchGuard<'a, T> {
    latch: &'a Latch<T>,
    _marker: std::marker::PhantomData<*const ()>,
}

impl<'a, T> Deref for LatchGuard<'a, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.latch.inner.get() }
    }
}
impl<'a, T> DerefMut for LatchGuard<'a, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.latch.inner.get() }
    }
}
impl<'a, T> Drop for LatchGuard<'a, T> {
    #[inline]
    fn drop(&mut self) {
        debug_assert!(self.latch.locked.get());
        self.latch.locked.set(false);
    }
}
