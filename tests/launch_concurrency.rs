use trust_tee::prelude::*;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::channel;
use std::time::Duration;

#[test]
fn test_launch_concurrency() {
    // Scenario:
    // 1. Client A launches a fiber that waits for a signal.
    // 2. Client B sends an apply request that sends the signal.
    // 3. If launch blocks the runtime, Client B's request is never processed, signal is never sent -> Deadlock (timeout).
    // 4. If launch is concurrent, Client B runs, sends signal, Client A completes.

    let latch = Latch::new(0u32);
    let remote_a = Remote::entrust(latch);
    let remote_b = remote_a.clone();

    let (tx, rx) = channel();
    let tx = Arc::new(Mutex::new(tx));
    let rx = Arc::new(Mutex::new(rx));

    // Client A: Launch and wait
    let rx_clone = rx.clone();
    let handle_a = std::thread::spawn(move || {
        remote_a.launch(move |_| {
            // Wait for signal from Client B
            // We use a timeout to fail fast if deadlock
            let rx = rx_clone.lock().unwrap();
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(_) => {
                    println!("Client A: Received signal!");
                }
                Err(_) => {
                    panic!("Client A: Timed out waiting for signal! Deadlock detected.");
                }
            }
        });
    });

    // Give Client A time to send request and block
    std::thread::sleep(Duration::from_millis(100));

    // Client B: Apply and signal
    let tx_clone = tx.clone();
    let handle_b = std::thread::spawn(move || {
        remote_b.apply(move |val| {
            println!("Client B: Running apply. Value: {}", *val);
            // Signal Client A
            tx_clone.lock().unwrap().send(()).unwrap();
            *val.lock() += 1;
        });
    });

    handle_a.join().unwrap();
    handle_b.join().unwrap();
}
