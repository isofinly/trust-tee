use core::marker::PhantomData;
use crossbeam_queue::SegQueue;
use smallvec::SmallVec;
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    static FIBER_POOL: RefCell<Vec<crate::util::fiber::Sender<Job>>> = RefCell::new(Vec::new());
}

use crate::{
    runtime::serialization::decode_and_call,
    runtime::slots::{
        ChannelPair, FatClosurePtr, HEADER_BYTES, PropertyPtr, RequestRecordHeader, header,
    },
    trust::remote::Remote,
    util::affinity::PinConfig,
};

// Concrete, zero-overhead registrar to register new ChannelPairs with the worker.
pub(crate) struct Registrar<T> {
    reg: Arc<SegQueue<ClientPair<T>>>,
    pub(crate) trustee_id: Arc<core::sync::atomic::AtomicUsize>,
    pub(crate) trustee_prop: Arc<core::sync::atomic::AtomicPtr<()>>,
}

impl<T> Registrar<T> {
    #[inline]
    fn new(
        reg: Arc<SegQueue<ClientPair<T>>>,
        trustee_id: Arc<core::sync::atomic::AtomicUsize>,
        trustee_prop: Arc<core::sync::atomic::AtomicPtr<()>>,
    ) -> Self {
        Self {
            reg,
            trustee_id,
            trustee_prop,
        }
    }
    #[inline]
    pub(crate) fn register(&self, chan: Arc<ChannelPair>) {
        self.reg.push(ClientPair {
            chan,
            pending_launch: false,
            launch_req_ready: false,
            _phantom: PhantomData,
        });
    }
}

impl<T> Clone for Registrar<T> {
    fn clone(&self) -> Self {
        Self {
            reg: self.reg.clone(),
            trustee_id: self.trustee_id.clone(),
            trustee_prop: self.trustee_prop.clone(),
        }
    }
}

struct ClientPair<T> {
    chan: Arc<ChannelPair>,
    pending_launch: bool,
    launch_req_ready: bool,
    _phantom: PhantomData<T>,
}

type Job = Box<dyn FnOnce() + Send>;

/// Runtime hosting a property `T` on a worker thread and serving clients.
pub struct Runtime<T> {
    reg: Arc<SegQueue<ClientPair<T>>>,
    trustee_id: Arc<core::sync::atomic::AtomicUsize>,
    trustee_prop: Arc<core::sync::atomic::AtomicPtr<()>>,
    _worker: Option<crate::util::fiber::JoinHandle<()>>,
}

impl<T> Clone for Runtime<T> {
    fn clone(&self) -> Self {
        Runtime {
            reg: self.reg.clone(),
            trustee_id: self.trustee_id.clone(),
            trustee_prop: self.trustee_prop.clone(),
            _worker: None,
        }
    }
}

