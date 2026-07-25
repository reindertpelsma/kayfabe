//! ★ CONCURRENCY STRESS (decision #17): the multi-vCPU reality, simulated hard.
//!
//! The core WILL be invoked concurrently from multiple vCPUs — different guest
//! processes ringing doorbells / allocating / mapping simultaneously. Safe Rust +
//! `#![forbid(unsafe_code)]` already make a core data race *unrepresentable*
//! (compile-time-asserted `Send + Sync`, see `kayfabe_util::assert_send_sync!` at the
//! bottom of every logic crate); what these tests prove is the other half of the
//! contract: **under a realistic caller-provided synchronization strategy, the
//! core's invariants hold across hundreds of thousands of interleaved ops** — no
//! panic, no corruption, no cross-proc leak, completion conservation, consistent
//! final state.
//!
//! The modeled strategy is the realistic one from the contract (`kayfabe-core` crate
//! docs): a **device-global `RwLock`** for RmGraph/routing/delivery mutation +
//! concurrent lock-free reads, and **split per-`Proc` `&mut` borrows** for the
//! per-proc planes (the #14-isolation-as-parallelism payoff).
//!
//! ## Lock ordering (the harness's, deadlock-freedom by construction)
//!
//! The harness owns four locks. Their acquisition order is total and one-way:
//!
//! ```text
//!   gpu (RwLock)  →  recorder (Mutex, also taken by mocks UNDER gpu)  →  counters
//! ```
//!
//! Rules the code below obeys (and any edit must keep obeying):
//!
//! 1. **Never acquire `gpu` while holding `recorder` or a counter lock.** The mock
//!    isolates lock `recorder` *while the caller holds `gpu` for writing* (every
//!    recorded verb), so taking `gpu` under `recorder` would be a classic
//!    inverted-order deadlock.
//! 2. **Every guard is dropped before the next lock in a checkpoint** — drains use
//!    explicit scopes, no `lock()` temporaries spanning further acquisitions.
//! 3. **No lock is held across `Barrier::wait` or a thread join.**
//!
//! ## Bounded termination (the M4b lesson)
//!
//! A prior draft of this suite sized the soak at 150k ops/thread (2.4M total).
//! That was NOT a deadlock — all 16 workers kept making progress — but a debug
//! build pushing 2.4M acquisitions through one contended `RwLock` (plus ~7% full
//! RmGraph re-projections, serialized) needs minutes, which reads as a hang under
//! any sane CI timeout. The rules now: **iteration counts are fixed and sized to
//! seconds** (measured: the soak ≈ 15 s debug on a 16-way box), every thread is
//! joined, and every test arms a [`watchdog`] that aborts the process loudly if a
//! regression ever makes it wedge — a future hang fails fast instead of eating the
//! suite's timeout.
//!
//! ## Race detection (CI ceiling)
//!
//! The suite runs green on stable (the floor). For runtime race detection, run it
//! under ThreadSanitizer on nightly (requires `rust-src`):
//!
//! ```text
//! KAYFABE_SLOW=1 KAYFABE_STRESS_WATCHDOG_SECS=900 RUSTFLAGS="-Zsanitizer=thread" \
//!     cargo +nightly test -p kayfabe-tests --test concurrency_stress \
//!     -Zbuild-std --target x86_64-unknown-linux-gnu -- --test-threads=1
//! ```
//!
//! (`KAYFABE_SLOW=1` because the big soak is gated — a TSan run must include it.
//! CI runs this nightly: the `tsan` job in `.github/workflows/ci.yml`.)
//!
//! Expected result: zero TSan reports (the core has no atomics, no lock-free paths,
//! no interior mutability — hence no `loom` tests either; if a lock-free path is
//! ever introduced, add a `loom` model for it alongside this file).

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use kayfabe_arch::ids::GpuId;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant as WallInstant};

