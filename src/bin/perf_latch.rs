use std::env;

use trust_tee::{Latch, LocalTrustee};

fn main() {
    let iterations: usize = env::var("ITER")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50_000);

    let lt = LocalTrustee::new();
    let guarded = lt.entrust(Latch::new(0usize));

    for _ in 0..iterations {
        guarded.apply(|l| {
            let mut g = l.lock();
            *g += 1;
        });
    }

    let v = guarded.lock_with(|l| *l);
    println!("latch_sum={v}");
}
