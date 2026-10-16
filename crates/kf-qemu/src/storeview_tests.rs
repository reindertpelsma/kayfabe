use super::*;
use kf_linux_raw::{Backing, CachePolicy, HostOffset, HostPageSize, SharedRam, VolatileRegion};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Stub {
    arms: AtomicU64,
    releases: AtomicU64,
    fail_arms: AtomicUsize,
    fail_release: AtomicBool,
    partial_arms: AtomicUsize,
}
struct View {
    region: Option<VolatileRegion>,
    _fd: SharedRam,
    rm_live: bool,
}
fn pattern(off: u64) -> u8 {
    (off ^ (off >> 8) ^ (off >> 17)).wrapping_mul(37) as u8
}
impl Backend for Arc<Stub> {
    type View = View;
    fn arm(&self, off: u64, len: u64) -> Result<View, ArmError<View>> {
        let failures = self.fail_arms.load(Ordering::Relaxed);
        if failures != 0
            && self
                .fail_arms
                .compare_exchange(failures, failures - 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            return Err(ArmError { retained: None });
        }
        let fd = SharedRam::create_named(c"kf-storeview-stub", len).unwrap();
        let region = VolatileRegion::map(
            Backing::SharedFile {
                fd: fd.as_backing_fd(),
                offset: 0,
            },
            len,
            CachePolicy::WriteBack,
            HostPageSize::query(),
        )
        .unwrap();
        // Immutable pattern: no concurrent CPU write overlaps a read.
        for at in (0..len).step_by(8) {
            let mut b = [0; 8];
            for (i, x) in b.iter_mut().enumerate() {
                *x = pattern(off + at + i as u64);
            }
            region
                .store_u64(HostOffset::new(at), u64::from_le_bytes(b))
                .unwrap();
        }
        self.arms.fetch_add(1, Ordering::Relaxed);
        let mut v = View {
            region: Some(region),
            _fd: fd,
            rm_live: true,
        };
        if self.partial_arms.swap(0, Ordering::Relaxed) != 0 {
            v.region.take();
            return Err(ArmError { retained: Some(v) });
        }
        Ok(v)
    }
    fn release(&self, v: &mut View) -> Result<(), Error> {
        v.region.take();
        if self.fail_release.load(Ordering::Relaxed) {
            return Err(Error::Release);
        }
        if v.rm_live {
            self.releases.fetch_add(1, Ordering::Relaxed);
            v.rm_live = false;
        }
        Ok(())
    }
    fn read(&self, v: &View, off: u64, out: &mut [u8]) -> Result<(), Error> {
        v.region
            .as_ref()
            .ok_or(Error::Read)?
            .copy_out(HostOffset::new(off), out)
            .map_err(|_| Error::Read)
    }
    fn write(&self, v: &mut View, off: u64, bytes: &[u8]) -> Result<(), Error> {
        // The production API must handle arbitrary byte writes; the test backend
        // only needs a checked aligned word for a nonconcurrent write test.
        let b: [u8; 8] = bytes.try_into().map_err(|_| Error::Write)?;
        v.region
            .as_ref()
            .ok_or(Error::Write)?
            .store_u64(HostOffset::new(off), u64::from_le_bytes(b))
            .map_err(|_| Error::Write)
    }
}
fn ready(t: &Windowed<Arc<Stub>>, owner: &Arc<Owner>, off: u64) {
    for _ in 0..20 {
        match t.prepare(owner, off) {
            Ok(()) => return,
            Err(Error::Retry) => (),
            e => panic!("prepare {e:?}"),
        }
    }
    panic!("did not prepare");
}
fn verify(t: &Windowed<Arc<Stub>>, off: u64, len: usize) -> Result<(), Error> {
    let mut out = [0; ACCESS];
    t.read(off, &mut out[..len])?;
    for (i, b) in out[..len].iter().enumerate() {
        assert_eq!(*b, pattern(off + i as u64));
    }
    Ok(())
}
#[test]
fn bounds_crossings_more_than_sixteen_windows_and_balanced_resources() {
    let stub = Arc::new(Stub::default());
    let owner = Arc::new(Owner::default());
    let mut t = Windowed::new(stub.clone(), WINDOW * 64, 64).unwrap();
    for i in 0..32 {
        ready(&t, &owner, i * WINDOW);
        verify(&t, i * WINDOW + 3, 256).unwrap();
    }
    ready(&t, &owner, WINDOW);
    verify(&t, WINDOW - 1, 256).unwrap();
    assert_eq!(t.read(u64::MAX, &mut [0; 1]), Err(Error::Bounds));
    assert_eq!(t.read(0, &mut [0; 257]), Err(Error::Bounds));
    assert_eq!(t.read(WINDOW * 64, &mut []), Ok(()));
    assert_eq!(t.write(WINDOW * 64 - 1, &[0; 8]), Err(Error::Bounds));
    t.write(8, &[0; 8]).unwrap();
    let mut out = [1; 8];
    t.read(8, &mut out).unwrap();
    assert_eq!(out, [0; 8]);
    assert_eq!(t.live(), 32);
    t.release().unwrap();
    assert_eq!(owner.live(), 0);
    assert_eq!(t.live(), 0);
    assert_eq!(
        stub.arms.load(Ordering::Relaxed),
        stub.releases.load(Ordering::Relaxed)
    );
}
#[test]
fn failed_second_span_never_partially_changes_output() {
    let t = Windowed::new(Arc::new(Stub::default()), 2 * WINDOW, 2).unwrap();
    ready(&t, &Arc::new(Owner::default()), 0);
    let mut out = [77; 256];
    assert_eq!(t.read(WINDOW - 8, &mut out), Err(Error::Retry));
    assert_eq!(out, [77; 256]);
}
#[test]
fn stalled_reader_and_exiting_thread_do_not_block_other_slots() {
    let t = Arc::new(Windowed::new(Arc::new(Stub::default()), 4 * WINDOW, 2).unwrap());
    let owner = Arc::new(Owner::default());
    ready(&t, &owner, 0);
    ready(&t, &owner, WINDOW);
    let pin = t.slot(0).unwrap().pin().unwrap();
    assert!(!t.evict(&owner).unwrap()); // clears hot bit on the other span
    assert!(t.evict(&owner).unwrap());
    assert!(t.slot(0).unwrap().claim_idle().is_none());
    let other = t.clone();
    std::thread::spawn(move || {
        let _pin = other.slot(0).unwrap().pin().unwrap();
    })
    .join()
    .unwrap();
    drop(pin);
    assert!(!t.evict(&owner).unwrap());
    assert!(t.evict(&owner).unwrap());
    assert_eq!(t.live(), 0);
}
#[test]
fn hostile_owner_cannot_evict_an_equal_or_smaller_owner() {
    let t = Windowed::new(Arc::new(Stub::default()), 8 * WINDOW, 4).unwrap();
    let victim = Arc::new(Owner::default());
    let hostile = Arc::new(Owner::default());
    ready(&t, &victim, 0);
    for i in 1..4 {
        ready(&t, &hostile, i * WINDOW);
    }
    for i in 4..8 {
        ready(&t, &hostile, i * WINDOW);
        verify(&t, 0, 64).unwrap();
    }
    assert_eq!(victim.live(), 1);
}
#[test]
fn arm_failure_storm_and_release_failure_retain_ownership() {
    let stub = Arc::new(Stub::default());
    let owner = Arc::new(Owner::default());
    let mut t = Windowed::new(stub.clone(), 4 * WINDOW, 4).unwrap();
    stub.fail_arms.store(1000, Ordering::Relaxed);
    for _ in 0..100 {
        assert_eq!(t.prepare(&owner, 0), Err(Error::Arm));
    }
    assert_eq!(t.live(), 0);
    assert_eq!(owner.live(), 0);
    stub.fail_arms.store(0, Ordering::Relaxed);
    ready(&t, &owner, 0);
    stub.fail_release.store(true, Ordering::Relaxed);
    assert!(!t.evict(&owner).unwrap());
    assert_eq!(t.evict(&owner), Err(Error::Release));
    assert_eq!(verify(&t, 0, 64), Err(Error::Retry));
    assert_eq!(owner.live(), 1);
    stub.fail_release.store(false, Ordering::Relaxed);
    t.release().unwrap();
    assert_eq!(owner.live(), 0);
    assert_eq!(
        stub.arms.load(Ordering::Relaxed),
        stub.releases.load(Ordering::Relaxed)
    );
}

