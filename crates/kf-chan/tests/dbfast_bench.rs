//! ★ **Doorbell fast-path measurements, GPU-free** (`docs/design/V3_DOORBELL_IOEVENTFD.md` §7).
//! Opt-in (they are measurements, not assertions): `KF_DBFAST_BENCH=1 cargo test --release -p kf-chan
//! --test dbfast_bench -- --nocapture --test-threads 1`. Without it each prints `DBFAST-BENCH: SKIPPED`.
//! ⚠ On a vast box these numbers are NESTED (the box is itself a KVM guest); label them so.
//!
//! A. **vCPU cost of one doorbell store**, real KVM guest: trapped (exit to userspace + the device's
//!    trap arm) vs ioeventfd (in-kernel match + eventfd signal), as a function of how many OTHER tokens
//!    are registered at the same address (the kernel scans same-address ioeventfds linearly).
//! B. **eventfd signal → drainer delivery**, the fast path's other half: a loop that mirrors the
//!    device's drainer (doorbells polled before every privileged apply and at the top of the loop,
//!    doorbell eventfds in the same epoll set as its wake, park/wait/unpark), under concurrent
//!    privileged-register traffic with a configurable apply cost.

mod common;

use common::{DB_OFF, DevSink, Drainer, Guest, Host, KvmVerb, caps, kick, loop_stores, plane};
use kf_chan::dbfast::{DbFast, Delivered, RegOutcome, Sink};
use kf_chip::hwref::DieGroup;
use kf_core::{HostOps, HostSlice, Owner, Plane, Step, Translatable};
use kf_linux_raw::{Notifier, PollTimeout, ReadyTokens};
use kf_trap::tokenindex::{GuestTokenFormat, TokenIndex};
use kf_trap::{Action, Class, RegWrite, Route, Wake};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn enabled(name: &str) -> bool {
    let on = std::env::var("KF_DBFAST_BENCH").is_ok_and(|v| v == "1");
    if !on {
        eprintln!(
            "DBFAST-BENCH: SKIPPED {name} (set KF_DBFAST_BENCH=1; measurements, not assertions)"
        );
    }
    on && kf_linux_raw::kvm_gate::kvm_available()
}

fn pct(v: &mut [u64], p: f64) -> u64 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let i = (((v.len() as f64) * p / 100.0).ceil() as usize).clamp(1, v.len()) - 1;
    v[i]
}

/// A. One store's vCPU cost: trapped vs ioeventfd, vs the number of same-address registrations.
#[test]
fn bench_a_vcpu_cost_per_doorbell_store() {
    if !enabled("bench_a_vcpu_cost_per_doorbell_store") {
        return;
    }
    const N: u32 = 40_000;
    let fmt = GuestTokenFormat::for_die_group(DieGroup::Ga10x).expect("format");
    let ix = TokenIndex::RunlistVector;
    println!(
        "DBFAST-BENCH A: ns per guest doorbell store, {N} stores in a loop (wall time of the vCPU loop / {N})"
    );
    println!("  others_registered  trapped_ns  ioeventfd_ns  ratio  deliveries  stores/delivery");
    for others in [0u32, 15, 127, 511] {
        let plane = plane();
        let c = caps();
        // The timed token: chid 0x7FF on runlist 0 (registered LAST, the worst scan position).
        let (idx, v) = (
            ix.of_channel(0, 0x7FF).unwrap(),
            fmt.value(0, 0x7FF).unwrap(),
        );
        plane
            .allocate_channel(
                &mut c.lock().unwrap(),
                idx,
                Route::Passthrough,
                0xAAA,
                Owner::User,
            )
            .expect("allocate");
        let g = Guest::new(|db| loop_stores(db, &[v], N));
        let host = Arc::new(Host::default());
        let sink = Arc::new(DevSink::new(plane, Arc::clone(&host)));
        let k = kick();
        let f = Arc::new(DbFast::new(1 << 20, k).expect("fast path"));
        f.enable(Box::new(KvmVerb(Arc::clone(&g.vm))));
        f.site_add(g.doorbell);
        for chid in 1..=others {
            let _ = f.register(ix.of_channel(1, chid).unwrap(), fmt.value(1, chid).unwrap());
        }
        // Trapped: the timed value is not registered (the kernel scans the others, then exits).
        let mut vcpu = g.vcpu();
        let t = Instant::now();
        g.run(&mut vcpu, |x| sink.trap(x));
        let trapped = t.elapsed();
        // ioeventfd: registered; a drainer delivers.
        let d = Drainer::start(&f, k, &sink);
        assert!(matches!(
            f.register(idx, v),
            RegOutcome::Registered { placed: 1, .. }
        ));
        vcpu.enter_flat_protected_mode(g.code_gpa, 0).expect("pm");
        let t = Instant::now();
        g.run(&mut vcpu, |x| sink.trap(x));
        let fast = t.elapsed();
        let l = f.deregister(idx).expect("acked");
        drop(d);
        let per = |d: Duration| d.as_nanos() as f64 / f64::from(N);
        println!(
            "  {others:>17}  {:>10.0}  {:>12.0}  {:>5.1}  {:>10}  {:>15.1}",
            per(trapped),
            per(fast),
            per(trapped) / per(fast),
            l.wakes,
            l.doorbells as f64 / l.wakes.max(1) as f64
        );
        assert_eq!(l.doorbells, u64::from(N), "every store delivered");
    }
}

