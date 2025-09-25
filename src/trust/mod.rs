mod local;
/// Remote trust implementation for cross-thread execution.
pub mod remote;

pub use local::Local as Trust;
