use crate::runtime::serialization::{SlotWriter, encode_closure};
use crate::runtime::slots::{ChannelPair, HEADER_BYTES, RequestRecordHeader, SLOT_BYTES, header};
use crate::util::WaitBudget;
use core::marker::PhantomData;
use std::sync::Arc;

/// Client Remote Trust to submit operations to a `Runtime<T>` and await replies.
pub struct Remote<T> {
    pub(crate) chan: Arc<ChannelPair>,
    pub(crate) _phantom: PhantomData<T>,
}

impl<T: Send + 'static> super::common::TrustLike for Remote<T> {
    type Value = T;
    #[inline]
    /// Create a `Trust<T>` (remote) holding `value` managed by a remote trustee.
    /// Note: This requires a runtime to be set up separately.
    fn entrust(_value: T) -> Self {
        todo!("Remote::entrust - requires runtime setup")
    }

    #[inline]
    /// Mutably apply `f` to the inner `T` and return its result.
    fn apply<R>(&self, _f: impl FnOnce(&mut T) -> R) -> R {
        todo!("Remote::apply_then - requires runtime integration")
    }

    #[inline]
    /// Apply `f`, then enqueue `then` to run with the result.
    fn apply_then<R>(&self, _f: impl FnOnce(&mut T) -> R, _then: impl FnOnce(R)) {
        todo!("Remote::apply_then - requires runtime integration")
    }

    #[inline]
    /// Mutably apply `f` with an extra argument `w`, returning its result.
    fn apply_with<V, R>(&self, _f: impl FnOnce(&mut T, V) -> R, _w: V) -> R {
        todo!("Remote::apply_with - requires runtime integration")
    }

    #[inline]
    /// Borrow the inner `T` immutably to compute `R`.
    fn with<R>(&self, _f: impl FnOnce(&T) -> R) -> R {
        todo!("Remote::with - requires runtime integration")
    }
}

impl<T: Send + 'static> Remote<T> {
    #[inline]
    /// Apply a mutation on the remote property and wait for completion.
    pub fn apply_mut(&self, f: fn(&mut T)) {
        let base = self.chan.request.get() as *mut u8;
        let mut w = SlotWriter {
            base,
            len: SLOT_BYTES,
            cursor: HEADER_BYTES + core::mem::size_of::<RequestRecordHeader>(),
        };
        let hdr = encode_closure::<_, (&mut T,), ()>(&mut w, 0, move |(p,)| (f)(p), &[]);
        unsafe {
            let hdr_ptr = base.add(HEADER_BYTES) as *mut RequestRecordHeader;
            core::ptr::write_unaligned(hdr_ptr, hdr);
        }
        // Signal request ready: toggle bit relative to response
        let resp_flags = self.chan.response.get() as *const core::sync::atomic::AtomicU32;
        let resp_word = unsafe { (*resp_flags).load(core::sync::atomic::Ordering::Acquire) };
        let req_ready = !header::ready(resp_word);
        let word = header::pack_ready_count(req_ready, 1);
        let req_flags = self.chan.request.get() as *const core::sync::atomic::AtomicU32;
        unsafe { (*req_flags).store(word, core::sync::atomic::Ordering::Release) };

        let mut budget = WaitBudget::hot();
        loop {
            let resp_word_now =
                unsafe { (*resp_flags).load(core::sync::atomic::Ordering::Acquire) };
            if header::ready(resp_word_now) == req_ready {
                break;
            }
            budget.step();
        }
    }

    #[inline]
    /// Apply a function on the remote property and return a `u64` result.
    pub fn apply_map_u64(&self, f: fn(&mut T) -> u64) -> u64 {
        unsafe {
            let base = self.chan.request.get() as *mut u8;
            let mut w = SlotWriter {
                base,
                len: SLOT_BYTES,
                cursor: HEADER_BYTES + core::mem::size_of::<RequestRecordHeader>(),
            };
            // Encode closure that returns u64; trustee writes it into response
            let hdr = encode_closure::<_, (&mut T,), u64>(&mut w, 1, move |(p,)| (f)(p), &[]);
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
            let le = core::ptr::read_unaligned(resp_data.cast::<u64>());
            u64::from_le(le)
        }
    }

    #[inline]
    // TODO: There's some performance left on the table here.
    /// Apply a mutation `n` times on the remote property and wait for completion.
    pub fn apply_batch_mut(&self, f: fn(&mut T), n: u8) {
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
