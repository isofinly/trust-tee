use core::marker::PhantomData;
use crossbeam_queue::{ArrayQueue, SegQueue};
use smallvec::SmallVec;
use std::{sync::Arc, thread};

use crate::{
    affinity::{PinConfig, pin_current_thread},
    ring::wait::WaitBudget,
};

// Remote ops/responses are POD and allocation-free.
enum RemoteOp<T> {
    Apply(fn(&mut T)),
    MapU64(fn(&mut T) -> u64),
    ApplyBatch(fn(&mut T), u32),
    // Signal the worker to terminate cleanly.
    Terminate,
}

enum RemoteResp {
    None,
    U64(u64),
    NoneBatch(u32),
}

struct ClientPair<T> {
    req: Arc<ArrayQueue<RemoteOp<T>>>,
    resp: Arc<ArrayQueue<RemoteResp>>,
}

/// Remote runtime hosting a property `T` on a worker thread and serving clients.
pub struct RemoteRuntime<T> {
    reg_q: Arc<SegQueue<ClientPair<T>>>,
    capacity: usize,
    _worker: Option<thread::JoinHandle<()>>,
}

impl<T: Send + 'static> RemoteRuntime<T> {
    /// Spawn a remote runtime worker thread with default burst and no pinning.
    pub fn spawn(value: T, capacity: usize) -> (Self, RemoteTrust<T>) {
        Self::spawn_with_pin(value, capacity, 64, None)
    }

    /// Spawn a remote runtime worker with explicit `burst` and optional pinning.
    pub fn spawn_with_pin(
        value: T,
        capacity: usize,
        burst: usize,
        pin: Option<PinConfig>,
    ) -> (Self, RemoteTrust<T>) {
        let reg_q = Arc::new(SegQueue::new());
        let worker_reg = reg_q.clone();

        let _worker = thread::spawn(move || {
            if let Some(cfg) = pin {
                pin_current_thread(&cfg);
            }
            let mut prop = value;
            let mut clients: SmallVec<[ClientPair<T>; 64]> = SmallVec::new();
            let mut start_idx: usize = 0;
            let mut idle_rounds: u32 = 0;

            loop {
                // Drain registrations in bounded chunks.
                let mut drained = 0;
                while drained < 256 {
                    if let Some(cp) = worker_reg.pop() {
                        clients.push(cp);
                        drained += 1;
                    } else {
                        break;
                    }
                }

                if clients.is_empty() {
                    // Light idle: short spin, then yield; never park.
                    if idle_rounds < 8 {
                        core::hint::spin_loop();
                        idle_rounds += 1;
                    } else {
                        std::thread::yield_now();
                        idle_rounds = 0;
                    }
                    continue;
                }

                let n = clients.len();
                let mut progressed = false;

                for offs in 0..n {
                    let idx = (start_idx + offs) % n;
                    let cp = &clients[idx];

                    let mut processed = 0usize;
                    let local_burst = {
                        let pending = cp.req.len();
                        if pending == 0 {
                            0
                        } else {
                            core::cmp::min(burst, pending)
                        }
                    };
                    while processed < local_burst {
                        match cp.req.pop() {
                            Some(RemoteOp::Apply(f)) => {
                                f(&mut prop);
                                let _ = cp.resp.push(RemoteResp::None);
                                processed += 1;
                                progressed = true;
                            }
                            Some(RemoteOp::MapU64(f)) => {
                                let v = f(&mut prop);
                                let _ = cp.resp.push(RemoteResp::U64(v));
                                processed += 1;
                                progressed = true;
                            }
                            Some(RemoteOp::ApplyBatch(f, nrep)) => {
                                for _ in 0..nrep {
                                    f(&mut prop);
                                }
                                let _ = cp.resp.push(RemoteResp::NoneBatch(nrep));
                                processed += 1;
                                progressed = true;
                            }
                            Some(RemoteOp::Terminate) => {
                                // Break outer loop and exit thread.
                                return;
                            }
                            None => break,
                        }
                    }

                    if processed > 0 {
                        start_idx = (idx + 1) % n;
                    }
                }

                if progressed {
                    idle_rounds = 0;
                } else {
                    if idle_rounds < 8 {
                        core::hint::spin_loop();
                        idle_rounds += 1;
                    } else {
                        std::thread::yield_now();
                        idle_rounds = 0;
                    }
                }
            }
        });

        let rt = RemoteRuntime {
            reg_q,
            capacity,
            _worker: Some(_worker),
        };
        let handle = rt.handle();
        (rt, handle)
    }

    #[inline]
    /// Create a client handle by registering bounded MPMC queues with the worker.
    pub fn handle(&self) -> RemoteTrust<T> {
        let req = Arc::new(ArrayQueue::new(self.capacity));
        let resp = Arc::new(ArrayQueue::new(self.capacity));
        self.reg_q.push(ClientPair {
            req: req.clone(),
            resp: resp.clone(),
        });
        RemoteTrust {
            req,
            resp,
            _phantom: PhantomData,
        }
    }
}

