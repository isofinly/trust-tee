/// Lightweight wait budget: bounded spin then yield; never parks by default.
#[derive(Copy, Clone)]
pub struct WaitBudget {
    spins: u32,
    yields: u32,
    spin_cap: u32,
    yield_cap: u32,
}

impl WaitBudget {
    /// Create a hot wait budget optimized for high contention.
    #[inline]
    pub fn hot() -> Self {
        Self {
            spins: 0,
            yields: 0,
            spin_cap: 128,
            yield_cap: 8,
        }
    }

    /// Reset the wait budget counters.
    #[inline]
    pub fn reset(&mut self) {
        self.spins = 0;
        self.yields = 0;
    }

    /// Perform one step of the wait strategy (spin, yield, or continue spinning).
    #[inline]
    pub fn step(&mut self) {
        if self.spins < self.spin_cap {
            core::hint::spin_loop();
            self.spins += 1;
        } else if self.yields < self.yield_cap {
            std::thread::yield_now();
            self.yields += 1;
        } else {
            // Stay hot without parking to avoid scheduler-induced latency.
            core::hint::spin_loop();
        }
    }

    /// Helper function to acquire a lock using WaitBudget for efficient spinning/yielding
    #[inline]
    pub fn acquire_lock_with_budget<F, R>(mut acquire_fn: F)
    where
        F: FnMut() -> Result<R, R>,
    {
        let mut budget = WaitBudget::hot();
        loop {
            match acquire_fn() {
                Ok(_value) => break,
                Err(_) => budget.step(),
            }
        }
    }
}

impl Default for WaitBudget {
    fn default() -> Self {
        Self {
            spins: 0,
            yields: 0,
            spin_cap: 64,
            yield_cap: 4,
        }
    }
}