#[test]
fn partial_arm_owns_a_charge_until_cleanup_succeeds() {
    let stub = Arc::new(Stub::default());
    let owner = Arc::new(Owner::default());
    let mut t = Windowed::new(stub.clone(), 4 * WINDOW, 4).unwrap();
    stub.partial_arms.store(1, Ordering::Relaxed);
    assert_eq!(t.prepare(&owner, 0), Err(Error::Arm));
    assert_eq!(t.live(), 1);
    assert_eq!(owner.live(), 1);
    assert_eq!(verify(&t, 0, 64), Err(Error::Retry));
    assert!(t.evict(&owner).unwrap());
    ready(&t, &owner, 0);
    t.release().unwrap();
    assert_eq!(owner.live(), 0);
    assert_eq!(
        stub.arms.load(Ordering::Relaxed),
        stub.releases.load(Ordering::Relaxed)
    );
}
#[test]
fn failed_eviction_during_arm_retry_returns_its_uncommitted_charge() {
    let stub = Arc::new(Stub::default());
    let owner = Arc::new(Owner::default());
    let mut t = Windowed::new(stub.clone(), 4 * WINDOW, 4).unwrap();
    ready(&t, &owner, 0);
    assert!(!t.evict(&owner).unwrap());
    stub.fail_release.store(true, Ordering::Relaxed);
    stub.fail_arms.store(1, Ordering::Relaxed);
    assert_eq!(t.prepare(&owner, WINDOW), Err(Error::Release));
    assert_eq!(t.live(), 1);
    assert_eq!(owner.live(), 1);
    stub.fail_release.store(false, Ordering::Relaxed);
    t.release().unwrap();
    assert_eq!(t.live(), 0);
    assert_eq!(owner.live(), 0);
}

