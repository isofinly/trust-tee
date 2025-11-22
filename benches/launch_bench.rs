use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
use std::time::Duration;
use criterion::{Criterion, criterion_group, criterion_main, BenchmarkId};
use trust_tee::runtime::Runtime;
use trust_tee::prelude::*;

fn bench_launch(c: &mut Criterion) {
    let mut group = c.benchmark_group("launch_perf");
    group.measurement_time(Duration::from_secs(10));

    let (rt, trust) = Runtime::spawn(0u64);
    // Keep runtime alive
    let _rt_guard = rt;

    group.bench_function("launch_noop", |b| {
        b.iter(|| {
            trust.launch(|_t: &u64| {
                // no-op
            });
        });
    });

    group.finish();
}

criterion_group!(benches, bench_launch);
criterion_main!(benches);
