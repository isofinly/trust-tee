use crate::runtime::common::{Op, Resp};
use crate::util::WaitBudget;
use core::marker::PhantomData;
use crossbeam_queue::ArrayQueue;
use std::sync::Arc;

/// Client Remote Trust to submit operations to a `Runtime<T>` and await replies.
pub struct Trust<T> {
    pub(crate) req: Arc<ArrayQueue<Op<T>>>,
    pub(crate) resp: Arc<ArrayQueue<Resp>>,
    pub(crate) _phantom: PhantomData<T>,
}

impl<T: Send + 'static> Trust<T> {
    #[inline]
    /// Apply a mutation on the remote property and wait for completion.
    pub fn apply_mut(&self, f: fn(&mut T)) {
        // Push with backoff if full.
        let mut budget = WaitBudget::hot();
        loop {
            if self.req.push(Op::Apply(f)).is_ok() {
                break;
            }
            budget.step();
        }
        // Pop with backoff if empty.
        let mut budget = WaitBudget::hot();
        loop {
            if let Some(Resp::None) = self.resp.pop() {
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
            if self.req.push(Op::MapU64(f)).is_ok() {
                break;
            }
            budget.step();
        }
        let mut budget = WaitBudget::hot();
        loop {
            if let Some(Resp::U64(v)) = self.resp.pop() {
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
            if self.req.push(Op::ApplyBatch(f, n)).is_ok() {
                break;
            }
            budget.step();
        }
        let mut budget = WaitBudget::hot();
        loop {
            if let Some(Resp::NoneBatch(_n)) = self.resp.pop() {
                return;
            }
            budget.step();
        }
    }
}
