//! Trustee APIs: local trustee shortcut and explicit remote runtime with
//! per-client SPSC rings, trustee-side registration, and RR burst processing
//! plus minimal backoff to avoid hot spinning under contention.

use core::marker::PhantomData;
use smallvec::SmallVec;
use std::{sync::Arc, thread};

use crate::Trust;
use crate::affinity::{PinConfig, pin_current_thread};
use crate::slots::{Spsc, backoff_step};

/// Local trustee is allocation-free and inline.
pub struct LocalTrustee {
    _private: (),
}

impl LocalTrustee {
    #[inline]
    pub fn new() -> Self {
        Self { _private: () }
    }
    #[inline]
    pub fn entrust<T>(&self, value: T) -> Trust<T> {
        let _ = &self;
        Trust::new_local(value)
    }
}

// Remote ops/responses are POD and allocation-free.
enum RemoteOp<T> {
    Apply(fn(&mut T)),
    MapU64(fn(&mut T) -> u64),
    ApplyBatch(fn(&mut T), u32),
}

enum RemoteResp {
    None,
    U64(u64),
    NoneBatch(u32),
}

struct ClientPair<T> {
    req: Arc<Spsc<RemoteOp<T>>>,
    resp: Arc<Spsc<RemoteResp>>,
}

pub struct RemoteRuntime<T> {
    reg_q: Arc<Spsc<ClientPair<T>>>,
    capacity: usize,
    burst: usize,
    _worker: thread::JoinHandle<()>,
}

impl<T: Send + 'static> RemoteRuntime<T> {
    pub fn spawn(value: T, capacity: usize) -> (Self, RemoteTrust<T>) {
        Self::spawn_with_pin(value, capacity, 64, None)
    }

    pub fn spawn_with_pin(
        value: T,
        capacity: usize,
        burst: usize,
        pin: Option<PinConfig>,
    ) -> (Self, RemoteTrust<T>) {
        let reg_q = Arc::new(Spsc::new(1024));
        let worker_reg = reg_q.clone();

        let _worker = thread::spawn(move || {
            if let Some(cfg) = pin {
                pin_current_thread(&cfg);
            }
            let mut prop = value;
            let mut clients: SmallVec<[ClientPair<T>; 64]> = SmallVec::new();
            let mut start_idx: usize = 0;
            let mut idle_spins = 0u32;

            loop {
                // Drain registrations in bounded chunks.
                let mut drained = 0;
                while drained < 256 {
                    if let Some(cp) = worker_reg.try_pop() {
                        clients.push(cp);
                        drained += 1;
                    } else {
                        break;
                    }
                }

                if clients.is_empty() {
                    idle_spins = backoff_step(idle_spins);
                    continue;
                }

                let n = clients.len();
                let mut progressed = false;

                for offs in 0..n {
                    let idx = (start_idx + offs) % n;
                    let cp = &clients[idx];

                    let mut processed = 0usize;
                    while processed < burst {
                        match cp.req.try_pop() {
                            Some(RemoteOp::Apply(f)) => {
                                f(&mut prop);
                                let _ = cp.resp.try_push(RemoteResp::None);
                                processed += 1;
                                progressed = true;
                            }
                            Some(RemoteOp::MapU64(f)) => {
                                let v = f(&mut prop);
                                let _ = cp.resp.try_push(RemoteResp::U64(v));
                                processed += 1;
                                progressed = true;
                            }
                            Some(RemoteOp::ApplyBatch(f, nrep)) => {
                                for _ in 0..nrep {
                                    f(&mut prop);
                                }
                                let _ = cp.resp.try_push(RemoteResp::NoneBatch(nrep));
                                processed += 1;
                                progressed = true;
                            }
                            None => break,
                        }
                    }

                    if processed > 0 {
                        start_idx = (idx + 1) % n;
                    }
                }

                if progressed {
                    idle_spins = 0;
                } else {
                    idle_spins = backoff_step(idle_spins);
                }
            }
        });

        let rt = RemoteRuntime {
            reg_q,
            capacity,
            burst,
            _worker,
        };
        let handle = rt.handle();
        (rt, handle)
    }

    #[inline]
    pub fn handle(&self) -> RemoteTrust<T> {
        let req = Arc::new(Spsc::new(self.capacity));
        let resp = Arc::new(Spsc::new(self.capacity));
        self.reg_q.push_backoff(ClientPair {
            req: req.clone(),
            resp: resp.clone(),
        });
        RemoteTrust {
            req_q: req,
            resp_q: resp,
            _phantom: PhantomData,
        }
    }
}

pub struct RemoteTrust<T> {
    pub(crate) req_q: Arc<Spsc<RemoteOp<T>>>,
    pub(crate) resp_q: Arc<Spsc<RemoteResp>>,
    pub(crate) _phantom: PhantomData<T>,
}

impl<T: Send + 'static> RemoteTrust<T> {
    #[inline]
    pub fn apply_mut(&self, f: fn(&mut T)) {
        // Push with backoff if full.
        self.req_q.push_backoff(RemoteOp::Apply(f));
        // Pop with backoff if empty.
        let mut spins = 0u32;
        loop {
            if let Some(RemoteResp::None) = self.resp_q.try_pop() {
                return;
            }
            spins = backoff_step(spins);
        }
    }

    #[inline]
    pub fn apply_map_u64(&self, f: fn(&mut T) -> u64) -> u64 {
        self.req_q.push_backoff(RemoteOp::MapU64(f));
        let mut spins = 0u32;
        loop {
            if let Some(RemoteResp::U64(v)) = self.resp_q.try_pop() {
                return v;
            }
            spins = backoff_step(spins);
        }
    }

    #[inline]
    pub fn apply_batch_mut(&self, f: fn(&mut T), n: u32) {
        self.req_q.push_backoff(RemoteOp::ApplyBatch(f, n));
        let mut spins = 0u32;
        loop {
            if let Some(RemoteResp::NoneBatch(_n)) = self.resp_q.try_pop() {
                return;
            }
            spins = backoff_step(spins);
        }
    }
}
