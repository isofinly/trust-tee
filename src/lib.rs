#![forbid(unsafe_op_in_unsafe_fn)]
#![deny(missing_docs)]

//! Trust<T> MVP with: local trustee shortcut, per-client SPSC rings for remote
//! delegation, trustee-side registration + RR burst processing, optional pinning,
//! and a trustee-local Latch<T>. Zero per-op allocation and no TLS.

mod affinity;
mod fiber;
mod latch;
mod ring;
mod trust;
mod trustee;

pub use crate::trustee::local::LocalTrustee;
pub use crate::trustee::remote::RemoteRuntime;
pub use crate::trustee::remote::RemoteTrust;
pub use affinity::{PinConfig, pin_current_thread};
pub use latch::Latch;
// No longer exposing SPSC in the public API; internals now use ArrayQueue.
pub use trust::*;
