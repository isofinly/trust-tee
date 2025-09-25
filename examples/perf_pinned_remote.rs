use std::env;

use trust_tee::{PinConfig, RemoteRuntime};

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

    let (_rt, handle) = RemoteRuntime::spawn_with_pin(0i64, 1024, 64, Some(pin));

    for _ in 0..iterations {
        handle.apply_mut(incr);
    }

    let sum = handle.apply_map_u64(|c| *c as u64);
    println!("pinned_remote_sum={sum}");
}
