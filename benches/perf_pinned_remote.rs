use std::env;

use trust_tee::prelude::*;

#[inline(never)]
fn touch(v: u64) {
    std::hint::black_box(v);
}

fn incr(c: &mut i64) {
    *c += 1;
}

fn main() {
    let iterations: usize = env::var("ITER")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50_000);

    #[cfg(target_os = "linux")]
    let pin = PinConfig {
        core_id: Some(0),
        numa_node: Some(0),
        mem_bind: true,
        mac_affinity_tag: None,
    };
    #[cfg(target_os = "macos")]
    let pin = PinConfig {
        core_id: None,
        numa_node: None,
        mem_bind: false,
        mac_affinity_tag: Some(1),
    };

    let trust = Remote::entrust_with_pin(0i64, pin);

    // Warmup to stabilize cache state
    for _ in 0..(iterations / 10).max(1) {
        trust.apply(incr);
    }

    for _ in 0..iterations {
        trust.apply(incr);
    }

    let sum = trust.apply(|c| *c as u64);
    touch(sum);
    println!("pinned_remote_sum={sum}");
}