use kayfabe_arch::ids::{GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_completion::OsEventRef;
use kayfabe_core::ProcId;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, Proc};
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_fwd::{gate_working_set, handle_doorbell, publish_backing, resolve};
use kayfabe_mocks::{MockArch, MockIsolateFactory, RmVerb, SharedRecorder};
use kayfabe_tests::{Scenario, identical_handles};
use kayfabe_util::Instant;

/// Simulated vCPU threads hammering one shared device.
const N_THREADS: usize = 16;
/// Ops per thread in the big soak (total = `N_THREADS * OPS_PER_THREAD` = 160k).
/// Sized to seconds, not minutes (see "Bounded termination" above): ≈15 s debug.
const OPS_PER_THREAD: usize = 10_000;
/// Guest processes sharing the device (each the #14 shape: identical handles).
const N_PROCS: usize = 8;

/// Arm a watchdog for one test: if the returned guard is not dropped (test body
/// completed OR panicked and unwound) within `limit`, abort the whole process with
/// a loud message. This converts any future wedge in this suite into a fast, named
/// failure instead of an opaque CI timeout. The watchdog thread is detached and
/// exits quietly once the guard drops.
///
/// `KAYFABE_STRESS_WATCHDOG_SECS` overrides `limit` — sanitizer runs (TSan ≈5–15×
/// slower) must set it so the watchdog measures wedging, not instrumentation tax.
#[must_use]
fn watchdog(test: &'static str, limit: Duration) -> WatchdogGuard {
    let limit = match std::env::var("KAYFABE_STRESS_WATCHDOG_SECS") {
        Ok(s) => Duration::from_secs(s.parse().expect("KAYFABE_STRESS_WATCHDOG_SECS: seconds")),
        Err(_) => limit,
    };
    let done = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&done);
    thread::spawn(move || {
        let deadline = WallInstant::now() + limit;
        while WallInstant::now() < deadline {
            if flag.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(Duration::from_millis(200));
        }
        if !flag.load(Ordering::Relaxed) {
            eprintln!("WATCHDOG: {test} still running after {limit:?} — aborting the process");
            std::process::abort();
        }
    });
    WatchdogGuard(done)
}

/// Disarms its [`watchdog`] on drop (normal completion or unwind).
struct WatchdogGuard(Arc<AtomicBool>);
impl Drop for WatchdogGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Per-proc PDB (distinct — THE memory-boundary identity).
fn pdb_of(i: usize) -> Pdb {
    Pdb(0x3400_0000 + (i as u64) * 0x1_0000)
}
/// Per-proc GR/CE vChids (distinct — THE exec-plane identity).
fn gr_vchid(i: usize) -> VChid {
    VChid(0x100 + i as u16)
}
fn ce_vchid(i: usize) -> VChid {
    VChid(0x200 + i as u16)
}
/// Per-proc memory-object handles (identical values across procs — keyed by
/// `(client, handle)`, so this is deliberately the hostile #14 shape).
const MEM_HANDLES: [HObject; 4] = [
    HObject(0x6000_0000),
    HObject(0x6000_0001),
    HObject(0x6000_0002),
    HObject(0x6000_0003),
];

/// Build the shared device: `n` guest processes with IDENTICAL guest handles,
/// DISTINCT PDBs/vChids, plus per-proc memory objects for RM-map churn.
fn stress_gpu(n: usize) -> (Gpu, SharedRecorder) {
    let arch = Box::new(MockArch::new());
    let (factory, recorder) = MockIsolateFactory::new();
    // Sparse reservations: arenas cost address space, not RAM — be generous.
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");

    let mut s = Scenario::new();
    for i in 0..n {
        let client = HClient(0xA0 + i as u32);
        s.compute_process(
            client,
            pdb_of(i),
            identical_handles(gr_vchid(i).0, ce_vchid(i).0),
        );
        for (k, &h) in MEM_HANDLES.iter().enumerate() {
            // Declared backing, distinct per (proc, object).
            let phys = 0x9_0000_0000 + (i as u64) * 0x1000_0000 + (k as u64) * 0x10_0000;
            s.memory(client, HObject(0x5c00_0001), h, phys);
        }
    }
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies cleanly");
    }
    assert_eq!(gpu.procs.len(), n);
    (gpu, recorder)
}