#[test]
fn full_view_bounds_and_release() {
    let stub = Arc::new(Stub::default());
    let mut f = Full::new(stub.clone(), WINDOW).ok().unwrap();
    let mut b = [0; ACCESS];
    f.read(17, &mut b).unwrap();
    for (i, x) in b.iter().enumerate() {
        assert_eq!(*x, pattern(17 + i as u64));
    }
    assert_eq!(f.read(WINDOW - 1, &mut b), Err(Error::Bounds));
    f.release().unwrap();
    assert_eq!(
        stub.arms.load(Ordering::Relaxed),
        stub.releases.load(Ordering::Relaxed)
    );
}
#[test]
fn module_has_no_waiting_primitives() {
    for s in [
        include_str!("storeview.rs"),
        include_str!("storeview_unsafe.rs"),
        include_str!("storeview_native.rs"),
    ] {
        for term in [
            "Mutex",
            "RwLock",
            "thread::sleep",
            "thread::park",
            "Condvar::",
        ] {
            assert!(!s.contains(term), "forbidden {term}");
        }
    }
}
/// Set seconds/threads/seed for the required long runs. Pattern checked on EVERY
/// successful read; Retry is expected during bounded contention, never re-run a failure.
#[test]
#[ignore = "explicit long stress/soak"]
fn concurrent_real_mapping_stress() {
    let secs = std::env::var("KF3_STORE_STRESS_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);
    let threads = std::env::var("KF3_STORE_STRESS_THREADS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let seed: u64 = std::env::var("KF3_STORE_STRESS_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x20261016);
    eprintln!("STORE_STRESS_START seconds={secs} threads={threads} seed={seed}");
    let fd_count = || std::fs::read_dir("/proc/self/fd").unwrap().count();
    let map_count = || {
        std::fs::read_to_string("/proc/self/maps")
            .unwrap()
            .lines()
            .filter(|s| s.contains("kf-storeview-stub"))
            .count()
    };
    let fds_before = fd_count();
    let maps_before = map_count();
    let stub = Arc::new(Stub::default());
    let t = Arc::new(Windowed::new(stub.clone(), 128 * WINDOW, 32).unwrap());
    let until = Instant::now() + Duration::from_secs(secs);
    let mut jobs = Vec::new();
    for i in 0..threads {
        let t = t.clone();
        jobs.push(std::thread::spawn(move || {
            let owner = Arc::new(Owner::default());
            let mut rng = seed.wrapping_add(i as u64 + 1);
            let mut good = 0u64;
            while Instant::now() < until {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                let off = (rng % 128) * WINDOW + ((rng >> 12) % (WINDOW - 256));
                match verify(&t, off, ACCESS) {
                    Ok(()) => good += 1,
                    Err(Error::Retry) => {
                        let _ = t.prepare(&owner, off);
                    }
                    Err(e) => panic!("read {e:?} thread={i} seed={seed}"),
                }
            }
            good
        }));
    }
    let good: u64 = jobs.into_iter().map(|j| j.join().unwrap()).sum();
    let mut t = Arc::try_unwrap(t).ok().unwrap();
    t.release().unwrap();
    assert!(good > 0);
    assert_eq!(t.live(), 0);
    let fds_after = fd_count();
    let maps_after = map_count();
    eprintln!(
        "STORE_RESOURCES fds={fds_before}->{fds_after} owned_maps={maps_before}->{maps_after}"
    );
    assert_eq!(fds_before, fds_after);
    assert_eq!(maps_before, maps_after);
    assert_eq!(
        stub.arms.load(Ordering::Relaxed),
        stub.releases.load(Ordering::Relaxed)
    );
    eprintln!(
        "STORE_STRESS_EXIT good_reads={good} arms={} releases={}",
        stub.arms.load(Ordering::Relaxed),
        stub.releases.load(Ordering::Relaxed)
    );
}

#[test]
fn retired_owner_does_not_reserve_its_last_window_forever() {
    let t = Windowed::new(Arc::new(Stub::default()), 4 * WINDOW, 1).unwrap();
    let dead = Arc::new(Owner::default());
    ready(&t, &dead, 0);
    dead.retire();
    let new = Arc::new(Owner::default());
    ready(&t, &new, WINDOW);
    assert_eq!(dead.live(), 0);
    assert_eq!(new.live(), 1);
}
#[test]
fn concurrent_hostile_evictions_leave_a_live_owners_last_window() {
    let t = Arc::new(Windowed::new(Arc::new(Stub::default()), 16 * WINDOW, 16).unwrap());
    let victim = Arc::new(Owner::default());
    let hostile = Arc::new(Owner::default());
    for i in 0..8 {
        ready(&t, &victim, i * WINDOW);
    }
    let mut jobs = Vec::new();
    for _ in 0..16 {
        let t = t.clone();
        let h = hostile.clone();
        jobs.push(std::thread::spawn(move || {
            for _ in 0..16 {
                t.evict(&h).unwrap();
            }
        }));
    }
    for j in jobs {
        j.join().unwrap();
    }
    assert_eq!(victim.live(), 1);
    assert_eq!(t.live(), 1);
}

#[test]
#[ignore = "explicit read-path scaling report"]
fn read_path_scaling() {
    let secs = std::env::var("KF3_STORE_SCALE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    for shared in [true, false] {
        for threads in [1usize, 2, 4, 8, 16, 32] {
            let t = Arc::new(Windowed::new(Arc::new(Stub::default()), 32 * WINDOW, 32).unwrap());
            let owner = Arc::new(Owner::default());
            for i in 0..threads {
                ready(&t, &owner, i as u64 * WINDOW);
            }
            let barrier = Arc::new(std::sync::Barrier::new(threads + 1));
            let until = Instant::now() + Duration::from_secs(secs);
            let mut jobs = Vec::new();
            for i in 0..threads {
                let t = t.clone();
                let barrier = barrier.clone();
                jobs.push(std::thread::spawn(move || {
                    let off = if shared { 0 } else { i as u64 * WINDOW };
                    let mut good = 0u64;
                    let mut retries = 0u64;
                    let mut samples = Vec::new();
                    let mut out = [0; 64];
                    barrier.wait();
                    while Instant::now() < until {
                        let start = Instant::now();
                        let r = t.read(off, &mut out);
                        let ns = start.elapsed().as_nanos() as u64;
                        match r {
                            Ok(()) => {
                                for (k, b) in out.iter().enumerate() {
                                    assert_eq!(*b, pattern(off + k as u64));
                                }
                                good += 1;
                                if samples.len() < 8192 && good % 32 == 0 {
                                    samples.push(ns);
                                }
                            }
                            Err(Error::Retry) => retries += 1,
                            Err(e) => panic!("scaling read {e:?}"),
                        }
                    }
                    (good, retries, samples)
                }));
            }
            let start = Instant::now();
            barrier.wait();
            let mut good = 0u64;
            let mut retries = 0u64;
            let mut samples = Vec::new();
            for job in jobs {
                let (g, r, s) = job.join().unwrap();
                good += g;
                retries += r;
                samples.extend(s);
            }
            let elapsed = start.elapsed().as_secs_f64();
            samples.sort_unstable();
            assert!(good > 0);
            let p50 = samples[samples.len() / 2];
            let p99 = samples[samples.len() * 99 / 100];
            eprintln!(
                "STORE_SCALE threads={threads} shared_span={shared} bytes=64 seconds={elapsed:.3} good={good} retries={retries} reads_per_s={:.0} p50_ns={p50} p99_ns={p99}; pattern-verified RAM stub, timer and verification overhead included in throughput",
                good as f64 / elapsed
            );
        }
    }
}
