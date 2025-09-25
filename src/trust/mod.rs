mod facade;
mod local;
/// Remote trust implementation for cross-thread execution.
pub mod remote;

pub use facade::TrustLike;
pub use local::Local as Trust;
pub use remote::Remote;
