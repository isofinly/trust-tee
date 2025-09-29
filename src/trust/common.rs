/// A trait for types that can manage trust over a property.
pub trait TrustLike {
    /// The type of value being managed.
    type Value;

    /// Create a new trust instance holding `value`.
    fn entrust(value: Self::Value) -> Self;
    /// Get underlying value and drop the trust instance.
    fn into_inner(self) -> Self::Value;
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
    pub(crate) inner: T,
}

impl<T: TrustLike> Trust<T> {
    /// Create a new `Trust<T>` wrapper around the given implementation.
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Get underlying value and drop the trust instance.
    pub fn into_inner(self) -> T::Value {
        self.inner.into_inner()
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

impl<T: TrustLike> core::fmt::Display for Trust<T>
where
    T::Value: core::fmt::Display,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = self.inner.with(|v| format!("{}", v));
        f.write_str(&s)
    }
}

impl<T: TrustLike> core::fmt::Debug for Trust<T>
where
    T::Value: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = self.inner.with(|v| format!("{:?}", v));
        f.debug_tuple("Trust").field(&s).finish()
    }
}

// Specialized implementations for typed construction
impl<T> Trust<super::local::Local<T>> {
    /// Create a new `Trust<Local<T>>` holding `value` managed by the local trustee.
    pub fn entrust(value: T) -> Self {
        Self {
            inner: super::local::Local::entrust(value),
        }
    }
}

impl<T: Send + 'static> Trust<super::remote::Remote<T>> {
    /// Create a new `Trust<Remote<T>>` holding `value` managed by a remote trustee.
    pub fn entrust(value: T) -> Self {
        Self {
            inner: super::remote::Remote::entrust(value),
        }
    }

    /// Create a new `Trust<Remote<T>>` with pinning configuration for the worker thread.
    pub fn entrust_with_pin(value: T, pin: crate::util::affinity::PinConfig) -> Self {
        use crate::runtime::Runtime;
        let (rt, handle) = Runtime::spawn_with_pin(value, 64, Some(pin));
        Self {
            inner: super::remote::Remote {
                chan: handle.chan.clone(),
                _phantom: core::marker::PhantomData,
                _owner: Some(rt),
                _not_sync: core::marker::PhantomData,
            },
        }
    }
}

// Type aliases for convenience
/// A trust that manages values locally without cross-thread communication.
pub type Local<T> = Trust<super::local::Local<T>>;
/// A trust that manages values remotely using cross-thread communication.
pub type Remote<T> = Trust<super::remote::Remote<T>>;

impl<T: Send + 'static> Clone for Trust<super::remote::Remote<T>> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}
