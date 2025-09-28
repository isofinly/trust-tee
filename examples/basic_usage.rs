use trust_tee::prelude::*;

fn main() {
    let local_trust = Trust::new(Local::entrust(42u32));

    let result = local_trust.apply(|x| {
        *x += 1;
        *x
    });
    println!("Local result: {}", result);

    let latch_trust = Trust::new(Local::entrust(Latch::new(100u32)));

    let latch_result = latch_trust.lock_apply(|x| {
        *x += 10;
        *x
    });
    println!("Latch result: {}", latch_result);

    let (runtime, remote_trust) = Runtime::spawn(200u32);
    let trust = Trust::new(remote_trust);

    trust.apply(|x| *x += 1);
    let remote_result = trust.apply(|x| *x as u64);

    println!("Remote result: {}", remote_result);

    // Runtime will be dropped and cleaned up here
    drop(runtime);
}
