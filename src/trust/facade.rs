/// Common trait for both local and remote trust implementations.
pub trait TrustLike<T> {
    /// Apply a function to the inner value mutably and return the result.
    fn with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> R;

    /// Apply a function to the inner value immutably and return the result.
    fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R;
}

impl<T> TrustLike<T> for crate::trust::Trust<T> {
    #[inline]
    fn with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        self.apply(f)
    }

    #[inline]
    fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.with(f)
    }
}

impl<T: Send + 'static> TrustLike<T> for crate::trust::Remote<T> {
    #[inline]
    fn with_mut<R>(&self, _f: impl FnOnce(&mut T) -> R) -> R {
        // Remote doesn't support arbitrary closures, only function pointers
        // This is a limitation of the current design for cross-thread safety
        unimplemented!("Remote trust doesn't support arbitrary closures")
    }

    #[inline]
    fn with<R>(&self, _f: impl FnOnce(&T) -> R) -> R {
        // Remote doesn't support arbitrary closures, only function pointers
        // This is a limitation of the current design for cross-thread safety
        unimplemented!("Remote trust doesn't support arbitrary closures")
    }
}

// Note: The user-facing `Trust<T>` type is exported from `trust::mod` as a type alias
// to the local implementation for the local path. Remote is available as `trust::Remote<T>`.
