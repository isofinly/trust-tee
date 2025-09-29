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
    let trust = Remote::entrust(0i64);

    trust.apply(incr);
    let v = trust.apply(get);
    assert_eq!(v, 1);

    let h2 = trust.clone();
    let t1 = thread::spawn({
        let h = h2;
        move || {
            for _ in 0..10_000 {
                h.apply(incr);
            }
        }
    });

    let h3 = trust.clone();
    let t2 = thread::spawn({
        let h = h3;
        move || {
            for _ in 0..10_000 {
                h.apply(incr);
            }
        }
    });

    t1.join().unwrap();
    t2.join().unwrap();

    let v = trust.apply(get);
    println!("final: {v}");
    assert_eq!(v, 1 + 20_000);
}
