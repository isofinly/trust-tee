use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use trust_tee::prelude::*;

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

fn incr_i64(c: &mut i64) {
    *c += 1;
}

fn bench_contention(c: &mut Criterion) {
    // let workers = num_cpus::get().saturating_sub(1).max(1);
    let workers = 3;
    let batch_sizes: &[u32] = &[1, 4, 16, 64];

    let mut group = c.benchmark_group("high_contention");
    group.measurement_time(Duration::from_secs(15));
    group.warm_up_time(Duration::from_millis(1200));
    group.throughput(Throughput::Elements(1));

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

    // 2b) Mutex<u64> uncontended latency baseline
    {
        let ctr = Arc::new(Mutex::new(0u64));
        group.bench_function("mutex_uncontended", |b| {
            b.iter(|| {
                let mut g = ctr.lock().unwrap();
                *g += 1;
            });
        });
    }

    // 3) LocalTrustee (single-threaded fast path; !Sync prevents shared contention)
    {
        let t = Local::entrust(0i64);
        group.bench_function("local_trustee", |b| {
            b.iter(|| {
                t.apply(|c| *c += 1);
            });
        });
    }

    // 4) RemoteRuntime with pinning, burst RR fairness, and batched clients.
    {
        #[cfg(target_os = "linux")]
        let pin_t = PinConfig {
            core_id: Some(0),
            numa_node: None,
            mem_bind: false,
            mac_affinity_tag: None,
        };
        #[cfg(target_os = "macos")]
        let pin_t = PinConfig {
            core_id: None,
            numa_node: None,
            mem_bind: false,
            mac_affinity_tag: Some(1),
        };

        let (rt, remote) = Runtime::spawn_with_pin(0i64, 64, Some(pin_t.clone()));
        let trust = Trust::new(remote);

        for &batch in batch_sizes {
            let run = Arc::new(AtomicBool::new(true));

            let threads: Vec<_> = (0..workers)
                .map(|i| {
                    let run = Arc::clone(&run);
                    let h = rt.entrust();
                    let trust = Trust::new(h);
                    #[cfg(target_os = "linux")]
                    let client_pin: Option<PinConfig> = if i == 0 {
                        Some(PinConfig {
                            core_id: Some(1),
                            numa_node: None,
                            mem_bind: false,
                            mac_affinity_tag: None,
                        })
                    } else {
                        None
                    };
                    #[cfg(target_os = "macos")]
                    let client_pin: Option<PinConfig> = if i == 0 {
                        Some(PinConfig {
                            core_id: None,
                            numa_node: None,
                            mem_bind: false,
                            mac_affinity_tag: Some(2),
                        })
                    } else {
                        None
                    };

                    thread::spawn(move || {
                        if let Some(cfg) = client_pin {
                            // Best-effort pin to keep topology predictable.
                            pin_current_thread(&cfg);
                        }
                        while run.load(Ordering::Relaxed) {
                            trust.apply_batch_mut(incr_i64, batch as u8);
                            // Yield to let trustee run; avoids perfectly hot spin loops.
                            std::thread::yield_now();
                        }
                    })
                })
                .collect();

            group.throughput(Throughput::Elements(batch as u64));
            group.bench_function(
                BenchmarkId::new("remote_trustee_batched", format!("w{workers}_b{batch}")),
                |b| {
                    b.iter(|| {
                        trust.apply_batch_mut(incr_i64, batch as u8);
                    });
                },
            );

            run.store(false, Ordering::Relaxed);
            for t in threads {
                let _ = t.join();
            }
        }
    }

    group.finish();
}

criterion_group!(benches, bench_contention);
criterion_main!(benches);
