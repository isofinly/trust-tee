use trust_tee::{LocalTrustee, Trust};

fn main() {
    let lt = LocalTrustee::new();
    let counter = lt.entrust(17i64);

    // Increment twice synchronously.
    counter.apply(|c| *c += 1);
    counter.apply(|c| *c += 1);

    // Read value.
    let v = counter.apply(|c| *c);
    assert_eq!(v, 19);

    // Non-blocking style (runs inline on local path).
    counter.apply_then(
        |c| {
            *c += 1;
            *c
        },
        |v| {
            assert_eq!(v, 20);
        },
    );

    let final_v = counter.apply(|c| *c);
    println!("final: {final_v}");
}
