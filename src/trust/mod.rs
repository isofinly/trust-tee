mod common;
mod local;
/// Remote trust implementation for cross-thread execution.
pub mod remote;

pub use common::{Trust, TrustLike};
pub use local::Local;
pub use remote::Remote;
// pub enum Trust<T> {
//     Local(Local<T>),
//     Remote(Remote<T>),
// }

// impl<T> Trust<T> {
//     pub fn new_local(value: T) -> Self {
//         Trust::Local(LocalTrust::new(value))
//     }

//     pub fn new_remote(value: T) -> Self {
//         Trust::Remote(RemoteTrust::new(value))
//     }
// }
