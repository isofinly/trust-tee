//! Trustee APIs: local trustee shortcut and an explicit remote runtime handle.
//!
//! RemoteRuntime owns prebuilt SPSC queues and a trustee thread per property.
//! The remote API is constrained to non-capturing function items to remain
//! allocation-free per operation and avoid transferring captured environments.

use crate::{
    affinity::{PinConfig, pin_current_thread},
    slots::Spsc,
};
use crossbeam_queue::ArrayQueue;
use std::{marker::PhantomData, sync::Arc, thread, time::Duration};

use crate::Trust;

/// Handle to the local trustee on the current OS thread.
pub struct LocalTrustee {
    _private: (),
}

impl LocalTrustee {
    /// Construct a local trustee handle.
    #[inline]
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Entrust a property `T` to this local trustee.
    #[inline]
    pub fn entrust<T>(&self, value: T) -> Trust<T> {
        let _ = &self;
        Trust::new_local(value)
    }
}

/// Remote operation variants kept allocation-free per op.
pub(crate) enum RemoteOp<T> {
    /// Mutate-only operation: fn(&mut T)
    Apply(fn(&mut T)),
    /// Map to u64: fn(&mut T) -> u64
    MapU64(fn(&mut T) -> u64),
}

/// Response variants aligned FIFO with requests.
pub(crate) enum RemoteResp {
    None,
    U64(u64),
}

/// Remote runtime handle to a remotely entrusted property `T`.
///
/// API is constrained to non-capturing function items to remain allocation-free.
pub struct RemoteRuntime<T> {
    req_q: Arc<Spsc<RemoteOp<T>>>,
    resp_q: Arc<Spsc<RemoteResp>>,
    _worker: thread::JoinHandle<()>,
}

impl<T: Send + 'static> RemoteRuntime<T> {
    /// Spawn a trustee OS thread for `value` with fixed-capacity SPSC queues.
    ///
    /// Capacity should be chosen to absorb bursts; per-op is allocation-free.
    pub fn spawn(value: T, capacity: usize) -> (Self, RemoteTrust<T>) {
        Self::spawn_with_pin(value, capacity, None)
    }

    /// Spawn with optional pinning configuration.
    pub fn spawn_with_pin(
        value: T,
        capacity: usize,
        pin: Option<PinConfig>,
    ) -> (Self, RemoteTrust<T>) {
        let req_q = Arc::new(Spsc::new(capacity));
        let resp_q = Arc::new(Spsc::new(capacity));

        let worker_req = req_q.clone();
        let worker_resp = resp_q.clone();

        let _worker = thread::spawn(move || {
            if let Some(cfg) = pin {
                pin_current_thread(&cfg);
            }
            let mut prop = value;
            let mut idle = 0u32;
            loop {
                if let Some(op) = worker_req.try_pop() {
                    idle = 0;
                    match op {
                        RemoteOp::Apply(f) => {
                            f(&mut prop);
                            let _ = worker_resp.try_push(RemoteResp::None);
                        }
                        RemoteOp::MapU64(f) => {
                            let v = f(&mut prop);
                            let _ = worker_resp.try_push(RemoteResp::U64(v));
                        }
                    }
                } else {
                    idle = idle.saturating_add(1);
                    if idle < 10 {
                        core::hint::spin_loop();
                    } else if idle < 100 {
                        thread::yield_now();
                    } else {
                        thread::sleep(Duration::from_micros(50));
                    }
                }
            }
        });

        let rt = RemoteRuntime {
            req_q: req_q.clone(),
            resp_q: resp_q.clone(),
            _worker,
        };
        let handle = RemoteTrust {
            req_q,
            resp_q,
            _phantom: PhantomData,
        };
        (rt, handle)
    }

    /// Access to the client-side handle (clone without extra allocation).
    #[inline]
    pub fn handle(&self) -> RemoteTrust<T> {
        RemoteTrust {
            req_q: self.req_q.clone(),
            resp_q: self.resp_q.clone(),
            _phantom: PhantomData,
        }
    }
}

/// A client handle to a remotely entrusted property `T`.
///
/// API is constrained to non-capturing function items to remain allocation-free.
pub struct RemoteTrust<T> {
    pub(crate) req_q: Arc<Spsc<RemoteOp<T>>>,
    pub(crate) resp_q: Arc<Spsc<RemoteResp>>,
    pub(crate) _phantom: PhantomData<T>,
}

impl<T: Send + 'static> RemoteTrust<T> {
    /// Apply a mutate-only operation: fn(&mut T).
    ///
    /// Blocks until the trustee pushes its matching None response.
    #[inline]
    pub fn apply_mut(&self, f: fn(&mut T)) {
        // Push request (spin until space is available).
        while self.req_q.try_push(RemoteOp::Apply(f)).is_err() {
            std::hint::spin_loop();
        }
        // Pop matching response.
        loop {
            if let Some(resp) = self.resp_q.try_pop() {
                match resp {
                    RemoteResp::None => return,
                    RemoteResp::U64(_) => continue, // unexpected type; skip
                }
            } else {
                std::hint::spin_loop();
            }
        }
    }

    /// Apply a mapping operation to u64: fn(&mut T) -> u64, returning the value.
    #[inline]
    pub fn apply_map_u64(&self, f: fn(&mut T) -> u64) -> u64 {
        while self.req_q.try_push(RemoteOp::MapU64(f)).is_err() {
            std::hint::spin_loop();
        }
        loop {
            if let Some(resp) = self.resp_q.try_pop() {
                if let RemoteResp::U64(v) = resp {
                    return v;
                }
            } else {
                std::hint::spin_loop();
            }
        }
    }
}
