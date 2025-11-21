use crate::runtime::Runtime;
use crate::runtime::runtime::Registrar;
use crate::runtime::serialization::{SlotWriter, encode_closure};
use crate::runtime::slots::{
    ChannelPair, HEADER_BYTES, PropertyPtr, RequestRecordHeader, SLOT_BYTES, header,
};
use crate::trust::common::TrustLike;
use crate::util::WaitBudget;
use bincode;
use core::cell::Cell;
use core::marker::PhantomData;
use std::sync::Arc;

/// Client Remote Trust to submit operations to a `Runtime<T>` and await replies.
pub struct Remote<T> {
    pub(crate) chan: Arc<ChannelPair>,
    // Registrar registers a new ChannelPair with the worker thread
    pub(crate) registrar: Registrar<T>,
    pub(crate) _phantom: PhantomData<T>,
    // When created via Remote::entrust(), keep the runtime alive until this handle drops.
    pub(crate) _owner: Option<Arc<Runtime<T>>>,
    // Not Sync marker to enforce SPSC semantics: &Remote<T> is !Sync
    pub(crate) _not_sync: PhantomData<Cell<()>>,
}

// Enforce SPSC: Remote handles are sendable.
unsafe impl<T: Send + 'static> Send for Remote<T> {}

impl<T> Clone for Remote<T> {
    fn clone(&self) -> Self {
        let chan = Arc::new(ChannelPair::default());
        self.registrar.register(chan.clone());

        Self {
            chan,
            registrar: self.registrar.clone(),
            _phantom: PhantomData,
            _owner: self._owner.clone(), // Share ownership of runtime
            _not_sync: PhantomData,
        }
    }
}

