//! ★★★★★ **The doorbell fast path against a REAL KVM guest** (`docs/design/V3_DOORBELL_IOEVENTFD.md`
//! §6). GPU-free: a flat 32-bit guest stores tokens to a doorbell register on a read-only memslot
//! (kf3's usermode page is a ROM device over the host window); the fast path places real
//! `KVM_IOEVENTFD` registrations; every exit is handled by the SAME trap arm the device uses
//! (`kf_core::Plane::trap_write(Class::Doorbell, …)`), and every eventfd delivery goes through it too.
//!
//! What is proved here, each by its own test:
//! 1. **match and fallback** — a registered token's stores never exit and are delivered by the
//!    drainer; every other store (unknown token, a non-canonical value that masks into a live slot,
//!    a token after its removal) exits and is handled by the trap, in order, with its exact value;
//!    Passthrough rings the twin, Translated is handed to its channel (RUNG + worker wake).
//! 2. **churn** — birth / free / re-birth of tokens while the guest rings them flat out on the
//!    vCPU: every single store is delivered EXACTLY once (trap + eventfd counts = guest stores —
//!    the lost-wakeup proof), and no eventfd delivery ever rings a freed twin or another
//!    generation's twin (the generation proof).
//! 3. **exhaustion** (`tests/dbfast_exhaust.rs`, its own process) — the kernel's real refusal.
//!
//! ⊘ Every test here is gated on `/dev/kvm` and prints `KVM-GATE: RAN|SKIPPED <name>`.

mod common;

use common::{DevSink, Drainer, Guest, Host, KvmVerb, caps, kick, loop_stores, plane, stores};
use kf_chan::dbfast::{DbFast, RegOutcome};
use kf_chip::hwref::DieGroup;
use kf_core::{Owner, Plane, VmCaps};
use kf_trap::Route;
use kf_trap::tokenindex::{GuestTokenFormat, TokenIndex};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// ★★★ Match and fallback, exactly: registered tokens are delivered by the drainer with no exit;
/// every other store exits in order with its exact value and is handled by the trap.
#[test]
fn registered_tokens_skip_the_exit_and_everything_else_traps_in_order() {
    kf_linux_raw::require_kvm!(
        "registered_tokens_skip_the_exit_and_everything_else_traps_in_order"
    );
    let fmt = GuestTokenFormat::for_die_group(DieGroup::Ga10x).expect("format");
    let ix = TokenIndex::RunlistVector;
    let (vp, vt) = (fmt.value(0, 1).unwrap(), fmt.value(13, 2).unwrap());
    let unknown = fmt.value(5, 0x33).unwrap();
    let noncanon = vp | 0x8000; // bits 15:12 are not decoded: masks into vp's slot
    assert_eq!(ix.of_doorbell(noncanon), ix.of_doorbell(vp));
    let g = Guest::new(|db| stores(db, &[vp, vt, unknown, noncanon, vt, vp]));

    let plane = plane();
    let c = caps();
    let (ip, it) = (ix.of_channel(0, 1).unwrap(), ix.of_channel(13, 2).unwrap());
    plane
        .allocate_channel(
            &mut c.lock().unwrap(),
            ip,
            Route::Passthrough,
            0xAAA,
            Owner::User,
        )
        .expect("pt");
    plane
        .allocate_channel(
            &mut c.lock().unwrap(),
            it,
            Route::Translated,
            0xBBB,
            Owner::Kernel,
        )
        .expect("tr");
    let host = Arc::new(Host::default());
    let sink = Arc::new(DevSink::new(plane, Arc::clone(&host)));
    let k = kick();
    let f = Arc::new(DbFast::new(64, k).expect("fast path"));
    assert!(f.enable(Box::new(KvmVerb(Arc::clone(&g.vm)))));
    f.site_add(g.doorbell);
    assert!(matches!(
        f.register(ip, vp),
        RegOutcome::Registered { placed: 1, .. }
    ));
    assert!(matches!(
        f.register(it, vt),
        RegOutcome::Registered { placed: 1, .. }
    ));

    let mut vcpu = g.vcpu();
    let mut exits = Vec::new();
    g.run(&mut vcpu, |v| {
        exits.push(v);
        sink.trap(v);
    });
    assert_eq!(
        exits,
        vec![unknown, noncanon],
        "only the unregistered values leave the guest, in order, exactly"
    );
    // The drainer's first poll delivers both registrations: two stores each, ONE delivery each.
    assert_eq!(f.service_ready(&*sink), 2);
    let rings = sink.fast_rings.lock().unwrap().clone();
    assert_eq!(rings.len(), 1, "one host ring covers vp's two stores");
    assert_eq!(rings[0].1, 0xAAA);
    // The translated token is RUNG with its bit published: a worker pass serves its host ring.
    let mut scratch = Vec::new();
    assert_eq!(plane.worker_pass(&*host, &mut scratch, 64), 1);
    assert_eq!(*host.serves.lock().unwrap(), vec![0xBBB]);
    // The non-canonical store trapped and — masking into vp's slot — rang the twin inline.
    assert_eq!(sink.trapped.lock().unwrap().get(&noncanon), Some(&1));

    // Removal: the token's stores trap again.
    let d = Drainer::start(&f, k, &sink);
    let l = f.deregister(ip).expect("acked");
    assert_eq!((l.doorbells, l.forwarded, l.sites), (2, 2, 1));
    let mut exits = Vec::new();
    vcpu.enter_flat_protected_mode(g.code_gpa, 0).expect("pm");
    g.run(&mut vcpu, |v| {
        exits.push(v);
        sink.trap(v);
    });
    assert_eq!(exits, vec![vp, unknown, noncanon, vp]);
    drop(d);
    assert_eq!(f.counters.deassign_failed.load(Ordering::Relaxed), 0);
}

