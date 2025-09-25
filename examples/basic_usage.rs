use trust_tee::prelude::*;

fn main() {
    // Local trust - single-threaded, no atomics
    let local_trust = Trust::new(Local::entrust(42u32));

    // Use local trust
    let result = local_trust.apply(|x| {
        *x += 1;
        *x
    });
    println!("Local result: {}", result);

    // Use with Latch for single-threaded mutual exclusion
    let latch_trust = Trust::new(Local::entrust(Latch::new(100u32)));

    // Use the launch APIs for Latch-protected operations
    let latch_result = latch_trust.lock_apply(|x| {
        *x += 10;
        *x
    });
    println!("Latch result: {}", latch_result);

    // Remote trust - cross-thread execution
    let (runtime, remote_trust) = Runtime::spawn(200u32);

    // Use remote trust with function pointers
    remote_trust.apply_mut(|x| *x += 1);
    let remote_result = remote_trust.apply_map_u64(|x| *x as u64);
    println!("Remote result: {}", remote_result);

    // Runtime will be dropped and cleaned up here
    drop(runtime);
}
