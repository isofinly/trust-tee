use std::time::Instant;
use trust_tee::prelude::*;

fn main() {
    // 1. Create a remote trust.
    // We need to run code ON the trustee thread to test the shortcut.
    // The only way to run code on the trustee thread is to `apply` a closure.

    let remote = Remote::entrust(0usize);

    // 2. Define a closure that calls `apply` recursively.
    // Without the shortcut, this would deadlock because the outer `apply` holds the trustee thread,
    // and the inner `apply` would try to send a message to the trustee thread (itself) and wait for a response.

    let remote_clone: Remote<usize> = remote.clone();

    println!("Starting recursive apply test...");
    let start = Instant::now();

    let result = remote.apply(move |val| {
        *val += 1;
        // Recursive call!
        // This should execute directly via the shortcut.
        remote_clone.apply(|v| {
            *v += 1;
        });
        *val
    });

    let duration = start.elapsed();
    println!("Recursive apply finished in {:?}. Result: {}", duration, result);

    assert_eq!(result, 2);

    // 3. Benchmark the shortcut performance.
    // We'll do a loop inside the trustee thread.

    let remote_clone2 = remote.clone();
    remote.apply(move |_| {
        let start = Instant::now();
        let iters = 1_000_000;
        for _ in 0..iters {
            remote_clone2.apply(|v| *v += 1);
        }
        let duration = start.elapsed();
        println!("{} iterations of local apply took {:?} ({:?}/op)", iters, duration, duration / iters);
    });
}