/// The device's drainer, as far as the doorbell/register interplay goes: `HostOps::apply_register`
/// polls doorbells first, then "applies" (a busy spin of `apply_ns`).
struct BenchHost {
    fast: Arc<DbFast>,
    sink: Arc<StampSink>,
    apply_ns: u64,
    applied: AtomicU64,
}
impl HostOps for BenchHost {
    fn ring_host(&self, _: u32) {}
    fn run_translated(&self, _: u32, _: u64) -> bool {
        false
    }
    fn run_emulated(&self, _: u32, _: u64) {}
    fn apply_register(&self, _: u8, _: u32, _: u64, _: u8) {
        self.fast.service_ready(&*self.sink);
        let t = Instant::now();
        while (t.elapsed().as_nanos() as u64) < self.apply_ns {
            std::hint::spin_loop();
        }
        self.applied.fetch_add(1, Ordering::Relaxed);
    }
    fn operands_translatable(&self, _: u32, _: u64) -> Translatable {
        Translatable::Yes
    }
    fn forge_completion(&self, _: u32) {}
    fn refuse_and_poison(&self, _: u32) {}
    fn fault_channel(&self, _: u32) {}
    fn map_guest_slice(&self, _: HostSlice) {}
    fn teardown_step(&self, _: Step) {}
}

/// A sink that stamps each delivery (the passthrough ring would be the next instruction).
struct StampSink {
    plane: &'static Plane<'static>,
    last: Mutex<Option<Instant>>,
    n: AtomicU64,
}
impl Sink for StampSink {
    fn deliver(&self, _idx: u32, value: u32, _tag: u64) -> Delivered {
        let a = self
            .plane
            .trap_write(Class::Doorbell, 0, DB_OFF, u64::from(value), 4);
        *self.last.lock().unwrap() = Some(Instant::now());
        self.n.fetch_add(1, Ordering::Release);
        match a {
            Action::RingHostInline { .. } => Delivered::Rang { reached: true },
            _ => Delivered::Absorbed,
        }
    }
}