/// Tiny deterministic per-thread RNG (xorshift64*) — reproducible interleavings
/// *per thread*; the cross-thread schedule is the OS's, which is the point.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// Assert a recorded verb only touches ITS OWN isolate's namespace (the mock mints
/// handles as `(iso+1) << 32 | n` and channel tokens as `(iso+1) << 20 | n`, so any
/// cross-proc leak through the core would be visible right here).
fn assert_verb_in_namespace(iso: u32, verb: &RmVerb) {
    let ns = u64::from(iso) + 1;
    let own = |h: kayfabe_isolate::HostHandle| {
        assert_eq!(
            h.0 >> 32,
            ns,
            "handle {h:?} leaked across isolates (ns {ns})"
        );
    };
    match *verb {
        RmVerb::Alloc { parent, handle, .. } => {
            if parent.0 != 0 {
                own(parent);
            }
            own(handle);
        }
        RmVerb::AllocEngineObject { chan, handle, .. } => {
            own(chan);
            own(handle);
        }
        RmVerb::AllocVaSpace { handle } | RmVerb::AllocSysmem { handle, .. } => own(handle),
        RmVerb::AllocChannel {
            vas,
            handle,
            token,
            engine: _,
        } => {
            own(vas);
            own(handle);
            assert_eq!(token >> 20, ns, "host token leaked across isolates");
        }
        RmVerb::Schedule { chan } => own(chan),
        RmVerb::MapGpuVa { vas, memory, .. } => {
            own(vas);
            own(memory);
        }
        RmVerb::UnmapGpuVa { vas, .. } => own(vas),
        RmVerb::RingDoorbell { token } => {
            assert_eq!(token >> 20, ns, "doorbell token leaked across isolates");
        }
        RmVerb::Free { obj } => own(obj),
        RmVerb::Control { obj, .. } => own(obj),
        RmVerb::ExportSurface { memory, surface } => {
            own(memory);
            assert_eq!(surface.0 >> 32, ns, "surface token leaked across isolates");
        }
    }
}

/// Full-device consistency sweep: everything derivable must agree, all per-proc
/// state must be disjoint. Called under exclusive access at checkpoints + at the end.
fn assert_device_consistent(gpu: &Gpu) {
    // Arenas pairwise disjoint (system included) — the #14 collision class.
    let mut ranges: Vec<_> = gpu
        .procs
        .values()
        .map(|p| p.arenas[&GpuId::ZERO].range.clone())
        .collect();
    ranges.push(gpu.system.arenas[&GpuId::ZERO].range.clone());
    for (i, a) in ranges.iter().enumerate() {
        for b in ranges.iter().skip(i + 1) {
            assert!(
                a.end <= b.start || b.end <= a.start,
                "arena overlap: {a:?} vs {b:?}"
            );
        }
    }
    // Routing maps agree with the procs they point at.
    for (&(gpu_t, pdb), &pid) in &gpu.spine.by_pdb {
        let p = gpu.procs.get(&pid).expect("by_pdb points at a live proc");
        assert!(
            p.vases.contains_key(&(gpu_t, pdb)),
            "by_pdb[{pdb:?}] → proc without that Vas"
        );
    }
    for (&(_gpu_t, vchid), &(pid, cid)) in &gpu.spine.by_vchid {
        let p = gpu.procs.get(&pid).expect("by_vchid points at a live proc");
        let c = p
            .channels
            .get(&cid)
            .expect("by_vchid points at a live channel");
        assert_eq!(
            c.vchid, vchid,
            "channel's vChid disagrees with its routing key"
        );
    }
    for p in gpu.procs.values() {
        // Exec state refers only to live channels.
        for cid in &p.exec.scheduled {
            assert!(
                p.channels.contains_key(cid),
                "scheduled ghost channel {cid:?}"
            );
        }
        // Isolate session == ProcId by construction.
        assert_eq!(
            p.isolates[&GpuId::ZERO].id().0,
            p.id.0,
            "isolate session != ProcId"
        );
        for (&(gpu_t, pdb), vas) in &p.vases {
            assert_eq!(vas.pdb, pdb);
            assert_eq!(vas.gpu, gpu_t);
            // Host-published bindings landed in THIS proc's arena; every RPC-bound
            // VA is genuinely in the table.
            for (va, len, b) in vas.table.iter() {
                assert!(len > 0);
                if b.host_va.is_some() {
                    assert!(
                        p.arenas[&GpuId::ZERO].range.contains(&b.phys),
                        "published phys {:#x} outside {:?}'s arena {:?}",
                        b.phys,
                        p.id,
                        p.arenas[&GpuId::ZERO].range
                    );
                }
                let _ = va;
            }
            for &va in &vas.rpc_bound {
                assert!(
                    vas.table.resolve(pdb, GpuVa(va)).is_ok(),
                    "rpc_bound VA {va:#x} missing from the table"
                );
            }
        }
    }
}

