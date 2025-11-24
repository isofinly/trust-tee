use std::sync::{Arc, Mutex, Condvar};

/// A simple latch for synchronization.
pub struct Latch<T> {
    lock: Arc<Mutex<T>>,
    cvar: Arc<Condvar>,
}

impl<T> Latch<T> {
    /// Create a new latch with the given value.
    pub fn new(val: T) -> Self {
        Self {
            lock: Arc::new(Mutex::new(val)),
            cvar: Arc::new(Condvar::new()),
        }
    }

    /// Lock the latch to access the value.
    pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock.lock().unwrap()
    }

    /// Wait until the condition is met.
    pub fn wait(&self, condition: impl Fn(&T) -> bool) {
        let mut guard = self.lock.lock().unwrap();
        while !condition(&*guard) {
            guard = self.cvar.wait(guard).unwrap();
        }
    }

    /// Notify all waiting threads.
    pub fn notify_all(&self) {
        self.cvar.notify_all();
    }
}

impl<T: Clone> Clone for Latch<T> {
    fn clone(&self) -> Self {
        Self {
            lock: self.lock.clone(),
            cvar: self.cvar.clone(),
        }
    }
}
