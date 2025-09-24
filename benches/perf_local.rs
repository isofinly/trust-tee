use std::env;

use trust_tee::LocalTrustee;

fn main() {
    let iterations: usize = env::var("ITER")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50_000);

    let trustee = LocalTrustee::new();
    let counter = trustee.entrust(0i64);

    for _ in 0..iterations {
        counter.apply(|c| *c += 1);
    }

    let sum = counter.apply(|c| *c);
    println!("local_sum={sum}");
}
