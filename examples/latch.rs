use trust_tee::{Latch, LocalTrustee};

fn main() {
    let lt = LocalTrustee::new();
    let guarded = lt.entrust(Latch::new(0usize));

    // Run inside local trustee apply; take the latch and mutate.
    guarded.apply(|l| {
        let mut g = l.lock();
        *g += 1;
    });

    let v = guarded.apply(|l| {
        let g = l.lock();
        *g
    });
    assert_eq!(v, 1);
}