impl<T: Send + 'static> Runtime<T> {
    /// Spawn a remote runtime worker thread with default burst and no pinning.
    pub fn spawn(value: T) -> (Self, Remote<T>) {
        Self::spawn_with_pin(value, 64, None)
    }

    /// Spawn a remote runtime worker with explicit `burst` and optional pinning.
    pub fn spawn_with_pin(value: T, _burst: usize, _pin: Option<PinConfig>) -> (Self, Remote<T>) {
        let reg: Arc<SegQueue<ClientPair<T>>> = Arc::new(SegQueue::new());
        let reg_consumer = reg.clone();

        let trustee_id = Arc::new(core::sync::atomic::AtomicUsize::new(0));
        let trustee_prop = Arc::new(core::sync::atomic::AtomicPtr::new(core::ptr::null_mut()));

        let worker_trustee_id = trustee_id.clone();
        let worker_trustee_prop = trustee_prop.clone();

        let _worker = unsafe {
            crate::util::fiber::Builder::new()
                .name("trustee".to_string())
                .spawn(move || {
                    // Note: PinConfig is ignored here as may manages threads.

                    // Wrap prop in Arc<UnsafeCell> to allow shared ownership between worker and fibers.
                    // This prevents use-after-free if the worker exits while a fiber is still running.
                    let prop = Arc::new(core::cell::UnsafeCell::new(value));

                    // Publish trustee thread ID and property pointer once.
                    worker_trustee_id.store(
                        crate::util::thread_id::get_thread_id(),
                        core::sync::atomic::Ordering::Relaxed,
                    );
                    worker_trustee_prop.store(
                        prop.as_ref().get() as *mut (),
                        core::sync::atomic::Ordering::Relaxed,
                    );

                    let mut clients: SmallVec<[ClientPair<T>; 64]> = SmallVec::new();
                    let mut start_idx: usize = 0;
                    let mut idle_rounds: u32 = 0;

                    let mut loop_counter: usize = 0;

                    loop {
                        loop_counter = loop_counter.wrapping_add(1);

                        // Drain registrations in bounded chunks.
                        // Throttled Polling: Only check registrations if we are idle OR every 16th round.
                        if clients.is_empty() || loop_counter % 16 == 0 {
                            let mut drained = 0;
                            while drained < 256 {
                                {
                                    match reg_consumer.pop() {
                                        Some(cp) => {
                                            clients.push(cp);
                                            drained += 1;
                                        }
                                        None => break,
                                    }
                                }
                            }
                        }

                        if clients.is_empty() {
                            // Light idle: short spin, then yield; never park.
                            if idle_rounds < 8 {
                                core::hint::spin_loop();
                                idle_rounds += 1;
                            } else {
                                crate::util::fiber::yield_now();
                                idle_rounds = 0;
                            }
                            continue;
                        }

                        let n = clients.len();
                        let mut progressed = false;

                        for offs in 0..n {
                            // Optimize Round-Robin Indexing: Replace modulo with conditional subtraction.
                            let mut idx = start_idx + offs;
                            if idx >= n {
                                idx -= n;
                            }
                            let cp = &mut clients[idx];

                            let req_flags =
                                cp.chan.request.get() as *const core::sync::atomic::AtomicU32;
                            let resp_flags =
                                cp.chan.response.get() as *const core::sync::atomic::AtomicU32;
                            let req_word = (*req_flags).load(core::sync::atomic::Ordering::Acquire);
                            let resp_word =
                                (*resp_flags).load(core::sync::atomic::Ordering::Acquire);
                            let req_ready = header::ready(req_word);
                            let resp_ready = header::ready(resp_word);

                            if cp.pending_launch {
                                if req_ready == resp_ready {
                                    // Launch completed (fiber flipped the bit).
                                    cp.pending_launch = false;
                                } else if req_ready != cp.launch_req_ready {
                                    // Launch completed (observed via client progress).
                                    // Client has already flipped req_ready, so previous launch is done.
                                    cp.pending_launch = false;
                                    // Fall through to process new request.
                                } else {
                                    // Launch still in progress. Skip.
                                    continue;
                                }
                            }

                            if req_ready == resp_ready {
                                continue;
                            }

                            // Process a batch of request records according to the request count.
                            let req_count = header::count(req_word) as usize;
                            let mut processed: usize = 0;
                            let mut deferred_response = false;

                            {
                                let base = cp.chan.request.get() as *mut u8;
                                let mut header_cursor = HEADER_BYTES;

                                for _ in 0..req_count {
                                    header_cursor = header::next_boundary_offset(
                                        header_cursor,
                                        core::mem::size_of::<RequestRecordHeader>(),
                                        core::mem::align_of::<RequestRecordHeader>(),
                                    );
                                    let record_ptr = base.add(header_cursor) as *const u8;

                                    let hdr: RequestRecordHeader = core::ptr::read_unaligned(
                                        record_ptr as *const RequestRecordHeader,
                                    );

                                    match hdr.property_ptr {
                                        PropertyPtr::CallMutRetUnit => {
                                            for _ in 0..hdr.repeat_count {
                                                let _: () = decode_and_call::<(&mut T,), ()>(
                                                    &hdr,
                                                    (&mut *prop.as_ref().get(),),
                                                );
                                            }
                                        }
                                        PropertyPtr::CallMutOutPtr => {
                                            let resp_base = cp.chan.response.get() as *mut u8;
                                            let resp_data = resp_base.add(HEADER_BYTES);
                                            let _: () = decode_and_call::<(*mut T, *mut u8), ()>(
                                                &hdr,
                                                (prop.as_ref().get(), resp_data),
                                            );
                                        }
                                        PropertyPtr::CallMutArgsOutPtr => {
                                            let resp_base = cp.chan.response.get() as *mut u8;
                                            let resp_data = resp_base.add(HEADER_BYTES);
                                            let base = cp.chan.request.get() as *mut u8;
                                            let args_ptr =
                                                base.add(hdr.args_offset as usize) as *const u8;
                                            let args_len = hdr.args_len;
                                            let _: () = decode_and_call::<
                                                (*mut T, *mut u8, *const u8, u32),
                                                (),
                                            >(
                                                &hdr,
                                                (
                                                    prop.as_ref().get(),
                                                    resp_data,
                                                    args_ptr,
                                                    args_len,
                                                ),
                                            );
                                        }
                                        PropertyPtr::IntoInner => {
                                            let resp_base = cp.chan.response.get() as *mut u8;
                                            let resp_data = resp_base.add(HEADER_BYTES);
                                            core::ptr::write_unaligned(
                                                resp_data.cast::<T>(),
                                                core::ptr::read(prop.as_ref().get()),
                                            );

                                            let req_flags = cp.chan.request.get()
                                                as *const core::sync::atomic::AtomicU32;
                                            let resp_flags = cp.chan.response.get()
                                                as *const core::sync::atomic::AtomicU32;
                                            let req_word_now = {
                                                (*req_flags)
                                                    .load(core::sync::atomic::Ordering::Acquire)
                                            };
                                            let req_ready_now = header::ready(req_word_now);
                                            let new_word =
                                                header::pack_ready_count(req_ready_now, 1);
                                            (*resp_flags).store(
                                                new_word,
                                                core::sync::atomic::Ordering::Release,
                                            );
                                            return;
                                        }
                                        PropertyPtr::Launch => {
                                            deferred_response = true;
                                            cp.pending_launch = true;
                                            cp.launch_req_ready = req_ready;

                                            // Capture necessary data for the fiber
                                            // We move an Arc<UnsafeCell<T>> to the fiber.
                                            // Since Arc<UnsafeCell<T>> is !Send, we convert to raw pointer.
                                            let prop_raw = Arc::into_raw(prop.clone())
                                                as *mut core::cell::UnsafeCell<T>;
                                            // Use AtomicPtr to carry the raw pointer safely.
                                            let prop_ptr_carrier =
                                                core::sync::atomic::AtomicPtr::new(prop_raw);

                                            let resp_base = cp.chan.response.get() as *mut u8;
                                            let resp_data = core::sync::atomic::AtomicPtr::new(
                                                resp_base.add(HEADER_BYTES),
                                            );
                                            let resp_flags = core::sync::atomic::AtomicPtr::new(
                                                cp.chan.response.get()
                                                    as *mut core::sync::atomic::AtomicU32,
                                            );

                                            // We need to copy `hdr` to the fiber because it's on the stack.
                                            // RequestRecordHeader is Send now.
                                            let hdr_copy = hdr;

                                            let mut job: Job = Box::new(move || {
                                                // Reconstruct Arc to ensure cleanup.
                                                let prop_raw = prop_ptr_carrier.load(
                                                    core::sync::atomic::Ordering::Relaxed,
                                                );
                                                // Safety: We created this pointer from Arc::into_raw.
                                                let prop_arc = Arc::from_raw(prop_raw);
                                                let prop_ptr = prop_arc.get();

                                                let resp_data = resp_data.load(
                                                    core::sync::atomic::Ordering::Relaxed,
                                                );
                                                let resp_flags = resp_flags.load(
                                                    core::sync::atomic::Ordering::Relaxed,
                                                );
                                                let hdr = hdr_copy;

                                                // Execute the closure.
                                                // Note: We pass `prop_ptr` (*mut T) directly.
                                                // The closure signature for Launch expects `*mut T` and casts to `&T`.
                                                // This avoids creating `&mut T` in the runtime, which would conflict with
                                                // concurrent `Apply` calls (which also create `&mut T`).
                                                let _: () =
                                                    decode_and_call::<(*mut T, *mut u8), ()>(
                                                        &hdr,
                                                        (prop_ptr, resp_data),
                                                    );

                                                // Signal completion
                                                // We assume single Launch request per batch for now.
                                                // We need to read current req_ready to flip it.
                                                // But we don't have easy access to req_flags here (it's in cp).
                                                // We can capture `resp_flags` and `req_ready`.
                                                // Wait, `req_ready` changes? No, it's fixed for this batch.
                                                let new_word =
                                                    header::pack_ready_count(req_ready, 1);
                                                (*resp_flags).store(
                                                    new_word,
                                                    core::sync::atomic::Ordering::Release,
                                                );
                                            });

                                            // Try to reuse a fiber from the pool
                                            let mut reused_tx = None;
                                            FIBER_POOL.with(|pool| {
                                                if let Some(tx) = pool.borrow_mut().pop() {
                                                    reused_tx = Some(tx);
                                                }
                                            });

                                            if let Some(tx) = reused_tx {
                                                // Send job to existing fiber
                                                match tx.send(job) {
                                                    Ok(_) => {
                                                        // Success
                                                    }
                                                    Err(e) => {
                                                        // If send failed, the fiber is dead. Spawn new.
                                                        job = e.0; // Recover the job
                                                        // Fallthrough to spawn
                                                        // println!("Spawning new fiber (reuse failed)");
                                                        // Spawn new pooled fiber
                                                        let (tx, rx) = crate::util::fiber::channel::<Job>();

                                                        // Send the first job immediately
                                                        let _ = tx.send(job);

                                                        crate::util::fiber::Builder::new()
                                                            .spawn(move || {
                                                                // Fiber Loop
                                                                loop {
                                                                    match rx.recv() {
                                                                        Ok(job) => {
                                                                            job();
                                                                            // Job done. Return self to pool.
                                                                            FIBER_POOL.with(|pool| {
                                                                                pool.borrow_mut().push(tx.clone());
                                                                            });
                                                                        }
                                                                        Err(_) => {
                                                                            // Sender dropped (runtime dropped?), exit.
                                                                            break;
                                                                        }
                                                                    }
                                                                }
                                                            })
                                                            .expect("spawn launch fiber");
                                                    }
                                                }
                                            } else {
                                                // println!("Spawning new fiber");
                                                // Spawn new pooled fiber
                                                let (tx, rx) = crate::util::fiber::channel::<Job>();

                                                // Send the first job immediately
                                                let _ = tx.send(job);

                                                crate::util::fiber::Builder::new()
                                                    .spawn(move || {
                                                        // Fiber Loop
                                                        loop {
                                                            match rx.recv() {
                                                                Ok(job) => {
                                                                    job();
                                                                    // Job done. Return self to pool.
                                                                    FIBER_POOL.with(|pool| {
                                                                        pool.borrow_mut().push(tx.clone());
                                                                    });
                                                                }
                                                                Err(_) => {
                                                                    // Sender dropped (runtime dropped?), exit.
                                                                    break;
                                                                }
                                                            }
                                                        }
                                                    })
                                                    .expect("spawn launch fiber");
                                            }
                                        }

                                        PropertyPtr::Terminate => {
                                            return;
                                        }
                                    }

                                    header_cursor += core::mem::size_of::<RequestRecordHeader>();
                                    processed += 1;
                                }
                            }

                            if deferred_response {
                            } else {
                                let new_word =
                                    header::pack_ready_count(req_ready, processed as u32);
                                (*resp_flags)
                                    .store(new_word, core::sync::atomic::Ordering::Release);
                            }

                            progressed = true;
                            start_idx = (idx + 1) % n;
                        }

                        if progressed {
                            idle_rounds = 0;
                        } else {
                            // Unified Idle Strategy: Spin briefly before yielding even when clients are present.
                            if idle_rounds < 8 {
                                core::hint::spin_loop();
                                idle_rounds += 1;
                            } else {
                                crate::util::fiber::yield_now();
                                idle_rounds = 0;
                            }
                        }
                    }
                })
        }
        .expect("failed to spawn trustee fiber");

        let rt = Runtime {
            reg: reg.clone(),
            trustee_id: trustee_id.clone(),
            trustee_prop: trustee_prop.clone(),
            _worker: Some(_worker),
        };

        // Manually construct the first remote handle to avoid circular dependency or complex signature
        let chan = Arc::new(ChannelPair::default());
        reg.push(ClientPair {
            chan: chan.clone(),
            pending_launch: false,
            launch_req_ready: false,
            _phantom: PhantomData,
        });

        let registrar = Registrar::new(reg, trustee_id, trustee_prop);
        let handle = Remote {
            chan,
            registrar,
            _phantom: PhantomData,
            _owner: Some(Arc::new(rt.clone())),
            _not_sync: core::marker::PhantomData,
        };

        (rt, handle)
    }

    /// Create a client handle by registering a channel pair with the worker.
    pub fn entrust(&self) -> Remote<T> {
        let chan = Arc::new(ChannelPair::default());
        self.reg.push(ClientPair {
            chan: chan.clone(),
            pending_launch: false,
            launch_req_ready: false,
            _phantom: PhantomData,
        });

        let registrar = Registrar::new(
            self.reg.clone(),
            self.trustee_id.clone(),
            self.trustee_prop.clone(),
        );
        Remote {
            chan,
            registrar,
            _phantom: PhantomData,
            _owner: None,
            _not_sync: core::marker::PhantomData,
        }
    }
}

