use std::env;

use trust_tee::LocalTrustee;

#[inline(never)]
fn touch(v: i64) {
    std::hint::black_box(v);
}

fn main() {
    let iterations: usize = env::var("ITER")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50_000);

    let trustee = LocalTrustee::new();
    let counter = trustee.entrust(0i64);

    // Warmup to stabilize cache state
    for _ in 0..(iterations / 10).max(1) {
        counter.apply(|c| *c += 1);
    }

    for _ in 0..iterations {
        counter.apply(|c| *c += 1);
    }

    let sum = counter.apply(|c| *c);
    touch(sum);
    println!("local_sum={sum}");
}
