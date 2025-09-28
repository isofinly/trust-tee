#![forbid(unsafe_op_in_unsafe_fn)]
#![deny(missing_docs)]

//! Trust<T> MVP with: local trustee shortcut, per-client SPSC rings for remote
//! delegation, trustee-side registration + RR burst processing, optional pinning,
//! and a trustee-local Latch<T>. Zero per-op allocation and no TLS.

//! Trust-based concurrency primitives for local and remote execution.

/// Local and remote trust implementations.
pub mod trust;

/// Single-threaded primitives that don't use atomics.
pub mod single_thread;

/// Shared utilities for waiting, affinity, and fiber management.
pub mod util;

/// Remote runtime for cross-thread trust execution.
pub mod runtime;

/// Common re-exports for convenient usage.
pub mod prelude {
    pub use crate::runtime::*;
    pub use crate::single_thread::Latch;
    pub use crate::trust::{Local, Remote, Trust, TrustLike};
    pub use crate::util::{PinConfig, WaitBudget, pin_current_thread};
}
