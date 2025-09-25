/// Lightweight wait budget: bounded spin then yield; never parks by default.
#[derive(Copy, Clone)]
pub struct WaitBudget {
    spins: u32,
    yields: u32,
    spin_cap: u32,
    yield_cap: u32,
}

impl WaitBudget {
    #[inline]
    pub fn hot() -> Self {
        Self {
            spins: 0,
            yields: 0,
            spin_cap: 128,
            yield_cap: 8,
        }
    }

    #[inline]
    pub fn reset(&mut self) {
        self.spins = 0;
        self.yields = 0;
    }

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
}
