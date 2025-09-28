mod common;
mod local;
/// Remote trust implementation for cross-thread execution.
pub mod remote;

pub use common::{Trust, TrustLike};
pub use local::Local;
pub use remote::Remote;