/// ★ The soak: `N_THREADS` simulated vCPUs drive one shared `Gpu` through the
/// intended synchronization (device-global `RwLock`) with a realistic mixed op
/// stream — doorbells, backing publications, RM map/unmap churn, completion
/// observe/pump/poll/drain, and lock-free concurrent reads — for 160k interleaved
/// ops (bounded; ≈15 s debug). Invariants are asserted continuously (reads), at
/// periodic verb-log drains (namespace containment), and in a final full sweep
/// (consistency + completion conservation).
#[test]
fn stress_multi_vcpu_interleaved_ops() {
    // The one long test in this file (measured ≈20 s debug; the other three are
    // ≈1 s each) — gated so the every-push fast path stays fast. The nightly
    // `slow` CI job and every TSan run set KAYFABE_SLOW=1 (a TSan pass that
    // skipped THIS test would be the race ceiling in name only).
    kayfabe_tests::skip_slow!("stress_multi_vcpu_interleaved_ops");
    let _wd = watchdog(
        "stress_multi_vcpu_interleaved_ops",
        Duration::from_secs(120),
    );
    let (gpu, recorder) = stress_gpu(N_PROCS);
    let pids: Vec<ProcId> = gpu.procs.keys().copied().collect();
    let gpu = RwLock::new(gpu);
    // Merged across threads at the end: every event observed / every event delivered.
    let observed = Mutex::new(BTreeSet::<OsEventRef>::new());
    let delivered = Mutex::new(BTreeSet::<OsEventRef>::new());
    let verbs_checked = Mutex::new(0usize);

    thread::scope(|s| {
        for tid in 0..N_THREADS {
            let gpu = &gpu;
            let recorder = &recorder;
            let pids = &pids;
            let observed = &observed;
            let delivered = &delivered;
            let verbs_checked = &verbs_checked;
            s.spawn(move || {
                let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ ((tid as u64 + 1) << 32));
                // Per-(thread, proc) local state: publish lanes + RM-map lanes are
                // thread-disjoint so concurrent mutation exercises the CORE, not
                // artificial VA collisions (which are separately tested as loud).
                let mut publish_cnt = [0u64; N_PROCS];
                let mut last_pub: [Option<(GpuVa, u64, u64)>; N_PROCS] = [None; N_PROCS];
                let mut rm_mapped = [false; N_PROCS];
                let mut ev_seq = 0u64;
                let mut local_observed = BTreeSet::new();
                let mut local_delivered = BTreeSet::new();

                for op in 0..OPS_PER_THREAD {
                    let i = (rng.next() % N_PROCS as u64) as usize;
                    let pid = pids[i];
                    let pdb = pdb_of(i);
                    match rng.next() % 100 {
                        // ---- concurrent lock-free reads (&Gpu) --------------------
                        0..=34 => {
                            let g = gpu.read().unwrap();
                            if let Some((va, gpa, host_va)) = last_pub[i] {
                                let (b, off) = resolve(&g, GpuId::ZERO, pdb, GpuVa(va.0 + 0x40))
                                    .expect("published VA resolves");
                                assert_eq!(off, 0x40);
                                assert_eq!(b.phys, gpa, "resolve returned another proc's backing");
                                assert_eq!(b.host_va, Some(host_va));
                                gate_working_set(
                                    &g,
                                    pid,
                                    g.spine.by_vchid[&(GpuId::ZERO, gr_vchid(i))].1,
                                    &[va],
                                )
                                .expect("published working set passes the ring-gate");
                            } else {
                                // Routing reads are always available.
                                assert_eq!(g.spine.by_pdb[&(GpuId::ZERO, pdb)], pid);
                                assert_eq!(g.spine.by_vchid[&(GpuId::ZERO, gr_vchid(i))].0, pid);
                            }
                        }
                        // ---- doorbells (exec-plane demux) -------------------------
                        35..=64 => {
                            let vchid = if rng.next().is_multiple_of(2) {
                                gr_vchid(i)
                            } else {
                                ce_vchid(i)
                            };
                            let mut g = gpu.write().unwrap();
                            let out = handle_doorbell(
                                &mut g,
                                GpuId::ZERO,
                                MockArch::token_for(vchid),
                                &[],
                            )
                            .expect("doorbell routes");
                            assert_eq!(out.proc, pid, "doorbell demuxed to the wrong proc");
                            assert_eq!(
                                out.host_token >> 20,
                                u64::from(pid.0) + 1,
                                "host token from another proc's isolate"
                            );
                        }
                        // ---- backing publication (address plane, per-proc) --------
                        65..=84 => {
                            let va = GpuVa(
                                0x40_0000_0000
                                    + (tid as u64) * 0x1_0000_0000
                                    + publish_cnt[i] * 0x1000,
                            );
                            let mut g = gpu.write().unwrap();
                            let p = g.procs.get_mut(&pid).expect("live proc");
                            let out =
                                publish_backing(p, GpuId::ZERO, pdb, va, 0x1000).expect("publish");
                            assert!(
                                p.arenas[&GpuId::ZERO].range.contains(&out.gpa),
                                "GPA {:#x} escaped {pid:?}'s arena",
                                out.gpa
                            );
                            publish_cnt[i] += 1;
                            last_pub[i] = Some((va, out.gpa, out.host_va));
                        }
                        // ---- completion plane: observe/pump/poll/drain ------------
                        85..=92 => {
                            let ev = OsEventRef(
                                ((tid as u64 + 1) << 48) | (u64::from(pid.0) << 40) | ev_seq,
                            );
                            ev_seq += 1;
                            let mut g = gpu.write().unwrap();
                            g.procs
                                .get_mut(&pid)
                                .unwrap()
                                .completion
                                .observe(ev)
                                .unwrap();
                            local_observed.insert(ev);
                            let batch = match rng.next() % 3 {
                                0 => g.pump_completions(GpuId::ZERO),
                                1 => g.completion_poll(GpuId::ZERO, pid, Instant(op as u64)),
                                _ => {
                                    g.completions_drained(GpuId::ZERO);
                                    g.pump_completions(GpuId::ZERO)
                                }
                            };
                            if let Some(b) = batch {
                                local_delivered.extend(b.events);
                            }
                        }
                        // ---- RM map/unmap churn through the graph (apply) ---------
                        _ => {
                            let client = HClient(0xA0 + i as u32);
                            let va = GpuVa(0x80_0000_0000 + (tid as u64) * 0x1_0000_0000);
                            let ev = if rm_mapped[i] {
                                RmEvent::Unmap {
                                    client,
                                    vaspace: HObject(0x5c00_0010),
                                    va,
                                }
                            } else {
                                RmEvent::MapMemoryDma {
                                    client,
                                    vaspace: HObject(0x5c00_0010),
                                    memory: MEM_HANDLES[tid % MEM_HANDLES.len()],
                                    va,
                                    offset: 0,
                                    len: 0x2000,
                                }
                            };
                            rm_mapped[i] = !rm_mapped[i];
                            gpu.write().unwrap().apply(ev).expect("map churn applies");
                        }
                    }

                    // Periodic checkpoint: drain the global verb log (bounds memory,
                    // models a consumer) and assert per-isolate namespace containment.
                    // LOCK ORDER (see module docs): each guard is scoped and dropped
                    // before the next acquisition — recorder, then counters, then a
                    // fresh `gpu` read. Never `gpu` under `recorder`.
                    if (op + 1).is_multiple_of(4096) {
                        let drained = {
                            let mut rec = recorder.lock().unwrap();
                            std::mem::take(&mut rec.log)
                        };
                        for (iso, verb) in &drained {
                            assert_verb_in_namespace(iso.0, verb);
                        }
                        *verbs_checked.lock().unwrap() += drained.len();
                        if tid == 0 {
                            assert_device_consistent(&gpu.read().unwrap());
                        }
                    }
                }
                observed.lock().unwrap().append(&mut local_observed);
                delivered.lock().unwrap().append(&mut local_delivered);
            });
        }
    });

    // ---- final sweep under exclusive access -------------------------------------
    let mut gpu = gpu.into_inner().unwrap();
    let mut observed = observed.into_inner().unwrap();
    let mut delivered = delivered.into_inner().unwrap();

    // Flush the completion plane: open the gate, post every straggler, drain.
    // Bounded: each iteration drains what the previous one posted; `pending` only
    // shrinks once the workers are joined, so this exits within two passes.
    loop {
        gpu.completions_drained(GpuId::ZERO);
        match gpu.pump_completions(GpuId::ZERO) {
            Some(b) => delivered.extend(b.events),
            None => break,
        }
    }
    // Conservation: exactly the observed events were delivered — nothing forged,
    // nothing lost (re-posts are idempotent set-wise).
    assert_eq!(
        delivered,
        std::mem::take(&mut observed),
        "completion conservation"
    );
    // Ack everything delivered; every queue must then be provably empty.
    let acked: Vec<OsEventRef> = delivered.into_iter().collect();
    for p in gpu.procs.values_mut() {
        for &e in &acked {
            p.completion.ack(e);
        }
        assert!(
            !p.completion.has_outstanding(),
            "queue not empty after full ack"
        );
    }

    // Full consistency sweep + the remaining verb-log tail.
    assert_device_consistent(&gpu);
    let tail = std::mem::take(&mut recorder.lock().unwrap().log);
    for (iso, verb) in &tail {
        assert_verb_in_namespace(iso.0, verb);
    }
    let total = *verbs_checked.lock().unwrap() + tail.len();
    // The soak was real: at least half the ops issued a host verb (doorbells ring
    // every time; publications alloc+map every time).
    assert!(
        total > N_THREADS * OPS_PER_THREAD / 2,
        "soak was real: {total} host verbs is too few"
    );
}

