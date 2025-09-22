use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use trust::{LocalTrustee, RemoteRuntime};

fn spawn_atomic_hammer(
    ctr: Arc<AtomicU64>,
    run: Arc<AtomicBool>,
    workers: usize,
) -> Vec<JoinHandle<()>> {
    (0..workers)
        .map(|_| {
            let ctr = Arc::clone(&ctr);
            let run = Arc::clone(&run);
            thread::spawn(move || {
                while run.load(Ordering::Relaxed) {
                    ctr.fetch_add(1, Ordering::SeqCst);
                }
            })
        })
        .collect()
}

fn spawn_mutex_hammer(
    ctr: Arc<Mutex<u64>>,
    run: Arc<AtomicBool>,
    workers: usize,
) -> Vec<JoinHandle<()>> {
    (0..workers)
        .map(|_| {
            let ctr = Arc::clone(&ctr);
            let run = Arc::clone(&run);
            thread::spawn(move || {
                while run.load(Ordering::Relaxed) {
                    if let Ok(mut g) = ctr.lock() {
                        *g += 1;
                    }
                }
            })
        })
        .collect()
}

// Remote trustee helpers use fn items (no captures) to keep calls lean.
fn incr_i64(c: &mut i64) {
    *c += 1;
}

fn bench_contention(c: &mut Criterion) {
    let workers = num_cpus::get().saturating_sub(1).max(1);

    let mut group = c.benchmark_group("high_contention");
    group.measurement_time(Duration::from_secs(10));
    group.warm_up_time(Duration::from_millis(800));
    group.throughput(Throughput::Elements(1)); // one op per iter

    // 1) AtomicU64 SeqCst under contention from background workers
    {
        let ctr = Arc::new(AtomicU64::new(0));
        let run = Arc::new(AtomicBool::new(true));
        let threads = spawn_atomic_hammer(Arc::clone(&ctr), Arc::clone(&run), workers);

        group.bench_function(BenchmarkId::new("atomic_seqcst", workers), |b| {
            b.iter(|| {
                ctr.fetch_add(1, Ordering::SeqCst);
            });
        });

        run.store(false, Ordering::Relaxed);
        for t in threads {
            let _ = t.join();
        }
    }

    // 2) Mutex<u64> under contention
    {
        let ctr = Arc::new(Mutex::new(0u64));
        let run = Arc::new(AtomicBool::new(true));
        let threads = spawn_mutex_hammer(Arc::clone(&ctr), Arc::clone(&run), workers);

        group.bench_function(BenchmarkId::new("mutex", workers), |b| {
            b.iter(|| {
                let mut g = ctr.lock().unwrap();
                *g += 1;
            });
        });

        run.store(false, Ordering::Relaxed);
        for t in threads {
            let _ = t.join();
        }
    }

    // 3) LocalTrustee (single-threaded fast path; !Sync prevents shared contention)
    {
        let lt = LocalTrustee::new();
        let t = lt.entrust(0i64);
        group.bench_function("local_trustee", |b| {
            b.iter(|| {
                t.apply(|c| *c += 1);
            });
        });
    }

    // 4) RemoteRuntime handle with background clients hammering the same trustee
    {
        let (rt, handle) = RemoteRuntime::spawn(0i64, 1024);
        let run = Arc::new(AtomicBool::new(true));

        // Background clients each get their own handle and hammer the remote trustee
        let threads: Vec<_> = (0..workers)
            .map(|_| {
                let run = Arc::clone(&run);
                let h = rt.handle();
                thread::spawn(move || {
                    while run.load(Ordering::Relaxed) {
                        h.apply_mut(incr_i64);
                    }
                })
            })
            .collect();

        group.bench_function(BenchmarkId::new("remote_trustee", workers), |b| {
            b.iter(|| {
                handle.apply_mut(incr_i64);
            });
        });

        run.store(false, Ordering::Relaxed);
        for t in threads {
            let _ = t.join();
        }
        // drop(rt, handle) at scope end
    }

    group.finish();
}

criterion_group!(benches, bench_contention);
criterion_main!(benches);
