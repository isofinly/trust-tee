use core::marker::PhantomData;
use smallvec::SmallVec;
use std::{
    sync::{
        Arc,
        mpsc::{Receiver, Sender, TryRecvError, channel},
    },
    thread,
};

use crate::{
    runtime::serialization::decode_and_call,
    runtime::slots::{ChannelPair, FatClosurePtr, HEADER_BYTES, RequestRecordHeader, header},
    trust::remote::Remote,
    util::WaitBudget,
    util::affinity::{PinConfig, pin_current_thread},
};
const TERM_PROP_PTR: u64 = u64::MAX;

struct ClientPair<T> {
    chan: Arc<ChannelPair>,
    _phantom: PhantomData<T>,
}

/// Runtime hosting a property `T` on a worker thread and serving clients.
pub struct Runtime<T> {
    reg_tx: Sender<ClientPair<T>>,
    _worker: Option<thread::JoinHandle<()>>,
}

impl<T: Send + 'static> Runtime<T> {
    /// Spawn a remote runtime worker thread with default burst and no pinning.
    pub fn spawn(value: T) -> (Self, Remote<T>) {
        Self::spawn_with_pin(value, 64, None)
    }

    /// Spawn a remote runtime worker with explicit `burst` and optional pinning.
    pub fn spawn_with_pin(value: T, _burst: usize, pin: Option<PinConfig>) -> (Self, Remote<T>) {
        let (reg_tx, reg_rx): (Sender<ClientPair<T>>, Receiver<ClientPair<T>>) = channel();

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
                    match reg_rx.try_recv() {
                        Ok(cp) => {
                            clients.push(cp);
                            drained += 1;
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => break,
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
                        let mut record_ptr = base.add(HEADER_BYTES) as *const u8;
                        for _ in 0..req_count {
                            let hdr: RequestRecordHeader =
                                core::ptr::read_unaligned(record_ptr as *const RequestRecordHeader);

                            match hdr.property_ptr {
                                0 => {
                                    let _: () =
                                        decode_and_call::<(&mut T,), ()>(&hdr, (&mut prop,));
                                }
                                1 => {
                                    let val: u64 =
                                        decode_and_call::<(&mut T,), u64>(&hdr, (&mut prop,));
                                    let resp_base = cp.chan.response.get() as *mut u8;
                                    let resp_data = resp_base.add(HEADER_BYTES);
                                    core::ptr::write_unaligned(
                                        resp_data.cast::<u64>(),
                                        val.to_le(),
                                    );
                                }
                                TERM_PROP_PTR => {
                                    return;
                                }
                                _ => {
                                    let vt = &*(hdr.closure.vtable
                                        as *const crate::runtime::slots::ErasedVTable<(), ()>);
                                    (vt.drop_in_place)(hdr.closure.data);
                                }
                            }

                            // Next record header is laid out immediately after previous header table entry.
                            record_ptr =
                                record_ptr.add(core::mem::size_of::<RequestRecordHeader>());
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
            reg_tx,
            _worker: Some(_worker),
        };
        let handle = rt.entrust();
        (rt, handle)
    }

    #[inline]
    /// Create a client handle by registering a channel pair with the worker.
    pub fn entrust(&self) -> Remote<T> {
        let chan = Arc::new(ChannelPair::default());
        self.reg_tx
            .send(ClientPair {
                chan: chan.clone(),
                _phantom: PhantomData,
            })
            .ok();
        Remote {
            chan,
            _phantom: PhantomData,
        }
    }
}

impl<T> Drop for Runtime<T> {
    fn drop(&mut self) {
        if let Some(handle) = self._worker.take() {
            // Send TERM to worker via a temporary registration
            let chan = Arc::new(ChannelPair::default());
            let _ = self.reg_tx.send(ClientPair {
                chan: chan.clone(),
                _phantom: PhantomData,
            });
            unsafe {
                let base = chan.request.get() as *mut u8;
                let hdr_ptr = base.add(HEADER_BYTES) as *mut RequestRecordHeader;
                let hdr = RequestRecordHeader {
                    closure: FatClosurePtr {
                        data: core::ptr::null_mut(),
                        vtable: core::ptr::null(),
                    },
                    property_ptr: TERM_PROP_PTR,
                    captured_len: 0,
                    args_len: 0,
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
