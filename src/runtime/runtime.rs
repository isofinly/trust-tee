use core::marker::PhantomData;
use crossbeam_queue::{ArrayQueue, SegQueue};
use smallvec::SmallVec;
use std::{sync::Arc, thread};

use crate::{
    runtime::common::{Op, Resp},
    trust::remote::Trust,
    util::WaitBudget,
    util::affinity::{PinConfig, pin_current_thread},
};

struct ClientPair<T> {
    req: Arc<ArrayQueue<Op<T>>>,
    resp: Arc<ArrayQueue<Resp>>,
}

/// Runtime hosting a property `T` on a worker thread and serving clients.
pub struct Runtime<T> {
    reg_q: Arc<SegQueue<ClientPair<T>>>,
    capacity: usize,
    _worker: Option<thread::JoinHandle<()>>,
}

impl<T: Send + 'static> Runtime<T> {
    /// Spawn a remote runtime worker thread with default burst and no pinning.
    pub fn spawn(value: T, capacity: usize) -> (Self, Trust<T>) {
        Self::spawn_with_pin(value, capacity, 64, None)
    }

    /// Spawn a remote runtime worker with explicit `burst` and optional pinning.
    pub fn spawn_with_pin(
        value: T,
        capacity: usize,
        burst: usize,
        pin: Option<PinConfig>,
    ) -> (Self, Trust<T>) {
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
                            Some(Op::Apply(f)) => {
                                f(&mut prop);
                                let _ = cp.resp.push(Resp::None);
                                processed += 1;
                                progressed = true;
                            }
                            Some(Op::MapU64(f)) => {
                                let v = f(&mut prop);
                                let _ = cp.resp.push(Resp::U64(v));
                                processed += 1;
                                progressed = true;
                            }
                            Some(Op::ApplyBatch(f, nrep)) => {
                                for _ in 0..nrep {
                                    f(&mut prop);
                                }
                                let _ = cp.resp.push(Resp::NoneBatch(nrep));
                                processed += 1;
                                progressed = true;
                            }
                            Some(Op::Terminate) => {
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
                    WaitBudget::default().step();
                }
            }
        });

        let rt = Runtime {
            reg_q,
            capacity,
            _worker: Some(_worker),
        };
        let handle = rt.entrust();
        (rt, handle)
    }

    #[inline]
    /// Create a client handle by registering bounded MPMC queues with the worker.
    pub fn entrust(&self) -> Trust<T> {
        let req = Arc::new(ArrayQueue::new(self.capacity));
        let resp = Arc::new(ArrayQueue::new(self.capacity));
        self.reg_q.push(ClientPair {
            req: req.clone(),
            resp: resp.clone(),
        });
        Trust {
            req,
            resp,
            _phantom: PhantomData,
        }
    }
}

impl<T> Drop for Runtime<T> {
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
                if req.push(Op::Terminate).is_ok() {
                    break;
                }
                budget.step();
            }
        }
        self.reg_q.push(ClientPair { req, resp });

        if let Some(handle) = self._worker.take() {
            let _ = handle.join();
        }
    }
}
