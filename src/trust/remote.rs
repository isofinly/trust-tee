use crate::runtime::Runtime;
use crate::runtime::serialization::{SlotWriter, encode_closure};
use crate::runtime::slots::{ChannelPair, HEADER_BYTES, RequestRecordHeader, SLOT_BYTES, header};
use crate::util::WaitBudget;
use bincode;
use core::cell::Cell;
use core::marker::PhantomData;
use std::sync::Arc;

/// Client Remote Trust to submit operations to a `Runtime<T>` and await replies.
pub struct Remote<T> {
    pub(crate) chan: Arc<ChannelPair>,
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
        unsafe {
            let base = self.chan.request.get() as *mut u8;
            let mut w = SlotWriter {
                base,
                len: SLOT_BYTES,
                cursor: HEADER_BYTES + core::mem::size_of::<RequestRecordHeader>(),
            };
            // Encode closure that writes its return value into response slot.
            let hdr = encode_closure::<_, (&mut T, *mut u8), ()>(
                &mut w,
                2,
                move |(p, out)| {
                    let r = f(p);
                    core::ptr::write_unaligned(out as *mut R, r);
                },
                &[],
            );
            let hdr_ptr = base.add(HEADER_BYTES) as *mut RequestRecordHeader;
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
        unsafe {
            let base = self.chan.request.get() as *mut u8;
            let mut wtr = SlotWriter {
                base,
                len: SLOT_BYTES,
                cursor: HEADER_BYTES + core::mem::size_of::<RequestRecordHeader>(),
            };
            // Serialize argument `w` into args buffer; trustee will deserialize.
            let args = bincode::serialize(&w).expect("bincode serialize");
            let hdr = encode_closure::<_, (&mut T, *mut u8, *const u8, u32), ()>(
                &mut wtr,
                3,
                move |(p, out, bytes, len)| {
                    let slice = { core::slice::from_raw_parts(bytes, len as usize) };
                    let v: V = bincode::deserialize(slice).expect("bincode deserialize");
                    let r = f(p, v);
                    core::ptr::write_unaligned(out as *mut R, r);
                },
                &args,
            );
            let hdr_ptr = base.add(HEADER_BYTES) as *mut RequestRecordHeader;
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
        unsafe {
            let base = self.chan.request.get() as *mut u8;
            let mut w = SlotWriter {
                base,
                len: SLOT_BYTES,
                cursor: HEADER_BYTES + core::mem::size_of::<RequestRecordHeader>(),
            };
            // Invoke f with &T view and write result into response
            let hdr = encode_closure::<_, (&mut T, *mut u8), ()>(
                &mut w,
                2,
                move |(p, out)| {
                    let r = f(&*p as &T);
                    core::ptr::write_unaligned(out as *mut R, r);
                },
                &[],
            );
            let hdr_ptr = base.add(HEADER_BYTES) as *mut RequestRecordHeader;
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
        // Delegate to the existing apply_batch_mut implementation
        unsafe {
            let base = self.chan.request.get() as *mut u8;
            let hdr_size = core::mem::size_of::<RequestRecordHeader>();
            let mut remaining = n;

            while remaining > 0 {
                // Compute closure env size/alignment: closure captures only `f: fn(&mut T)`
                let env_size = core::mem::size_of::<fn(&mut T)>();
                let env_align = core::mem::align_of::<fn(&mut T)>();

                let available = SLOT_BYTES - HEADER_BYTES;
                let cap_by_headers = available / hdr_size;
                let mut best = 0usize;
                let try_up_to = core::cmp::min(remaining as usize, cap_by_headers);
                for k in 1..=try_up_to {
                    // Payload starts after k headers
                    let mut cursor = HEADER_BYTES + k * hdr_size;
                    // Pack k envs back-to-back with alignment
                    for _ in 0..k {
                        let align_mask = env_align.saturating_sub(1);
                        if align_mask != 0 {
                            let rem = cursor & align_mask;
                            if rem != 0 {
                                cursor += env_align - rem;
                            }
                        }
                        cursor += env_size;
                    }
                    if cursor - HEADER_BYTES <= available {
                        best = k;
                    } else {
                        break;
                    }
                }
                let batch = core::cmp::max(1, best) as u8;

                // Writer cursor starts AFTER space reserved for all headers in this batch.
                let mut w = SlotWriter {
                    base,
                    len: SLOT_BYTES,
                    cursor: HEADER_BYTES + (hdr_size * batch as usize),
                };

                // Encode each record's capture/env immediately after the header table.
                for i in 0..batch {
                    let hdr =
                        encode_closure::<_, (&mut T,), ()>(&mut w, 0, move |(p,)| (f)(p), &[]);
                    let hdr_ptr = base.add(HEADER_BYTES + (i as usize) * hdr_size)
                        as *mut RequestRecordHeader;
                    core::ptr::write_unaligned(hdr_ptr, hdr);
                }

                // Toggle request ready bit relative to response and set request count.
                let resp_word = (&*self.chan.response.get())
                    .primary
                    .header
                    .flags
                    .load(core::sync::atomic::Ordering::Acquire);
                let req_ready = !header::ready(resp_word);
                let word = header::pack_ready_count(req_ready, batch as u32);
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

                remaining -= batch;
            }
        }
    }
}
