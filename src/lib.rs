#![forbid(unsafe_op_in_unsafe_fn)]
#![deny(missing_docs)]

mod affinity;
mod fiber;
mod latch;
mod slots;
mod trust;
mod trustee;

pub use affinity::{PinConfig, pin_current_thread};
use core::cell::UnsafeCell;
pub use latch::Latch;
pub use trust::*;
pub use trustee::{LocalTrustee, RemoteRuntime, RemoteTrust};
