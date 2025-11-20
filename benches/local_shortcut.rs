use criterion::{Criterion, criterion_group, criterion_main};
use std::time::Duration;
use trust_tee::prelude::*;
use trust_tee::runtime::Runtime;

fn bench_local_shortcut(c: &mut Criterion) {
    let mut group = c.benchmark_group("local_shortcut");
    group.measurement_time(Duration::from_secs(5));

    // 1. Benchmark: Remote thread calling apply (Standard path)
    group.bench_function("remote_thread_apply", |b| {
        let (_rt, remote) = Runtime::spawn(0usize);
        b.iter(|| {
            remote.apply(|v| *v += 1);
        });
    });

    // 2. Benchmark: Trustee thread calling apply (Shortcut path)
    // We use iter_custom to measure the loop time inside the trustee thread.
    group.bench_function("trustee_thread_apply", |b| {
        let (_rt, remote) = Runtime::spawn(0usize);
        let remote_clone = remote.clone();

        b.iter_custom(|iters| {
            let remote_clone = remote_clone.clone();
            remote.apply(move |_| {
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    remote_clone.apply(|v| *v += 1);
                }
                start.elapsed()
            })
        });
    });

    group.finish();
}

criterion_group!(benches, bench_local_shortcut);
criterion_main!(benches);