impl<T: Send + 'static> super::common::TrustLike for Remote<T> {
    type Value = T;
    #[inline]
    /// Create a `Trust<T>` (remote) holding `value` managed by a remote trustee.
    /// Note: This requires a runtime to be set up separately.
    fn entrust(value: T) -> Self {
        let (rt, handle) = Runtime::spawn(value);
        Self {
            chan: handle.chan.clone(),
            registrar: handle.registrar.clone(),
            _phantom: PhantomData,
            _owner: Some(Arc::new(rt)),
            _not_sync: PhantomData,
        }
    }

    #[inline]
    /// Mutably apply `f` to the inner `T` and return its result.
    fn apply<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R + Send + 'static,
        R: Send + 'static,
    {
        // Optimization: Check if we are running on the trustee thread.
        // If so, we can bypass the channel and execute directly.
        let trustee_id = self
            .registrar
            .trustee_id
            .load(core::sync::atomic::Ordering::Relaxed);
        if trustee_id != 0 && trustee_id == crate::util::thread_id::get_thread_id() {
            // We are on the trustee thread!
            // Access the property directly via the raw pointer.
            let prop_ptr = self
                .registrar
                .trustee_prop
                .load(core::sync::atomic::Ordering::Relaxed) as *mut T;
            if !prop_ptr.is_null() {
                unsafe {
                    let p = &mut *(prop_ptr as *mut T);
                    return f(p);
                }
            }
        }

        unsafe {
            let base = self.chan.request.get() as *mut u8;
            let hdr_offset = header::next_boundary_offset(
                HEADER_BYTES,
                core::mem::size_of::<RequestRecordHeader>(),
                core::mem::align_of::<RequestRecordHeader>(),
            );
            let payload_offset = hdr_offset + core::mem::size_of::<RequestRecordHeader>();

            let mut w = SlotWriter {
                base,
                len: SLOT_BYTES,
                cursor: payload_offset,
            };
            // Encode closure that writes its return value into response slot.
            let hdr = encode_closure::<_, (&mut T, *mut u8), ()>(
                &mut w,
                PropertyPtr::CallMutOutPtr,
                move |(p, out)| {
                    let r = f(p);
                    core::ptr::write_unaligned(out as *mut R, r);
                },
                &[],
            );
            let hdr_ptr = base.add(hdr_offset) as *mut RequestRecordHeader;
            core::ptr::write_unaligned(hdr_ptr, hdr);

            // Signal request ready
            let resp_flags = self.chan.response.get() as *const core::sync::atomic::AtomicU32;
            let req_flags = self.chan.request.get() as *const core::sync::atomic::AtomicU32;
            let resp_word = (*resp_flags).load(core::sync::atomic::Ordering::Acquire);
            let req_ready = !header::ready(resp_word);
            let word = header::pack_ready_count(req_ready, 1);
            (*req_flags).store(word, core::sync::atomic::Ordering::Release);

            // Wait for response
            let mut budget = WaitBudget::hot();
            loop {
                let resp_word_now = (*resp_flags).load(core::sync::atomic::Ordering::Acquire);
                if header::ready(resp_word_now) == req_ready {
                    break;
                }
                budget.step();
            }

            // Read back return value written by trustee closure
            let resp_base = self.chan.response.get() as *const u8;
            let resp_data = resp_base.add(HEADER_BYTES);
            core::ptr::read_unaligned(resp_data.cast::<R>())
        }
    }

    #[inline]
    /// Apply `f`, then enqueue `then` to run with the result.
    fn apply_then<F, R>(&self, f: F, then: impl FnOnce(R))
    where
        F: FnOnce(&mut T) -> R + Send + 'static,
        R: Send + 'static,
    {
        let r = self.apply(f);
        then(r)
    }

    #[inline]
    /// Mutably apply `f` with an extra argument `w`, returning its result.
    fn apply_with<F, V, R>(&self, f: F, w: V) -> R
    where
        F: FnOnce(&mut T, V) -> R + Send + 'static,
        V: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
        R: Send + 'static,
    {
        // Optimization: Check if we are running on the trustee thread.
        let trustee_id = self
            .registrar
            .trustee_id
            .load(core::sync::atomic::Ordering::Relaxed);
        if trustee_id != 0 && trustee_id == crate::util::thread_id::get_thread_id() {
            let prop_ptr = self
                .registrar
                .trustee_prop
                .load(core::sync::atomic::Ordering::Relaxed) as *mut T;
            if !prop_ptr.is_null() {
                unsafe {
                    let p = &mut *(prop_ptr as *mut T);
                    return f(p, w);
                }
            }
        }

        unsafe {
            let base = self.chan.request.get() as *mut u8;
            let hdr_offset = header::next_boundary_offset(
                HEADER_BYTES,
                core::mem::size_of::<RequestRecordHeader>(),
                core::mem::align_of::<RequestRecordHeader>(),
            );
            let payload_offset = hdr_offset + core::mem::size_of::<RequestRecordHeader>();

            let mut wtr = SlotWriter {
                base,
                len: SLOT_BYTES,
                cursor: payload_offset,
            };
            // Serialize argument `w` into args buffer; trustee will deserialize.
            let args = bincode::serialize(&w).expect("bincode serialize");
            let hdr = encode_closure::<_, (&mut T, *mut u8, *const u8, u32), ()>(
                &mut wtr,
                PropertyPtr::CallMutArgsOutPtr,
                move |(p, out, bytes, len)| {
                    let slice = { core::slice::from_raw_parts(bytes, len as usize) };
                    let v: V = bincode::deserialize(slice).expect("bincode deserialize");
                    let r = f(p, v);
                    core::ptr::write_unaligned(out as *mut R, r);
                },
                &args,
            );
            let hdr_ptr = base.add(hdr_offset) as *mut RequestRecordHeader;
            core::ptr::write_unaligned(hdr_ptr, hdr);

            let resp_flags = self.chan.response.get() as *const core::sync::atomic::AtomicU32;
            let req_flags = self.chan.request.get() as *const core::sync::atomic::AtomicU32;
            let resp_word = (*resp_flags).load(core::sync::atomic::Ordering::Acquire);
            let req_ready = !header::ready(resp_word);
            let word = header::pack_ready_count(req_ready, 1);
            (*req_flags).store(word, core::sync::atomic::Ordering::Release);

            let mut budget = WaitBudget::hot();
            loop {
                let resp_word_now = (*resp_flags).load(core::sync::atomic::Ordering::Acquire);
                if header::ready(resp_word_now) == req_ready {
                    break;
                }
                budget.step();
            }

            let resp_base = self.chan.response.get() as *const u8;
            let resp_data = resp_base.add(HEADER_BYTES);
            core::ptr::read_unaligned(resp_data.cast::<R>())
        }
    }

    #[inline]
    /// Borrow the inner `T` immutably to compute `R`.
    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R + Send + 'static,
        R: Send + 'static,
    {
        // Optimization: Check if we are running on the trustee thread.
        let trustee_id = self
            .registrar
            .trustee_id
            .load(core::sync::atomic::Ordering::Relaxed);
        if trustee_id != 0 && trustee_id == crate::util::thread_id::get_thread_id() {
            let prop_ptr = self
                .registrar
                .trustee_prop
                .load(core::sync::atomic::Ordering::Relaxed) as *mut T;
            if !prop_ptr.is_null() {
                unsafe {
                    let p = &*(prop_ptr as *const T);
                    return f(p);
                }
            }
        }

        unsafe {
            let base = self.chan.request.get() as *mut u8;
            let hdr_offset = header::next_boundary_offset(
                HEADER_BYTES,
                core::mem::size_of::<RequestRecordHeader>(),
                core::mem::align_of::<RequestRecordHeader>(),
            );
            let payload_offset = hdr_offset + core::mem::size_of::<RequestRecordHeader>();

            let mut w = SlotWriter {
                base,
                len: SLOT_BYTES,
                cursor: payload_offset,
            };
            // Invoke f with &T view and write result into response
            let hdr = encode_closure::<_, (&mut T, *mut u8), ()>(
                &mut w,
                PropertyPtr::CallMutOutPtr,
                move |(p, out)| {
                    let r = f(&*p as &T);
                    core::ptr::write_unaligned(out as *mut R, r);
                },
                &[],
            );
            let hdr_ptr = base.add(hdr_offset) as *mut RequestRecordHeader;
            core::ptr::write_unaligned(hdr_ptr, hdr);

            let resp_flags = self.chan.response.get() as *const core::sync::atomic::AtomicU32;
            let req_flags = self.chan.request.get() as *const core::sync::atomic::AtomicU32;
            let resp_word = (*resp_flags).load(core::sync::atomic::Ordering::Acquire);
            let req_ready = !header::ready(resp_word);
            let word = header::pack_ready_count(req_ready, 1);
            (*req_flags).store(word, core::sync::atomic::Ordering::Release);

            let mut budget = WaitBudget::hot();
            loop {
                let resp_word_now = (*resp_flags).load(core::sync::atomic::Ordering::Acquire);
                if header::ready(resp_word_now) == req_ready {
                    break;
                }
                budget.step();
            }

            let resp_base = self.chan.response.get() as *const u8;
            let resp_data = resp_base.add(HEADER_BYTES);
            core::ptr::read_unaligned(resp_data.cast::<R>())
        }
    }

    #[inline]
    /// Apply a mutation `n` times on the inner value and wait for completion.
    fn apply_batch_mut(&self, f: fn(&mut T), n: u8) {
        if n == 0 {
            return;
        }
        // Optimization: Check if we are running on the trustee thread.
        let trustee_id = self
            .registrar
            .trustee_id
            .load(core::sync::atomic::Ordering::Relaxed);
        if trustee_id != 0 && trustee_id == crate::util::thread_id::get_thread_id() {
            let prop_ptr = self
                .registrar
                .trustee_prop
                .load(core::sync::atomic::Ordering::Relaxed) as *mut T;
            if !prop_ptr.is_null() {
                unsafe {
                    let p = &mut *(prop_ptr as *mut T);
                    for _ in 0..n {
                        f(p);
                    }
                    return;
                }
            }
        }

        unsafe {
            let base = self.chan.request.get() as *mut u8;
            let hdr_offset = header::next_boundary_offset(
                HEADER_BYTES,
                core::mem::size_of::<RequestRecordHeader>(),
                core::mem::align_of::<RequestRecordHeader>(),
            );
            let payload_offset = hdr_offset + core::mem::size_of::<RequestRecordHeader>();

            let mut w = SlotWriter {
                base,
                len: SLOT_BYTES,
                cursor: payload_offset,
            };

            // Encode a single record, but set repeat_count = n.
            // The closure captures `f` (function pointer), which is Copy.
            // The trustee will execute this closure `n` times.
            let mut hdr = encode_closure::<_, (&mut T,), ()>(
                &mut w,
                PropertyPtr::CallMutRetUnit,
                move |(p,)| (f)(p),
                &[],
            );
            hdr.repeat_count = n as u32;

            let hdr_ptr = base.add(hdr_offset) as *mut RequestRecordHeader;
            core::ptr::write_unaligned(hdr_ptr, hdr);

            // Toggle request ready bit relative to response and set request count to 1.
            // The trustee sees 1 record, but that record instructs it to repeat `n` times.
            let resp_word = (&*self.chan.response.get())
                .primary
                .header
                .flags
                .load(core::sync::atomic::Ordering::Acquire);
            let req_ready = !header::ready(resp_word);
            let word = header::pack_ready_count(req_ready, 1);
            (&*self.chan.request.get())
                .primary
                .header
                .flags
                .store(word, core::sync::atomic::Ordering::Release);

            // Wait for response toggle to match.
            let mut budget = WaitBudget::hot();
            loop {
                let resp_word_now = (&*self.chan.response.get())
                    .primary
                    .header
                    .flags
                    .load(core::sync::atomic::Ordering::Acquire);
                if header::ready(resp_word_now) == req_ready {
                    break;
                }
                budget.step();
            }
        }
    }

    #[inline]
    /// Get underlying value and drop the trust instance.
    fn into_inner(self) -> T {
        // Only valid when this remote owns the runtime worker
        assert!(
            self._owner.is_some(),
            "Remote::into_inner requires owned runtime"
        );
        unsafe {
            let base = self.chan.request.get() as *mut u8;
            let hdr_offset = header::next_boundary_offset(
                HEADER_BYTES,
                core::mem::size_of::<RequestRecordHeader>(),
                core::mem::align_of::<RequestRecordHeader>(),
            );
            let payload_offset = hdr_offset + core::mem::size_of::<RequestRecordHeader>();

            let mut w = SlotWriter {
                base,
                len: SLOT_BYTES,
                cursor: payload_offset,
            };
            // property_ptr=4 instructs the worker to move out T into the response and terminate.
            let hdr =
                encode_closure::<_, (&mut T,), ()>(&mut w, PropertyPtr::IntoInner, |_p| (), &[]);
            let hdr_ptr = base.add(hdr_offset) as *mut RequestRecordHeader;
            core::ptr::write_unaligned(hdr_ptr, hdr);

            // Signal request ready with count=1
            let resp_flags = self.chan.response.get() as *const core::sync::atomic::AtomicU32;
            let req_flags = self.chan.request.get() as *const core::sync::atomic::AtomicU32;
            let resp_word = (*resp_flags).load(core::sync::atomic::Ordering::Acquire);
            let req_ready = !header::ready(resp_word);
            let word = header::pack_ready_count(req_ready, 1);
            (*req_flags).store(word, core::sync::atomic::Ordering::Release);

            // Wait for response
            let mut budget = WaitBudget::hot();
            loop {
                let resp_word_now = (*resp_flags).load(core::sync::atomic::Ordering::Acquire);
                if header::ready(resp_word_now) == req_ready {
                    break;
                }
                budget.step();
            }

            // Read moved-out T
            let resp_base = self.chan.response.get() as *const u8;
            let resp_data = resp_base.add(HEADER_BYTES);
            core::ptr::read_unaligned(resp_data.cast::<T>())
        }
    }

    #[inline]
    /// Launch a fiber to execute `f` on the inner value.
    fn launch<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R + Send + 'static,
        R: Send + 'static,
    {
        unsafe {
            let base = self.chan.request.get() as *mut u8;
            let hdr_offset = header::next_boundary_offset(
                HEADER_BYTES,
                core::mem::size_of::<RequestRecordHeader>(),
                core::mem::align_of::<RequestRecordHeader>(),
            );
            let payload_offset = hdr_offset + core::mem::size_of::<RequestRecordHeader>();

            let mut w = SlotWriter {
                base,
                len: SLOT_BYTES,
                cursor: payload_offset,
            };
            // Encode closure. Note: we use PropertyPtr::Launch.
            // The runtime expects FnOnce(&mut T, *mut u8).
            // We adapt f: FnOnce(&T) -> R.
            let hdr = encode_closure::<_, (&mut T, *mut u8), ()>(
                &mut w,
                PropertyPtr::Launch,
                move |(p, out)| {
                    // Safety: Launch implies concurrent shared access.
                    // We cast &mut T to &T.
                    let shared_p = &*(p as *const T);
                    let r = f(shared_p);
                    core::ptr::write_unaligned(out as *mut R, r);
                },
                &[],
            );
            let hdr_ptr = base.add(hdr_offset) as *mut RequestRecordHeader;
            core::ptr::write_unaligned(hdr_ptr, hdr);

            // Signal request ready
            let resp_flags = self.chan.response.get() as *const core::sync::atomic::AtomicU32;
            let req_flags = self.chan.request.get() as *const core::sync::atomic::AtomicU32;
            let resp_word = (*resp_flags).load(core::sync::atomic::Ordering::Acquire);
            let req_ready = !header::ready(resp_word);
            let word = header::pack_ready_count(req_ready, 1);
            (*req_flags).store(word, core::sync::atomic::Ordering::Release);

            // Wait for response (signaled by fiber)
            let mut budget = WaitBudget::hot();
            loop {
                let resp_word_now = (*resp_flags).load(core::sync::atomic::Ordering::Acquire);
                if header::ready(resp_word_now) == req_ready {
                    break;
                }
                budget.step();
            }

            // Read result
            let resp_base = self.chan.response.get() as *const u8;
            let resp_data = resp_base.add(HEADER_BYTES);
            core::ptr::read_unaligned(resp_data.cast::<R>())
        }
    }

    #[inline]
    /// Launch `f`, then enqueue `then` to run with the result.
    fn launch_then<F, R>(&self, f: F, then: impl FnOnce(R))
    where
        F: FnOnce(&T) -> R + Send + 'static,
        R: Send + 'static,
    {
        let r = self.launch(f);
        then(r)
    }
}

impl<T: Send + 'static + core::fmt::Display> core::fmt::Display for Remote<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = self.with(|v| format!("{}", v));
        f.write_str(&s)
    }
}

impl<T: Send + 'static + core::fmt::Debug> core::fmt::Debug for Remote<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = self.with(|v| format!("{:?}", v));
        f.debug_tuple("Remote").field(&s).finish()
    }
}
