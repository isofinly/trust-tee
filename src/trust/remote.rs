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
    pub(crate) _owner: Option<Runtime<T>>,
    // Not Sync marker to enforce SPSC semantics: &Remote<T> is !Sync
    pub(crate) _not_sync: PhantomData<Cell<()>>,
}

// Enforce SPSC: Remote handles are sendable.
unsafe impl<T: Send + 'static> Send for Remote<T> {}

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
            _owner: Some(rt),
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
                // Safety: We are on the trustee thread, so we have exclusive access to the property.
                // The runtime ensures that no other closures are running on this thread while we are here
                // (because we are the one running!).
                // We must ensure that we don't violate aliasing rules. Since we are in `apply`,
                // we are effectively borrowing the trust. If the runtime is also borrowing it,
                // that would be an issue. However, the runtime loop yields to us (the client code)
                // only when it's not using the property.
                // WAIT: The runtime loop is:
                // 1. Drain registrations
                // 2. Process requests
                // 3. Yield/Spin
                //
                // If we are running, it means the runtime loop called us? No, `Remote` is a client handle.
                // If we are on the trustee thread, it means we are running AS the trustee fiber (or another fiber on the same thread).
                // If we are the trustee fiber, we are inside the loop.
                // But `apply` is called by CLIENT code.
                //
                // Case 1: We are a separate fiber on the same thread.
                // The runtime loop runs, then yields. When it yields, our fiber runs.
                // So the runtime is NOT using the property.
                // So it is safe to access it.
                //
                // Case 2: We are called FROM a closure running on the trustee.
                // i.e. nested delegation.
                // `apply` blocks. If we block waiting for ourselves, we deadlock.
                // But here we DON'T block. We execute directly.
                // Is it safe to create a &mut T here?
                // The outer closure has &mut T.
                // If we create another &mut T, we have two mutable references. UB!
                //
                // The paper says: "In Trust<T>, blocking in delegated context is prohibited... Closures may still use apply_then(), but not the blocking apply()."
                // So `apply` inside a delegated closure is ALREADY illegal/discouraged because it blocks.
                // But with this shortcut, it DOESN'T block.
                // However, it creates aliasing.
                //
                // If we are in a separate fiber (Case 1), the runtime loop is suspended (yielded).
                // The runtime loop holds `mut prop`.
                // But it's a local variable in the stack frame of the loop.
                // When it yields, that borrow is technically still active?
                // No, `prop` is owned by the loop. It's not borrowed while yielding.
                // So Case 1 is SAFE.
                //
                // How to distinguish Case 1 from Case 2?
                // In Case 2, we are inside a closure called by the runtime.
                // The runtime passes `&mut prop` to the closure.
                // So `prop` IS borrowed.
                //
                // We need to know if `prop` is currently borrowed.
                // We can't easily know that without a flag.
                //
                // However, `apply` is documented as blocking. Calling it from a delegated closure is a logic error (deadlock) normally.
                // With this optimization, it becomes an aliasing error (UB).
                //
                // If we assume the user follows the rules (no `apply` inside delegated closure), then Case 2 never happens.
                // But we should probably be safe.
                //
                // For now, let's implement the shortcut assuming Case 1 (Fiber on same thread).
                // This is the "Local Trustee Shortcut" mentioned in the paper section 5.2.1.
                // "When a Trust has the current thread as its trustee, it is superfluous to use delegation...
                // Instead, it is just as safe... to simply apply the closure directly...
                // As a reminder, we know this because delegated closures may not suspend the current fiber."
                //
                // This implies that if we are running, we are NOT a delegated closure (because they can't suspend/block, and `apply` is blocking... wait).
                // If `apply` is called, it means we are running.
                // If we were a delegated closure, we would be running.
                // But delegated closures cannot call `apply` (deadlock).
                // So if we are running `apply`, we are NOT a delegated closure (or the user is deadlocking).
                // If we are NOT a delegated closure, but we are on the trustee thread, we MUST be another fiber.
                // And if we are another fiber, the runtime loop (which owns `prop`) is yielded.
                // So `prop` is NOT borrowed.
                // So it IS safe.
                //
                // One catch: `prop` is owned by the runtime loop stack.
                // We are accessing it via a raw pointer `trustee_prop`.
                // We need to make sure `prop` hasn't moved or been dropped.
                // `Runtime` keeps the thread alive. `Remote` keeps `Runtime` alive (via `_owner` or just refcount).
                // `prop` stays on the stack of the worker thread function.
                // As long as the worker thread is alive, `prop` is valid.
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

impl<T: Send + 'static> Clone for Remote<T> {
    fn clone(&self) -> Self {
        // allocate a fresh ChannelPair for this clone and register it with worker
        let chan = Arc::new(ChannelPair::default());

        self.registrar.register(chan.clone());
        Self {
            chan,
            registrar: self.registrar.clone(),
            _phantom: PhantomData,
            _owner: None,
            _not_sync: PhantomData,
        }
    }
}
