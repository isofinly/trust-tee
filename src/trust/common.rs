/// A trait for types that can manage trust over a property.
pub trait TrustLike {
    /// The type of value being managed.
    type Value;

    /// Create a new trust instance holding `value`.
    fn entrust(value: Self::Value) -> Self;
    /// Mutably apply `f` to the inner value and return its result.
    fn apply<R>(&self, f: impl FnOnce(&mut Self::Value) -> R) -> R;
    /// Apply `f`, then enqueue `then` to run with the result.
    fn apply_then<R>(&self, f: impl FnOnce(&mut Self::Value) -> R, then: impl FnOnce(R));
    /// Mutably apply `f` with an extra argument `w`, returning its result.
    fn apply_with<V, R>(&self, f: impl FnOnce(&mut Self::Value, V) -> R, w: V) -> R;
    /// Borrow the inner value immutably to compute `R`.
    fn with<R>(&self, f: impl FnOnce(&Self::Value) -> R) -> R;
}

/// A generic wrapper around any implementation of `TrustLike`.
pub struct Trust<T: TrustLike> {
    inner: T,
}

impl<T: TrustLike> Trust<T> {
    /// Create a new `Trust<T>` wrapper around the given implementation.
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Create a new `Trust<T>` by entrusting a value to the implementation.
    pub fn entrust(value: T::Value) -> Self {
        Self {
            inner: T::entrust(value),
        }
    }

    /// Mutably apply `f` to the inner value and return its result.
    pub fn apply<R>(&self, f: impl FnOnce(&mut T::Value) -> R) -> R {
        self.inner.apply(f)
    }

    /// Apply `f`, then enqueue `then` to run with the result.
    pub fn apply_then<R>(&self, f: impl FnOnce(&mut T::Value) -> R, then: impl FnOnce(R)) {
        self.inner.apply_then(f, then)
    }

    /// Mutably apply `f` with an extra argument `w`, returning its result.
    pub fn apply_with<V, R>(&self, f: impl FnOnce(&mut T::Value, V) -> R, w: V) -> R {
        self.inner.apply_with(f, w)
    }

    /// Borrow the inner value immutably to compute `R`.
    pub fn with<R>(&self, f: impl FnOnce(&T::Value) -> R) -> R {
        self.inner.with(f)
    }
}

// Specialized implementations for Latch<T>
impl<U> Trust<crate::trust::Local<crate::single_thread::Latch<U>>> {
    /// Execute `f` inside the latch's critical section on the local trustee.
    #[inline]
    pub fn lock_apply<R>(&self, f: impl FnOnce(&mut U) -> R) -> R {
        self.inner.lock_apply(f)
    }

    /// Lock + continuation variant; runs `then` immediately after `f`.
    #[inline]
    pub fn lock_apply_then<R>(&self, f: impl FnOnce(&mut U) -> R, then: impl FnOnce(R)) {
        self.inner.lock_apply_then(f, then)
    }

    /// Lock + an explicit out-of-band argument; no serialization on local path.
    #[inline]
    pub fn lock_apply_with<V, R>(&self, f: impl FnOnce(&mut U, V) -> R, w: V) -> R {
        self.inner.lock_apply_with(f, w)
    }

    /// Inspect immutably while holding the latch; handy for diagnostics/tests.
    #[inline]
    pub fn lock_with<R>(&self, f: impl FnOnce(&U) -> R) -> R {
        self.inner.lock_with(f)
    }
}