/// ★ The architectural payoff, proven at the type level AND temporally: two threads
/// mutate DIFFERENT procs **simultaneously with no shared lock** — disjoint `&mut
/// Proc` borrows out of `Gpu::procs` (the borrow checker is the proof of soundness;
/// the mid-work barrier is the proof of temporal overlap: it can only be passed
/// while BOTH threads are inside their per-proc mutation streams).
#[test]
fn per_proc_parallelism_two_procs_no_shared_lock() {
    let _wd = watchdog(
        "per_proc_parallelism_two_procs_no_shared_lock",
        Duration::from_secs(60),
    );
    const PUBLISHES: u64 = 20_000;
    let (mut gpu, _rec) = stress_gpu(2);
    let barrier = Barrier::new(2);

    let mut procs = gpu.procs.values_mut();
    let pa: &mut Proc = procs.next().expect("proc A");
    let pb: &mut Proc = procs.next().expect("proc B");

    let work = |p: &mut Proc, barrier: &Barrier| {
        let (gpu_t, pdb) = *p.vases.keys().next().expect("compute Vas");
        barrier.wait(); // start together
        let mut published = Vec::with_capacity(PUBLISHES as usize);
        for k in 0..PUBLISHES {
            // IDENTICAL guest VAs in both procs — the #14 shape, in parallel.
            let va = GpuVa(0x2_0020_0000 + k * 0x1000);
            let out = publish_backing(p, gpu_t, pdb, va, 0x1000).expect("publish");
            published.push(out);
            if k == PUBLISHES / 2 {
                // Both threads must be mid-mutation for anyone to pass this point.
                barrier.wait();
            }
        }
        published
    };

    let (pub_a, pub_b) = thread::scope(|s| {
        let ta = s.spawn(|| work(pa, &barrier));
        let tb = s.spawn(|| work(pb, &barrier));
        (ta.join().expect("A clean"), tb.join().expect("B clean"))
    });

    // Identical guest VAs → fully disjoint GPAs and host VAs (no cross-talk at all).
    let gpas_a: BTreeSet<u64> = pub_a.iter().map(|p| p.gpa).collect();
    let gpas_b: BTreeSet<u64> = pub_b.iter().map(|p| p.gpa).collect();
    assert_eq!(gpas_a.len() as u64, PUBLISHES, "no GPA reuse within A");
    assert!(gpas_a.is_disjoint(&gpas_b), "GPA overlap across procs");
    let hva_a: BTreeSet<u64> = pub_a.iter().map(|p| p.host_va).collect();
    let hva_b: BTreeSet<u64> = pub_b.iter().map(|p| p.host_va).collect();
    assert!(hva_a.is_disjoint(&hva_b), "host VA overlap across procs");

    // Each proc's host VAS came from its OWN isolate (namespace = ProcId + 1).
    for p in gpu.procs.values() {
        let vas = p.vases.values().next().unwrap();
        let hvas = vas.host_vas.expect("materialized");
        assert_eq!(
            hvas.0 >> 32,
            u64::from(p.id.0) + 1,
            "host VAS from a foreign isolate"
        );
    }
    assert_device_consistent(&gpu);
}

