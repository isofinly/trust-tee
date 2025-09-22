//! Single-producer/single-consumer ring buffer optimized for delegation patterns.
//! - Power-of-two capacity, cacheline padding, acquire/release atomics.
//! - One-time heap allocation at construction; zero per-op allocation.

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicUsize, Ordering};

const CACHELINE: usize = 64;

#[repr(align(64))]
struct Pad([u8; CACHELINE]);

#[repr(C)]
struct Padded<T> {
    val: T,
    _pad: Pad,
}

pub struct Spsc<T> {
    // Producer-only tail index (Release write), consumer reads with Acquire.
    tail: Padded<AtomicUsize>,
    // Consumer-only head index (Release write), producer reads with Acquire.
    head: Padded<AtomicUsize>,
    // Power-of-two capacity and mask for index wrap.
    cap: usize,
    mask: usize,
    // Buffer of MaybeUninit<T>.
    buf: Box<[UnsafeCell<MaybeUninit<T>>]>,
}

unsafe impl<T: Send> Send for Spsc<T> {}
unsafe impl<T: Send> Sync for Spsc<T> {}

impl<T> Spsc<T> {
    /// Create a ring with power-of-two capacity (rounded up if needed).
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.next_power_of_two().max(2);
        let mask = cap - 1;
        let mut v: Vec<UnsafeCell<MaybeUninit<T>>> = Vec::with_capacity(cap);
        // SAFETY: set_len after initializing with uninit elements.
        unsafe {
            v.set_len(cap);
        }
        let buf = v.into_boxed_slice();
        Self {
            tail: Padded {
                val: AtomicUsize::new(0),
                _pad: Pad([0; CACHELINE]),
            },
            head: Padded {
                val: AtomicUsize::new(0),
                _pad: Pad([0; CACHELINE]),
            },
            cap,
            mask,
            buf,
        }
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Try to push a value; Err(value) if full.
    #[inline]
    pub fn try_push(&self, value: T) -> Result<(), T> {
        let head = self.head.val.load(Ordering::Acquire);
        let tail = self.tail.val.load(Ordering::Relaxed);
        if tail.wrapping_sub(head) >= self.cap {
            return Err(value);
        }
        let idx = tail & self.mask;
        // SAFETY: single producer owns this slot before tail is published.
        unsafe {
            (*self.buf[idx].get()).as_mut_ptr().write(value);
        }
        self.tail.val.store(tail.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    /// Try to pop a value; None if empty.
    #[inline]
    pub fn try_pop(&self) -> Option<T> {
        let tail = self.tail.val.load(Ordering::Acquire);
        let head = self.head.val.load(Ordering::Relaxed);
        if head == tail {
            return None;
        }
        let idx = head & self.mask;
        // SAFETY: single consumer owns this slot before head is published.
        let val = unsafe { (*self.buf[idx].get()).as_ptr().read() };
        self.head.val.store(head.wrapping_add(1), Ordering::Release);
        Some(val)
    }

    /// Push with bounded backoff if full.
    #[inline]
    pub fn push_backoff(&self, mut value: T) {
        let mut spins = 0u32;
        loop {
            match self.try_push(value) {
                Ok(()) => return,
                Err(v) => {
                    value = v;
                    spins = backoff_step(spins);
                }
            }
        }
    }

    /// Pop with bounded backoff if empty.
    #[inline]
    pub fn pop_backoff(&self) -> T {
        let mut spins = 0u32;
        loop {
            if let Some(v) = self.try_pop() {
                return v;
            }
            spins = backoff_step(spins);
        }
    }
}

impl<T> Drop for Spsc<T> {
    fn drop(&mut self) {
        // Drain remaining initialized elements to drop them.
        let mut head = self.head.val.load(Ordering::Relaxed);
        let tail = self.tail.val.load(Ordering::Relaxed);
        while head != tail {
            let idx = head & self.mask;
            unsafe {
                std::ptr::drop_in_place((*self.buf[idx].get()).as_mut_ptr());
            }
            head = head.wrapping_add(1);
        }
    }
}

/// Simple backoff: spin, yield, then park for a very short timeout.
#[inline]
pub fn backoff_step(spins: u32) -> u32 {
    if spins < 40 {
        core::hint::spin_loop();
        spins + 1
    } else if spins < 80 {
        std::thread::yield_now();
        spins + 1
    } else {
        // Park timeout avoids full busy-wait without requiring explicit unpark.
        std::thread::park_timeout(std::time::Duration::from_micros(50));
        0
    }
}
