/// Operations that can be sent to a remote runtime.
pub enum Op<T> {
    /// Apply a mutation to the remote property.
    Apply(fn(&mut T)),
    /// Apply a function and return a u64 result.
    MapU64(fn(&mut T) -> u64),
    /// Apply a mutation multiple times.
    ApplyBatch(fn(&mut T), u32),
    /// Signal the worker to terminate cleanly.
    Terminate,
}

/// Responses from a remote runtime.
pub enum Resp {
    /// No return value.
    None,
    /// A u64 return value.
    U64(u64),
    /// Batch operation completed with count.
    NoneBatch(u32),
}
