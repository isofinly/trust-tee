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
    let iter: usize = std::env::var("ITER")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);

    let trust = Remote::entrust(0i64);

    trust.apply(incr);
    let v = trust.apply(get);
    assert_eq!(v, 1);

    let h2 = trust.clone();
    let t1 = thread::spawn({
        let h = h2;
        move || {
            for _ in 0..iter {
                h.apply(incr);
            }
        }
    });

    let h3 = trust.clone();
    let t2 = thread::spawn({
        let h = h3;
        move || {
            for _ in 0..iter {
                h.apply(incr);
            }
        }
    });

    t1.join().unwrap();
    t2.join().unwrap();

    let v = trust.apply(get);
    println!("final: {v}");
    assert_eq!(v, 1 + (iter as u64 * 2));
}
