use core::marker::PhantomData;
use crossbeam_queue::SegQueue;
use smallvec::SmallVec;
use std::{sync::Arc, thread};

use crate::{
    runtime::serialization::decode_and_call,
    runtime::slots::{
        ChannelPair, FatClosurePtr, HEADER_BYTES, PropertyPtr, RequestRecordHeader, header,
    },
    trust::remote::Remote,
    util::WaitBudget,
    util::affinity::{PinConfig, pin_current_thread},
};

// Concrete, zero-overhead registrar to register new ChannelPairs with the worker.
pub(crate) struct Registrar<T> {
    reg: Arc<SegQueue<ClientPair<T>>>,
}

impl<T> Registrar<T> {
    #[inline]
    fn new(reg: Arc<SegQueue<ClientPair<T>>>) -> Self {
        Self { reg }
    }
    #[inline]
    pub(crate) fn register(&self, chan: Arc<ChannelPair>) {
        self.reg.push(ClientPair {
            chan,
            _phantom: PhantomData,
        });
    }
}

impl<T> Clone for Registrar<T> {
    fn clone(&self) -> Self {
        Self {
            reg: self.reg.clone(),
        }
    }
}

struct ClientPair<T> {
    chan: Arc<ChannelPair>,
    _phantom: PhantomData<T>,
}

/// Runtime hosting a property `T` on a worker thread and serving clients.
pub struct Runtime<T> {
    reg: Arc<SegQueue<ClientPair<T>>>,
    _worker: Option<thread::JoinHandle<()>>,
}

impl<T> Clone for Runtime<T> {
    fn clone(&self) -> Self {
        Runtime {
            reg: self.reg.clone(),
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
    pub fn spawn_with_pin(value: T, _burst: usize, pin: Option<PinConfig>) -> (Self, Remote<T>) {
        let reg: Arc<SegQueue<ClientPair<T>>> = Arc::new(SegQueue::new());
        let reg_consumer = reg.clone();

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

                    let req_flags = cp.chan.request.get() as *const core::sync::atomic::AtomicU32;
                    let resp_flags = cp.chan.response.get() as *const core::sync::atomic::AtomicU32;
                    let req_word =
                        unsafe { (*req_flags).load(core::sync::atomic::Ordering::Acquire) };
                    let resp_word =
                        unsafe { (*resp_flags).load(core::sync::atomic::Ordering::Acquire) };
                    let req_ready = header::ready(req_word);
                    let resp_ready = header::ready(resp_word);
                    if req_ready == resp_ready {
                        continue;
                    }

                    // Process a batch of request records according to the request count.
                    let req_count = header::count(req_word) as usize;
                    let mut processed: usize = 0;
                    unsafe {
                        let base = cp.chan.request.get() as *mut u8;
                        let mut header_cursor = HEADER_BYTES;

                        for _ in 0..req_count {
                            header_cursor = header::next_boundary_offset(
                                header_cursor,
                                core::mem::size_of::<RequestRecordHeader>(),
                                core::mem::align_of::<RequestRecordHeader>(),
                            );
                            let record_ptr = base.add(header_cursor) as *const u8;

                            let hdr: RequestRecordHeader =
                                core::ptr::read_unaligned(record_ptr as *const RequestRecordHeader);

                            match hdr.property_ptr {
                                PropertyPtr::CallMutRetUnit => {
                                    // Execute the closure repeat_count times.
                                    // Note: The closure environment must be safe to reuse (Copy or stateless).
                                    // apply_batch_mut ensures this by capturing a function pointer.
                                    for _ in 0..hdr.repeat_count {
                                        let _: () =
                                            decode_and_call::<(&mut T,), ()>(&hdr, (&mut prop,));
                                    }
                                }
                                PropertyPtr::CallMutOutPtr => {
                                    // Generic return path: pass pointer to response payload to closure.
                                    let resp_base = cp.chan.response.get() as *mut u8;
                                    let resp_data = resp_base.add(HEADER_BYTES);
                                    let _: () = decode_and_call::<(&mut T, *mut u8), ()>(
                                        &hdr,
                                        (&mut prop, resp_data),
                                    );
                                }
                                PropertyPtr::CallMutArgsOutPtr => {
                                    // Serialized-args path: use header-provided args_offset to maintain strict provenance.
                                    let resp_base = cp.chan.response.get() as *mut u8;
                                    let resp_data = resp_base.add(HEADER_BYTES);
                                    let base = cp.chan.request.get() as *mut u8;
                                    let args_ptr = base.add(hdr.args_offset as usize) as *const u8;
                                    let args_len = hdr.args_len;
                                    let _: () =
                                        decode_and_call::<(&mut T, *mut u8, *const u8, u32), ()>(
                                            &hdr,
                                            (&mut prop, resp_data, args_ptr, args_len),
                                        );
                                }
                                PropertyPtr::IntoInner => {
                                    // Move the inner T into the response buffer, signal, then terminate.
                                    let resp_base = cp.chan.response.get() as *mut u8;
                                    let resp_data = resp_base.add(HEADER_BYTES);
                                    // Move out of prop and write into response payload
                                    core::ptr::write_unaligned(
                                        resp_data.cast::<T>(),
                                        core::ptr::read(&mut prop as *mut T),
                                    );

                                    // Toggle response ready bit to match current request and set count=1
                                    let req_flags = cp.chan.request.get()
                                        as *const core::sync::atomic::AtomicU32;
                                    let resp_flags = cp.chan.response.get()
                                        as *const core::sync::atomic::AtomicU32;
                                    let req_word_now = {
                                        (*req_flags).load(core::sync::atomic::Ordering::Acquire)
                                    };
                                    let req_ready_now = header::ready(req_word_now);
                                    let new_word = header::pack_ready_count(req_ready_now, 1);
                                    (*resp_flags)
                                        .store(new_word, core::sync::atomic::Ordering::Release);
                                    return;
                                }
                                PropertyPtr::Terminate => {
                                    return;
                                }
                            }

                            // Next record header is laid out immediately after previous header table entry.
                            header_cursor += core::mem::size_of::<RequestRecordHeader>();
                            processed += 1;
                        }
                    }

                    let new_word = header::pack_ready_count(req_ready, processed as u32);
                    unsafe { (*resp_flags).store(new_word, core::sync::atomic::Ordering::Release) };
                    progressed = true;
                    start_idx = (idx + 1) % n;
                }

                if progressed {
                    idle_rounds = 0;
                } else {
                    WaitBudget::default().step();
                }
            }
        });

        let rt = Runtime {
            reg,
            _worker: Some(_worker),
        };
        let handle = rt.entrust();
        (rt, handle)
    }

    #[inline]
    /// Create a client handle by registering a channel pair with the worker.
    pub fn entrust(&self) -> Remote<T> {
        let chan = Arc::new(ChannelPair::default());
        self.reg.push(ClientPair {
            chan: chan.clone(),
            _phantom: PhantomData,
        });

        let registrar = Registrar::new(self.reg.clone());
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
