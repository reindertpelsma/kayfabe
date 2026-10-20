//! CPU-only batch measurements. Diagnostic runtime uses libc CLOCK_MONOTONIC (vDSO/syscall not distinguished).
use std::{hint::black_box, time::Instant};
const OPS: u64 = 20000;
fn measure(mut f: impl FnMut()) -> [f64; 7] {
    core::array::from_fn(|_| {
        let t = Instant::now();
        for _ in 0..OPS {
            f();
        }
        t.elapsed().as_nanos() as f64 / OPS as f64
    })
}
fn print(label: &str, mut samples: Vec<f64>) {
    samples.sort_by(f64::total_cmp);
    println!(
        "{label}: n={} median_ns={:.3} min_ns={:.3} max_ns={:.3}",
        samples.len(),
        samples[samples.len() / 2],
        samples[0],
        samples[samples.len() - 1]
    );
}
fn main() {
    println!("batch_ops={OPS}; seven repetitions; ns per operation; CPU-only; no GPU/VM");
    print(
        if cfg!(feature = "prof") {
            "runtime_off_span_pair"
        } else {
            "feature_off_span_pair"
        },
        measure(|| {
            let _guard = black_box(kf_prof::site(
                black_box(kf_prof::tables::Site::HostRmIoctl),
                kf_prof::tables::State::HostIoctl,
                1,
            ));
        })
        .to_vec(),
    );
    print(
        if cfg!(feature = "prof") {
            "runtime_off_call_wrapper"
        } else {
            "feature_off_call_wrapper"
        },
        measure(|| {
            kf_prof::call(
                black_box(kf_prof::tables::Site::HostRmIoctl),
                kf_prof::tables::State::HostIoctl,
                1,
                || black_box(()),
            )
        })
        .to_vec(),
    );
    let mut state = kf_prof::thread::ThreadState::new(0);
    let mut tick = 1;
    print(
        "core_span_pair",
        measure(|| {
            tick += 2;
            let t = state
                .enter(
                    kf_prof::tables::State::Trap,
                    kf_prof::tables::Site::TrapDispatch,
                    black_box(tick),
                    tick,
                )
                .unwrap();
            black_box(state.leave(t, tick + 1));
            black_box(&state);
        })
        .to_vec(),
    );
    let mut ledger = kf_prof::ledger::Ledger::<13, 16>::new(1);
    print(
        "core_ledger_open_close",
        measure(|| {
            let t = ledger.open(8, black_box(7), 1).unwrap();
            black_box(ledger.close(t, 2));
            black_box(&ledger);
        })
        .to_vec(),
    );
    let mut h = kf_prof::histogram::Histogram::new();
    print(
        "core_histogram",
        measure(|| {
            h.record(black_box(270), 1);
            black_box(&h);
        })
        .to_vec(),
    );
    let cal = black_box(kf_prof::clock::Calibration::new(0, 1, 3000000000, 1, 1).unwrap());
    print(
        "clock_conversion",
        measure(|| {
            black_box(cal.to_ns(black_box(30000)).unwrap());
        })
        .to_vec(),
    );
    let mut ring = kf_prof::ring::Ring::<1024>::new().unwrap();
    let (mut p, mut c) = ring.split();
    let event = kf_prof::ring::Event {
        tag: 1,
        ..Default::default()
    };
    print(
        "trap_event_ring_roundtrip_uncontended",
        measure(|| {
            assert!(p.push(black_box(event)));
            black_box(c.pop().unwrap());
        })
        .to_vec(),
    );
    let mut samples = Vec::new();
    let mut dropped = 0;
    for _ in 0..7 {
        let mut ring = kf_prof::ring::Ring::<1024>::new().unwrap();
        let (mut p, mut c) = ring.split();
        let done = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|s| {
            s.spawn(|| {
                loop {
                    if let Some(v) = c.pop() {
                        black_box(v);
                    } else if done.load(std::sync::atomic::Ordering::Acquire) {
                        break;
                    } else {
                        std::hint::spin_loop();
                    }
                }
            });
            let t = Instant::now();
            for _ in 0..OPS {
                p.push(black_box(event));
            }
            samples.push(t.elapsed().as_nanos() as f64 / OPS as f64);
            dropped += p.dropped();
            done.store(true, std::sync::atomic::Ordering::Release);
        });
    }
    print("trap_event_ring_attempt_concurrent_consumer", samples);
    println!("concurrent_ring_dropped={dropped}; counted refusals; no producer retries");
    #[cfg(feature = "prof-inject")]
    diagnostic_runtime();
}
#[cfg(feature = "prof-inject")]
fn diagnostic_runtime() {
    use kf_prof::{
        runtime::Session,
        tables::{Kind, Site, State},
    };
    println!(
        "enabled_runtime_clock=DIAGNOSTIC_OS_CLOCK via libc; vDSO/syscall not distinguished; native invariant-TSC cost UNMEASURED"
    );
    for n in [1, 4, 8] {
        for op in [0, 1, 2, 3] {
            let path =
                std::env::temp_dir().join(format!("kf-prof-bench-{}-{n}-{op}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            let session = Session::create_diagnostic(&path).unwrap();
            let mut all = Vec::new();
            for _ in 0..7 {
                let barrier = std::sync::Barrier::new(n);
                std::thread::scope(|scope| {
                    let mut threads = Vec::new();
                    for _ in 0..n {
                        let s = &session;
                        let b = &barrier;
                        threads.push(scope.spawn(move || {
                            let _binding = s.bind_current(81).unwrap();
                            let mut relay = if op == 2 {
                                Some(s.register_relay(8).unwrap())
                            } else {
                                None
                            };
                            b.wait();
                            let t = Instant::now();
                            for i in 0..5000 {
                                match op {
                                    0 => drop(black_box(kf_prof::site(
                                        Site::HostRmIoctl,
                                        State::HostIoctl,
                                        i,
                                    ))),
                                    1 => {
                                        let token =
                                            kf_prof::ledger_open(Kind::ForwardedRm, i).unwrap();
                                        black_box(kf_prof::ledger_close(token));
                                    }
                                    3 => kf_prof::call(
                                        Site::HostRmIoctl,
                                        State::HostIoctl,
                                        i,
                                        || black_box(()),
                                    ),
                                    _ => relay.as_mut().unwrap().observe(
                                        kf_prof::relay::Observation {
                                            private_get: Some((i % 512) as u32),
                                            private_put: Some(500),
                                            ..Default::default()
                                        },
                                    ),
                                }
                            }
                            t.elapsed().as_nanos() as f64 / 5000.0
                        }));
                    }
                    for t in threads {
                        all.push(t.join().unwrap());
                    }
                });
            }
            print(
                &format!(
                    "diagnostic_runtime_{}_threads_{n}",
                    if op == 0 {
                        "span_pair"
                    } else if op == 1 {
                        "ledger_pair"
                    } else if op == 2 {
                        "relay_observation"
                    } else {
                        "call_wrapper"
                    }
                ),
                all,
            );
            session.stop().unwrap();
            std::fs::remove_dir_all(path).unwrap();
        }
    }
}
