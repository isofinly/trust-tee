use trust_tee::prelude::*;

fn main() {
    let local_trust = Local::entrust(42u32);

    let result = local_trust.apply(|x| {
        *x += 1;
        *x
    });
    println!("Local result: {}", result);

    let latch_trust = Local::entrust(Latch::new(100u32));

    let latch_result = latch_trust.lock_apply(|x| {
        *x += 10;
        *x
    });
    println!("Latch result: {}", latch_result);

    let remote_trust = Remote::entrust(200u32);

    remote_trust.apply(|x| *x += 1);
    let remote_result = remote_trust.apply(|x| *x as u64);

    println!("Remote result: {}", remote_result);

    // Runtime will be dropped and cleaned up when remote_trust is dropped
}
