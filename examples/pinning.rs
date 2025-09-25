use trust_tee::prelude::*;

fn incr(c: &mut i64) {
    *c += 1;
}
fn get(c: &mut i64) -> u64 {
    *c as u64
}

fn main() {
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

    // Spawn a pinned trustee with fixed-capacity queues.
    let (_rt, h) = Runtime::spawn_with_pin(0i64, 1024, 64, Some(pin));

    h.apply_mut(incr);
    let v = h.apply_map_u64(get);
    println!("final(remote): {v}");
}
