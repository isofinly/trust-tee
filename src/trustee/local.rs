use crate::Trust;

/// Local trustee is allocation-free and inline.
pub struct LocalTrustee {
    _private: (),
    _marker: std::marker::PhantomData<*const ()>,
}

impl LocalTrustee {
    #[inline]
    /// Create a new local trustee.
    pub fn new() -> Self {
        Self {
            _private: (),
            _marker: std::marker::PhantomData,
        }
    }
    #[inline]
    /// Entrust a value to its local trustee and return a `Trust<T>` handle.
    pub fn entrust<T>(&self, value: T) -> Trust<T> {
        let _ = &self;
        Trust::new_local(value)
    }
}
