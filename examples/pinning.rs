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

    let (_rt, h) = Runtime::spawn_with_pin(0i64, 64, Some(pin));
    let trust = Trust::new(h);

    trust.apply(incr);

    let v = trust.apply(get);

    println!("final(remote): {v}");
}
