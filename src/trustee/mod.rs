//! Trustee APIs: local trustee shortcut and explicit remote runtime with
//! per-client SPSC rings, trustee-side registration, and RR burst processing
//! plus minimal backoff to avoid hot spinning under contention.

pub mod local;
pub mod remote;
