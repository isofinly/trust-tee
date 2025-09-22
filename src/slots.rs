//! SPSC channel scaffolding using crossbeam ArrayQueue.
//!
//! Not used on the local trustee shortcut path, but wired and allocation-free
//! per operation once constructed. Construction may allocate internally;
//! ensure queues are created during initialization and reused.

use crossbeam_queue::ArrayQueue;

/// A single-producer, single-consumer fixed-capacity channel.
///
/// Create once during initialization and reuse to keep the fast path
/// allocation-free.
pub struct Spsc<T> {
    q: ArrayQueue<T>,
}

impl<T> Spsc<T> {
    /// Create a new fixed-capacity SPSC channel.
    ///
    /// Note: ArrayQueue allocates once; thereafter push/pop are allocation-free.
    pub fn new(capacity: usize) -> Self {
        Self {
            q: ArrayQueue::new(capacity),
        }
    }

    /// Try to push without blocking; returns Err(value) if full.
    #[inline]
    pub fn try_push(&self, value: T) -> Result<(), T> {
        self.q.push(value).map_err(|e| e)
    }

    /// Try to pop without blocking.
    #[inline]
    pub fn try_pop(&self) -> Option<T> {
        self.q.pop()
    }

    /// Current approximate length.
    #[inline]
    pub fn len(&self) -> usize {
        self.q.len()
    }

    /// Capacity of the channel.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.q.capacity()
    }

    /// Is the channel empty?
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }

    /// Is the channel full?
    #[inline]
    pub fn is_full(&self) -> bool {
        self.q.is_full()
    }

    /// Try push with spin-yield if full.
    #[inline]
    pub fn push_spin(&self, v: T)
    where
        T: Copy,
    {
        while self.q.push(v).is_err() {
            std::hint::spin_loop();
        }
    }

    /// Try pop with spin-yield if empty.
    #[inline]
    pub fn pop_spin(&self) -> T {
        loop {
            if let Some(v) = self.q.pop() {
                return v;
            }
            std::hint::spin_loop();
        }
    }
}
