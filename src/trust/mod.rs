mod common;
mod local;
/// Remote trust implementation for cross-thread execution.
pub mod remote;

pub use common::{Local, Remote, Trust, TrustLike};