/// B. eventfd signal → drainer delivery, idle and under concurrent privileged-register traffic.
#[test]
fn bench_b_signal_to_drainer_delivery_under_register_traffic() {
    if !enabled("bench_b_signal_to_drainer_delivery_under_register_traffic") {
        return;
    }
    const SAMPLES: usize = 3000;
    println!(
        "DBFAST-BENCH B: eventfd signal -> drainer delivery, {SAMPLES} doorbells one at a time, 20-400 us apart"
    );
    println!(
        "  scenario                                   p50_us   p99_us   max_us  reg_writes_applied"
    );
    // (name, register writes/s, apply cost µs, spin-after-delivery µs — the device's
    // `KF3_DBFAST_SPIN_US` experiment; 0 = off, the default).
    for (name, rate, apply_us, spin_us) in [
        ("idle drainer (no register traffic)", 0u64, 0u64, 0u64),
        ("2k reg writes/s, 20 us apply (~4% busy)", 2_000, 20, 0),
        ("10k reg writes/s, 20 us apply (~20% busy)", 10_000, 20, 0),
        ("10k reg writes/s, 60 us apply (~60% busy)", 10_000, 60, 0),
        ("idle drainer + 500 us spin-after-delivery", 0, 0, 500),
        (
            "10k reg/s 20 us + 500 us spin-after-delivery",
            10_000,
            20,
            500,
        ),
    ] {
        let plane = plane();
        let c = caps();
        let fmt = GuestTokenFormat::for_die_group(DieGroup::Ga10x).expect("format");
        let ix = TokenIndex::RunlistVector;
        let (idx, v) = (ix.of_channel(0, 5).unwrap(), fmt.value(0, 5).unwrap());
        plane
            .allocate_channel(
                &mut c.lock().unwrap(),
                idx,
                Route::Passthrough,
                0xAAA,
                Owner::User,
            )
            .expect("allocate");
        let drainer_efd: &'static Notifier = kick();
        let f = Arc::new(DbFast::new(64, drainer_efd).expect("fast path"));
        // A stand-in KVM verb: placements succeed; the benchmark signals the eventfd itself (the same
        // eventfd_signal → epoll wake path KVM's ioeventfd uses).
        struct Always;
        impl kf_chan::dbfast::Ioeventfd for Always {
            fn set(
                &self,
                _: u64,
                _: u32,
                _: std::os::fd::BorrowedFd<'_>,
                _: bool,
            ) -> Result<(), i32> {
                Ok(())
            }
        }
        f.enable(Box::new(Always));
        f.site_add(0x90);
        let tag = match f.register(idx, v) {
            RegOutcome::Registered { tag, .. } => tag,
            other => panic!("{other:?}"),
        };
        let sink = Arc::new(StampSink {
            plane,
            last: Mutex::new(None),
            n: AtomicU64::new(0),
        });
        let host = Arc::new(BenchHost {
            fast: Arc::clone(&f),
            sink: Arc::clone(&sink),
            apply_ns: apply_us * 1000,
            applied: AtomicU64::new(0),
        });
        let stop = Arc::new(AtomicBool::new(false));
        // The drainer, mirroring `Device::drainer_loop`.
        let drainer = {
            let (f, sink, host, stop) = (
                Arc::clone(&f),
                Arc::clone(&sink),
                Arc::clone(&host),
                Arc::clone(&stop),
            );
            std::thread::spawn(move || {
                let poller = f.poller();
                poller.watch(drainer_efd.as_source_fd(), 0).expect("watch");
                let spin = Duration::from_micros(spin_us);
                let mut last: Option<Instant> = None;
                while !stop.load(Ordering::Acquire) {
                    if f.service_ready(&*sink) > 0 {
                        last = Some(Instant::now());
                    }
                    let seen = plane.drainer_wake.seen();
                    if plane.drainer_pass(&*host, 256) > 0 {
                        continue;
                    }
                    if let Some(t) = last
                        && t.elapsed() < spin
                        && plane.ring.occupancy() == 0
                    {
                        std::hint::spin_loop();
                        continue;
                    }
                    if !plane.drainer_wake.try_park(seen) {
                        continue;
                    }
                    let mut ready = ReadyTokens::new();
                    let got = poller.wait(&mut ready, PollTimeout::Millis(50));
                    let t = Instant::now();
                    plane.drainer_wake.unpark();
                    let _ = drainer_efd.drain();
                    if got.is_ok() && f.on_ready(&ready, t, &*sink) > 0 {
                        last = Some(Instant::now());
                    }
                }
            })
        };
        // Register traffic: pushes into the privileged ring, waking the drainer as a vCPU would.
        let traffic = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                if rate == 0 {
                    return;
                }
                let gap = Duration::from_nanos(1_000_000_000 / rate);
                let mut next = Instant::now();
                while !stop.load(Ordering::Acquire) {
                    let _ = plane.ring.push(RegWrite {
                        bar: 0,
                        offset: 0x0011_0c00,
                        value: 1,
                        width: 4,
                    });
                    if plane.drainer_wake.bump() == Wake::SignalOne {
                        let _ = drainer_efd.signal();
                    }
                    next += gap;
                    while Instant::now() < next {
                        std::hint::spin_loop();
                    }
                }
            })
        };
        // The doorbell source: signal the registration's eventfd, wait for its delivery.
        let efd = f.efd_for_bench(tag).expect("the registration's eventfd");
        let mut lat = Vec::with_capacity(SAMPLES);
        let mut seed = 0x1234_5678_u64;
        for _ in 0..SAMPLES {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            std::thread::sleep(Duration::from_micros(20 + (seed >> 33) % 380));
            let before = sink.n.load(Ordering::Acquire);
            let t0 = Instant::now();
            efd.signal().expect("signal");
            let deadline = t0 + Duration::from_secs(1);
            while sink.n.load(Ordering::Acquire) == before && Instant::now() < deadline {
                std::hint::spin_loop();
            }
            if let Some(t1) = *sink.last.lock().unwrap() {
                lat.push(t1.saturating_duration_since(t0).as_nanos() as u64);
            }
        }
        stop.store(true, Ordering::Release);
        let _ = drainer_efd.signal();
        drainer.join().unwrap();
        traffic.join().unwrap();
        let us = |ns: u64| ns as f64 / 1000.0;
        let (p50, p99) = (pct(&mut lat, 50.0), pct(&mut lat, 99.0));
        println!(
            "  {name:<42} {:>7.1}  {:>7.1}  {:>7.1}  {}",
            us(p50),
            us(p99),
            us(*lat.last().unwrap_or(&0)),
            host.applied.load(Ordering::Relaxed)
        );
    }
}
