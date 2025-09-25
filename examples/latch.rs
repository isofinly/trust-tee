use trust_tee::prelude::*;

fn main() {
    let guarded = Trust::new(Local::entrust(Latch::new(0usize)));

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

    // Trust<Latch<T>> helper APIs (local path MVP)
    // 1) lock_apply: mutate under the latch and return a value
    let len = Trust::new(Local::entrust(Latch::new(Vec::<u32>::new()))).lock_apply(|v| {
        v.push(10);
        v.push(20);
        v.len()
    });
    assert_eq!(len, 2);

    // 2) lock_apply_then: continuation runs on client (immediate in MVP)
    let mut observed = 0usize;
    let vec_t = Trust::new(Local::entrust(Latch::new(Vec::<u8>::new())));
    vec_t.lock_apply_then(
        |v| {
            v.extend_from_slice(&[1, 2, 3, 4]);
            v.len()
        },
        |l| {
            observed = l;
        },
    );
    assert_eq!(observed, 4);

    // 3) lock_apply_with: pass an out-of-band payload
    let pushed = vec_t.lock_apply_with(
        |v, to_add: &[u8]| {
            v.extend_from_slice(to_add);
            v.len()
        },
        &[9, 9][..],
    );
    assert_eq!(pushed, 6);

    // 4) lock_with: read-only access while holding the latch
    let sum = vec_t.lock_with(|v| v.iter().copied().map(|x| x as u32).sum::<u32>());
    assert_eq!(sum, 1 + 2 + 3 + 4 + 9 + 9);
}