impl<T> Drop for RemoteRuntime<T> {
    fn drop(&mut self) {
        // Ask all registered clients to terminate the worker if any exist.
        // We cannot access clients directly here; instead we register a one-off
        // control client and send a Terminate op.
        let req = Arc::new(ArrayQueue::new(self.capacity));
        let resp = Arc::new(ArrayQueue::new(self.capacity));
        // Best-effort send; if full, spin with small budget.
        {
            let mut budget = WaitBudget::hot();
            loop {
                if req.push(RemoteOp::Terminate).is_ok() {
                    break;
                }
                budget.step();
            }
        }
        self.reg_q.push(ClientPair { req, resp });
        // Join the worker to ensure all allocations are torn down before exit.
        if let Some(handle) = self._worker.take() {
            let _ = handle.join();
        }
    }
}

/// Client handle to submit operations to a `RemoteRuntime<T>` and await replies.
pub struct RemoteTrust<T> {
    req: Arc<ArrayQueue<RemoteOp<T>>>,
    resp: Arc<ArrayQueue<RemoteResp>>,
    _phantom: PhantomData<T>,
}

impl<T: Send + 'static> RemoteTrust<T> {
    #[inline]
    /// Apply a mutation on the remote property and wait for completion.
    pub fn apply_mut(&self, f: fn(&mut T)) {
        // Push with backoff if full.
        let mut budget = WaitBudget::hot();
        loop {
            if self.req.push(RemoteOp::Apply(f)).is_ok() {
                break;
            }
            budget.step();
        }
        // Pop with backoff if empty.
        let mut budget = WaitBudget::hot();
        loop {
            if let Some(RemoteResp::None) = self.resp.pop() {
                return;
            }
            budget.step();
        }
    }

    #[inline]
    /// Apply a function on the remote property and return a `u64` result.
    pub fn apply_map_u64(&self, f: fn(&mut T) -> u64) -> u64 {
        let mut budget = WaitBudget::hot();
        loop {
            if self.req.push(RemoteOp::MapU64(f)).is_ok() {
                break;
            }
            budget.step();
        }
        let mut budget = WaitBudget::hot();
        loop {
            if let Some(RemoteResp::U64(v)) = self.resp.pop() {
                return v;
            }
            budget.step();
        }
    }

    #[inline]
    /// Apply a mutation `n` times on the remote property and wait for completion.
    pub fn apply_batch_mut(&self, f: fn(&mut T), n: u32) {
        let mut budget = WaitBudget::hot();
        loop {
            if self.req.push(RemoteOp::ApplyBatch(f, n)).is_ok() {
                break;
            }
            budget.step();
        }
        let mut budget = WaitBudget::hot();
        loop {
            if let Some(RemoteResp::NoneBatch(_n)) = self.resp.pop() {
                return;
            }
            budget.step();
        }
    }
}
