use trust_tee::prelude::*;
use std::time::Duration;

fn main() {
    // Test Remote Launch
    println!("Testing Remote Launch...");
    let latch = Latch::new(0);
    let remote = Remote::entrust(latch);

    // Launch a fiber that increments the value after a delay
    let result = remote.launch(|l| {
        println!("Remote Fiber started");
        trust_tee::util::fiber::sleep(Duration::from_millis(100));
        let mut guard = l.lock();
        *guard += 1;
        println!("Remote Fiber incremented value to {}", *guard);
        *guard
    });

    println!("Remote Launch returned: {}", result);
    assert_eq!(result, 1);

    // Verify value
    let val = remote.with(|l| *l.lock());
    println!("Remote Final value: {}", val);
    assert_eq!(val, 1);

    // Test Local Launch
    println!("Testing Local Launch...");
    let latch_local = Latch::new(10);
    let local = Local::entrust(latch_local);

    let res_local = local.launch(|l| {
        println!("Local Fiber started");
        trust_tee::util::fiber::sleep(Duration::from_millis(50));
        let mut guard = l.lock();
        *guard += 1;
        *guard
    });
    println!("Local Launch returned: {}", res_local);
    assert_eq!(res_local, 11);
}