/// SAME-proc ops must serialize — and stay exact under arbitrary interleaving:
/// four threads share ONE proc behind a mutex (the caller-provided exclusivity the
/// contract requires for `&mut`); the arena bump, the address table, and the
/// isolate's host mappings must come out byte-exact, with zero double-allocations.
///
/// (This test caught a real mock bug: `MockRmBackend::map_gpu_va` OR-ed its per-VAS
/// lane over a bump counter and minted duplicate host VAs after 2^16 pages — see
/// `mock_rm_map_gpu_va_stays_distinct_past_65536_pages` in `kayfabe-mocks`.)
#[test]
fn same_proc_interleaving_is_exact() {
    let _wd = watchdog("same_proc_interleaving_is_exact", Duration::from_secs(60));
    const THREADS: u64 = 4;
    const PER_THREAD: u64 = 25_000;
    let (mut gpu, _rec) = stress_gpu(1);
    let pid = *gpu.procs.keys().next().unwrap();
    let proc = gpu
        .procs
        .remove(&pid)
        .expect("take the proc out for direct sharding");
    let (gpu_t, pdb) = *proc.vases.keys().next().unwrap();
    let arena_start = proc.arenas[&GpuId::ZERO].range.start;
    let shared = Mutex::new(proc);

    thread::scope(|s| {
        for tid in 0..THREADS {
            let shared = &shared;
            s.spawn(move || {
                for k in 0..PER_THREAD {
                    let va = GpuVa(0x40_0000_0000 + tid * 0x1_0000_0000 + k * 0x1000);
                    let mut p = shared.lock().unwrap();
                    let out = publish_backing(&mut p, gpu_t, pdb, va, 0x1000).expect("publish");
                    assert!(p.arenas[&GpuId::ZERO].range.contains(&out.gpa));
                }
            });
        }
    });

    let p = shared.into_inner().unwrap();
    let vas = &p.vases[&(GpuId::ZERO, pdb)];
    let total = THREADS * PER_THREAD;
    // The bump allocator is exact: every alloc distinct, cursor advanced exactly.
    let gpas: BTreeSet<u64> = vas.table.iter().map(|(_, _, b)| b.phys).collect();
    assert_eq!(gpas.len() as u64, total, "duplicate GPA under interleaving");
    assert_eq!(*gpas.iter().next().unwrap(), arena_start);
    assert_eq!(
        *gpas.iter().next_back().unwrap(),
        arena_start + (total - 1) * 0x1000
    );
    assert_eq!(
        vas.table.iter().count() as u64,
        total,
        "a binding per publish"
    );
    // Every host VA is distinct and from THIS proc's isolate.
    let hvas: BTreeSet<u64> = vas.table.iter().filter_map(|(_, _, b)| b.host_va).collect();
    assert_eq!(
        hvas.len() as u64,
        total,
        "duplicate host VA under interleaving"
    );
}

