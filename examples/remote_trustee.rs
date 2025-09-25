use std::thread;

use trust_tee::prelude::*;

// Non-capturing functions only (fn items) to remain allocation-free.
fn incr(c: &mut i64) {
    *c += 1;
}
fn get(c: &mut i64) -> u64 {
    *c as u64
}

fn main() {
    // Spawn a trustee thread managing a single counter with SPSC queues.
    let (rt, handle) = Runtime::spawn(0i64);

    // Single-threaded usage.
    handle.apply_mut(incr);
    let v = handle.apply_map_u64(get);
    assert_eq!(v, 1);

    // Multi-threaded clients (still SPSC per property; here we serialize per handle).
    let h2 = rt.entrust();
    let t1 = thread::spawn({
        let h = h2;
        move || {
            for _ in 0..10_000 {
                h.apply_mut(incr);
            }
        }
    });

    let h3 = rt.entrust();
    let t2 = thread::spawn({
        let h = h3;
        move || {
            for _ in 0..10_000 {
                h.apply_mut(incr);
            }
        }
    });

    t1.join().unwrap();
    t2.join().unwrap();

    let v = handle.apply_map_u64(get);
    println!("final: {v}");
    assert_eq!(v, 1 + 20_000);
}