/// ★★★★★ **Birth / free / re-birth while the guest rings flat out** — the lost-wakeup and the
/// generation proofs in one run.
///
/// A vCPU thread runs a guest that stores token A (Passthrough) and token B (Translated) `N` times
/// each, with no pause. A churn thread meanwhile frees and re-births both tokens over and over,
/// in the device's own order: retire the token word → `deregister` (placements removed, final
/// drain, ack) → free the host twin → allocate a NEW host twin → `register` a NEW eventfd. A
/// drainer thread delivers eventfds; the vCPU thread handles exits through the trap arm.
///
/// Asserted: (1) **no store is lost or doubled** — trap exits + every registration's eventfd count
/// equals the guest's `N`, per token, exactly; (2) **no eventfd delivery rings a freed twin**;
/// (3) **every eventfd ring of a registration names that registration's own twin** — never the
/// twin of a later (or earlier) generation.
#[test]
fn churning_births_and_frees_never_lose_a_store_or_ring_the_wrong_twin() {
    kf_linux_raw::require_kvm!(
        "churning_births_and_frees_never_lose_a_store_or_ring_the_wrong_twin"
    );
    const N: u32 = 20_000;
    let fmt = GuestTokenFormat::for_die_group(DieGroup::Ga10x).expect("format");
    let ix = TokenIndex::RunlistVector;
    let (va, vb) = (fmt.value(0, 3).unwrap(), fmt.value(13, 4).unwrap());
    let (ia, ib) = (ix.of_channel(0, 3).unwrap(), ix.of_channel(13, 4).unwrap());
    let g = Arc::new(Guest::new(|db| loop_stores(db, &[va, vb], N)));

    let plane = plane();
    let c = Arc::new(caps());
    let host = Arc::new(Host::default());
    let sink = Arc::new(DevSink::new(plane, Arc::clone(&host)));
    let k = kick();
    let f = Arc::new(DbFast::new(64, k).expect("fast path"));
    f.enable(Box::new(KvmVerb(Arc::clone(&g.vm))));
    f.site_add(g.doorbell);
    let _d = Drainer::start(&f, k, &sink);

    // Generation 0 of both tokens. (Plain fns, so the churn thread can own them.)
    fn host_of(ia: u32, idx: u32, generation: u32) -> u32 {
        // The freed set retains every old twin. The former ranges (0xA000 + g and
        // 0xB000 + g) overlapped after 4096 generations, so a live A twin could look
        // like a freed B twin. Give the two tokens disjoint even/odd identities,
        // and refuse overflow rather than reusing an identity.
        generation
            .checked_mul(2)
            .and_then(|g| g.checked_add(0xA000 + u32::from(idx != ia)))
            .expect("test host-twin identity exhausted")
    }
    fn born(plane: &Plane<'_>, c: &Mutex<VmCaps>, ia: u32, idx: u32, generation: u32) {
        let route = if idx == ia {
            Route::Passthrough
        } else {
            Route::Translated
        };
        plane
            .allocate_channel(
                &mut c.lock().unwrap(),
                idx,
                route,
                host_of(ia, idx, generation),
                Owner::User,
            )
            .expect("allocate");
    }
    born(plane, &c, ia, ia, 0);
    born(plane, &c, ia, ib, 0);
    // tag → the host twin that registration's generation owns.
    let owner: Arc<Mutex<BTreeMap<u64, u32>>> = Arc::new(Mutex::new(BTreeMap::new()));
    for (idx, v) in [(ia, va), (ib, vb)] {
        if let RegOutcome::Registered { tag, .. } = f.register(idx, v) {
            owner.lock().unwrap().insert(tag, host_of(ia, idx, 0));
        }
    }
    let ledgers: Arc<Mutex<HashMap<u32, u64>>> = Arc::new(Mutex::new(HashMap::new()));

    // A worker that serves rung Translated tokens (so B's word cycles RUNG→BUSY→IDLE).
    let stop = Arc::new(AtomicBool::new(false));
    let worker = {
        let (host, stop) = (Arc::clone(&host), Arc::clone(&stop));
        std::thread::spawn(move || {
            let mut scratch = Vec::new();
            while !stop.load(Ordering::Acquire) {
                if plane.worker_pass(&*host, &mut scratch, 64) == 0 {
                    std::thread::sleep(Duration::from_micros(50));
                }
            }
        })
    };
    // The churn thread.
    let done = Arc::new(AtomicBool::new(false));
    let churn = {
        let (f, c, host, owner, ledgers, done) = (
            Arc::clone(&f),
            Arc::clone(&c),
            Arc::clone(&host),
            Arc::clone(&owner),
            Arc::clone(&ledgers),
            Arc::clone(&done),
        );
        std::thread::spawn(move || {
            let mut round_gen = 0u32;
            let mut rounds = 0u32;
            while !done.load(Ordering::Acquire) {
                round_gen += 1;
                for (idx, v) in [(ia, va), (ib, vb)] {
                    // Free: retire the word (waits out a worker's BUSY), then the registration.
                    while !plane.free_channel(&mut c.lock().unwrap(), idx) {
                        std::thread::yield_now();
                    }
                    if let Some(l) = f.deregister(idx) {
                        *ledgers.lock().unwrap().entry(v).or_default() += l.doorbells;
                    }
                    host.freed
                        .lock()
                        .unwrap()
                        .insert(host_of(ia, idx, round_gen - 1));
                    // Re-birth: a new twin, then a new registration.
                    born(plane, &c, ia, idx, round_gen);
                    if let RegOutcome::Registered { tag, .. } = f.register(idx, v) {
                        owner
                            .lock()
                            .unwrap()
                            .insert(tag, host_of(ia, idx, round_gen));
                    }
                }
                rounds += 1;
                std::thread::sleep(Duration::from_micros(u64::from(rounds % 7) * 30));
            }
            rounds
        })
    };

    let mut vcpu = g.vcpu();
    let t0 = Instant::now();
    g.run(&mut vcpu, |v| sink.trap(v));
    let wall = t0.elapsed();
    done.store(true, Ordering::Release);
    let rounds = churn.join().expect("churn");
    // Collect the live registrations' last counts.
    for (idx, v) in [(ia, va), (ib, vb)] {
        if let Some(l) = f.deregister(idx) {
            *ledgers.lock().unwrap().entry(v).or_default() += l.doorbells;
        }
    }
    stop.store(true, Ordering::Release);
    worker.join().expect("worker");

    let trapped = sink.trapped.lock().unwrap().clone();
    let fast = ledgers.lock().unwrap().clone();
    for v in [va, vb] {
        let (t, e) = (
            trapped.get(&v).copied().unwrap_or(0),
            fast.get(&v).copied().unwrap_or(0),
        );
        eprintln!(
            "CHURN value={v:#010x} trapped={t} eventfd={e} total={} of {N}",
            t + e
        );
        assert_eq!(
            t + e,
            u64::from(N),
            "value {v:#x}: every guest store delivered exactly once (trap {t} + eventfd {e})"
        );
    }
    assert!(
        rounds > 3,
        "the churn must actually have churned (rounds {rounds})"
    );
    assert_eq!(
        sink.fast_late.load(Ordering::Relaxed),
        0,
        "no eventfd delivery may ring a freed twin"
    );
    let owner = owner.lock().unwrap();
    for (tag, h) in sink.fast_rings.lock().unwrap().iter() {
        assert_eq!(
            owner.get(tag),
            Some(h),
            "registration {tag:#x} rang host {h:#x}, not its own generation's twin"
        );
    }
    eprintln!(
        "CHURN rounds={rounds} wall={wall:?} fast_rings={} trap_late_rings={} (the trap's own load→ring window) counters: {}",
        sink.fast_rings.lock().unwrap().len(),
        sink.trap_late.load(Ordering::Relaxed),
        f.status()
    );
    assert_eq!(f.counters.deassign_failed.load(Ordering::Relaxed), 0);
    assert_eq!(f.counters.ack_timeouts.load(Ordering::Relaxed), 0);
}
