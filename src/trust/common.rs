/// A trait for types that can manage trust over a property.
pub trait TrustLike {
    /// The type of value being managed.
    type Value;

    /// Create a new trust instance holding `value`.
    fn entrust(value: Self::Value) -> Self;
    /// Mutably apply `f` to the inner value and return its result.
    /// Only pure values may cross the channel; `F` must be `Send + 'static`.
    fn apply<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Self::Value) -> R + Send + 'static,
        R: Send + 'static;
    /// Apply `f`, then enqueue `then` to run with the result.
    fn apply_then<F, R>(&self, f: F, then: impl FnOnce(R))
    where
        F: FnOnce(&mut Self::Value) -> R + Send + 'static,
        R: Send + 'static;
    /// Mutably apply `f` with a pure value `w` serialized over the channel.
    /// `V` must be serde-serializable and -deserializable; no references or pointers may cross.
    fn apply_with<F, V, R>(&self, f: F, w: V) -> R
    where
        F: FnOnce(&mut Self::Value, V) -> R + Send + 'static,
        V: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
        R: Send + 'static;
    /// Borrow the inner value immutably to compute `R`.
    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Self::Value) -> R + Send + 'static,
        R: Send + 'static;
    /// Apply a mutation `n` times on the inner value and wait for completion.
    fn apply_batch_mut(&self, f: fn(&mut Self::Value), n: u8);
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
    pub fn apply<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T::Value) -> R + Send + 'static,
        R: Send + 'static,
    {
        self.inner.apply::<F, R>(f)
    }

    /// Apply `f`, then enqueue `then` to run with the result.
    pub fn apply_then<F, R>(&self, f: F, then: impl FnOnce(R))
    where
        F: FnOnce(&mut T::Value) -> R + Send + 'static,
        R: Send + 'static,
    {
        self.inner.apply_then::<F, R>(f, then)
    }

    /// Mutably apply `f` with an extra argument `w`, returning its result.
    pub fn apply_with<F, V, R>(&self, f: F, w: V) -> R
    where
        F: FnOnce(&mut T::Value, V) -> R + Send + 'static,
        V: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
        R: Send + 'static,
    {
        self.inner.apply_with::<F, V, R>(f, w)
    }

    /// Borrow the inner value immutably to compute `R`.
    pub fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T::Value) -> R + Send + 'static,
        R: Send + 'static,
    {
        self.inner.with::<F, R>(f)
    }

    /// Apply a mutation `n` times on the inner value and wait for completion.
    pub fn apply_batch_mut(&self, f: fn(&mut T::Value), n: u8) {
        self.inner.apply_batch_mut(f, n);
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
