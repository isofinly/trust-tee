use std::sync::atomic::{AtomicUsize, Ordering};

static THREAD_ID_COUNTER: AtomicUsize = AtomicUsize::new(1);

thread_local! {
    /// The unique ID of the current thread.
    pub static THREAD_ID: usize = THREAD_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
}

/// Get the unique ID of the current thread.
pub fn get_thread_id() -> usize {
    THREAD_ID.with(|id| *id)
}
