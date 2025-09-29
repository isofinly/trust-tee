use std::env;

use trust_tee::prelude::*;

#[inline(never)]
fn touch<T: Copy>(v: T) {
    std::hint::black_box(v);
}

fn main() {
    let iterations: usize = env::var("ITER")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50_000);

    let guarded = Local::entrust(Latch::new(0usize));

    // Warmup to stabilize cache state
    for _ in 0..(iterations / 10).max(1) {
        guarded.apply(|l| {
            let mut g = l.lock();
            *g += 1;
        });
    }

    for _ in 0..iterations {
        guarded.apply(|l| {
            let mut g = l.lock();
            *g += 1;
        });
    }

    let v = guarded.lock_with(|l| *l);
    touch(v);
    println!("latch_sum={:?}", v);
}