/// Concurrent readers share `&Gpu` across threads with NO lock at all — the direct
/// runtime witness of `Gpu: Sync` (and, by moving the device into and out of the
/// worker scope, of `Gpu: Send`): a million lock-free resolves + ring-gates racing
/// each other, all agreeing with the pre-published truth.
#[test]
fn lock_free_concurrent_reads_share_the_gpu() {
    let _wd = watchdog(
        "lock_free_concurrent_reads_share_the_gpu",
        Duration::from_secs(60),
    );
    const READERS: usize = 8;
    const READS: usize = 125_000;
    let (mut gpu, _rec) = stress_gpu(4);
    let pids: Vec<ProcId> = gpu.procs.keys().copied().collect();

    // Publish a truth set: 64 VAs per proc.
    let mut truth: BTreeMap<(usize, u64), (u64, u64)> = BTreeMap::new();
    for (i, &pid) in pids.iter().enumerate() {
        for k in 0..64u64 {
            let va = 0x2_0020_0000 + k * 0x1000;
            let p = gpu.procs.get_mut(&pid).unwrap();
            let out =
                publish_backing(p, GpuId::ZERO, pdb_of(i), GpuVa(va), 0x1000).expect("publish");
            truth.insert((i, va), (out.gpa, out.host_va));
        }
    }

    // `Gpu: Send` witness — move it to a worker thread and back.
    let gpu = thread::spawn(move || gpu)
        .join()
        .expect("Gpu crosses threads");

    let gpu_ref = &gpu;
    let truth = &truth;
    let pids = &pids;
    thread::scope(|s| {
        for tid in 0..READERS {
            s.spawn(move || {
                let mut rng = Rng(0xDEAD_BEEF ^ ((tid as u64 + 1) << 40));
                for _ in 0..READS {
                    let i = (rng.next() % 4) as usize;
                    let k = rng.next() % 64;
                    let va = 0x2_0020_0000 + k * 0x1000;
                    let (gpa, host_va) = truth[&(i, va)];
                    let (b, off) =
                        resolve(gpu_ref, GpuId::ZERO, pdb_of(i), GpuVa(va + 0x10)).expect("hit");
                    assert_eq!((b.phys, b.host_va, off), (gpa, Some(host_va), 0x10));
                    let (pid, cid) = gpu_ref.spine.by_vchid[&(GpuId::ZERO, gr_vchid(i))];
                    assert_eq!(pid, pids[i]);
                    gate_working_set(gpu_ref, pid, cid, &[GpuVa(va)]).expect("gate passes");
                    // A miss is still a loud fault under concurrency, never a guess.
                    assert!(
                        resolve(gpu_ref, GpuId::ZERO, pdb_of(i), GpuVa(0xdead_0000_0000)).is_err()
                    );
                }
            });
        }
    });
    assert_device_consistent(&gpu);
}