impl<T> Drop for Runtime<T> {
    fn drop(&mut self) {
        if let Some(handle) = self._worker.take() {
            // Send TERM to worker via a temporary registration
            let chan = Arc::new(ChannelPair::default());
            self.reg.push(ClientPair {
                chan: chan.clone(),
                pending_launch: false,
                launch_req_ready: false,
                _phantom: PhantomData,
            });
            unsafe {
                let base = chan.request.get() as *mut u8;
                let hdr_offset = header::next_boundary_offset(
                    HEADER_BYTES,
                    core::mem::size_of::<RequestRecordHeader>(),
                    core::mem::align_of::<RequestRecordHeader>(),
                );
                let hdr_ptr = base.add(hdr_offset) as *mut RequestRecordHeader;
                let hdr = RequestRecordHeader {
                    closure: FatClosurePtr {
                        data: core::ptr::null_mut(),
                        vtable: core::ptr::null(),
                    },
                    property_ptr: PropertyPtr::Terminate,
                    captured_len: 0,
                    args_len: 0,
                    args_offset: 0,
                    repeat_count: 0,
                };
                core::ptr::write_unaligned(hdr_ptr, hdr);
                let resp_word = (&*chan.response.get())
                    .primary
                    .header
                    .flags
                    .load(core::sync::atomic::Ordering::Acquire);
                let req_ready = !header::ready(resp_word);
                let word = header::pack_ready_count(req_ready, 1);
                (&*chan.request.get())
                    .primary
                    .header
                    .flags
                    .store(word, core::sync::atomic::Ordering::Release);
            }
            let _ = handle.join();
        }
    }
}
