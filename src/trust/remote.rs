use crate::runtime::common::{Op, Resp};
use crate::util::WaitBudget;
use core::marker::PhantomData;
use may::queue::spsc::Queue;
use std::sync::Arc;

/// Client Remote Trust to submit operations to a `Runtime<T>` and await replies.
pub struct Remote<T> {
    pub(crate) req: Arc<Queue<Op<T>>>,
    pub(crate) resp: Arc<Queue<Resp>>,
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
        todo!("Remote::apply - requires runtime integration")
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
        // Push to SPSC queue (unbounded, so no backoff needed)
        self.req.push(Op::Apply(f));

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
        // Push to SPSC queue (unbounded, so no backoff needed)
        self.req.push(Op::MapU64(f));

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
        // Push to SPSC queue (unbounded, so no backoff needed)
        self.req.push(Op::ApplyBatch(f, n));

        let mut budget = WaitBudget::hot();
        loop {
            if let Some(Resp::NoneBatch(_n)) = self.resp.pop() {
                return;
            }
            budget.step();
        }
    }
}
