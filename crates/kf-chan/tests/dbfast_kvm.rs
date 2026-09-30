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

use kf_chan::dbfast::{DbFast, Delivered, Ioeventfd, RegOutcome, Sink};
use kf_chip::hwref::DieGroup;
use kf_core::{HostOps, HostSlice, Owner, Plane, Step, Translatable, VmCaps, Vmm};
use kf_linux_raw::{
    GuestWindow, HostOffset, HostPageSize, Kvm, KvmMemslot, KvmVcpu, KvmVm, Notifier, PollTimeout,
    RawError, ReadyTokens, VcpuExit,
};
use kf_trap::tokenindex::{GuestTokenFormat, TokenIndex};
use kf_trap::{Action, Class, Route};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::os::fd::BorrowedFd;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// BAR0 offset of the doorbell (the usermode page `0xBB0000` + `NOTIFY_CHANNEL_PENDING` 0x90) —
/// what the device hands `trap_write`; the harness's guest-physical doorbell is elsewhere.
const DB_OFF: u32 = 0x00BB_0090;
/// The drainer's own wake tag in the fast path's poller.
const KICK_TAG: u64 = 1;

/// The real kernel as the fast path's KVM verb.
struct KvmVerb(Arc<KvmVm>);
impl Ioeventfd for KvmVerb {
    fn set(&self, gpa: u64, value: u32, fd: BorrowedFd<'_>, assign: bool) -> Result<(), i32> {
        self.0
            .ioeventfd(gpa, value, fd, assign)
            .map_err(|e| match e {
                RawError::Syscall { errno: Some(e), .. } => e,
                _ => -1,
            })
    }
}

/// A guest: code in RAM at one page, the doorbell page read-only at the next.
struct Guest {
    kvm: Kvm,
    vm: Arc<KvmVm>,
    code_gpa: u64,
    doorbell: u64,
    _slots: Vec<(Arc<GuestWindow>, KvmMemslot)>,
}

impl Guest {
    /// Code at page 1 (up to 4 pages), the doorbell page read-only at page 8; `image` is given
    /// the doorbell's guest-physical address.
    fn new(image: impl FnOnce(u64) -> Vec<u8>) -> Guest {
        let page = HostPageSize::query();
        let kvm = Kvm::open().expect("/dev/kvm");
        let vm = Arc::new(kvm.create_vm().expect("vm"));
        vm.set_tss_addr_if_supported().expect("tss");
        let code_gpa = page.bytes();
        let db_gpa = 8 * page.bytes();
        let doorbell = db_gpa + 0x90;
        let code = image(doorbell);
        let code_len = 4 * page.bytes();
        assert!(code.len() as u64 <= code_len, "guest image too long");
        let cw = Arc::new(GuestWindow::create(code_len, page).expect("code"));
        cw.write_from(HostOffset::ZERO, &code).expect("image");
        let cs = KvmMemslot::install(
            Arc::clone(&vm),
            0,
            code_gpa,
            Arc::clone(&cw),
            0,
            code_len,
            false,
        )
        .expect("code slot");
        let dw = Arc::new(GuestWindow::create(page.bytes(), page).expect("db"));
        let ds = KvmMemslot::install(
            Arc::clone(&vm),
            1,
            db_gpa,
            Arc::clone(&dw),
            0,
            page.bytes(),
            true,
        )
        .expect("read-only doorbell slot");
        Guest {
            kvm,
            vm,
            code_gpa,
            doorbell,
            _slots: vec![(cw, cs), (dw, ds)],
        }
    }

    fn vcpu(&self) -> KvmVcpu {
        let v = KvmVcpu::create(&self.kvm, Arc::clone(&self.vm), 0).expect("vcpu");
        v.enter_flat_protected_mode(self.code_gpa, 0).expect("pm");
        v
    }

    /// Run until HLT; every doorbell exit goes to `trap` with its value.
    fn run(&self, vcpu: &mut KvmVcpu, mut trap: impl FnMut(u32)) {
        loop {
            match vcpu.run().expect("run") {
                VcpuExit::Halted => return,
                VcpuExit::Interrupted => {}
                VcpuExit::Mmio {
                    gpa,
                    len: 4,
                    is_write: true,
                    data,
                } if gpa == self.doorbell => {
                    trap(u32::from_le_bytes([data[0], data[1], data[2], data[3]]));
                }
                other => panic!("unexpected exit {other:?}"),
            }
        }
    }
}

/// `mov dword [db], v` per token, then HLT.
fn stores(db: u64, tokens: &[u32]) -> Vec<u8> {
    let d = u32::try_from(db).expect("below 4 GiB").to_le_bytes();
    let mut v = Vec::new();
    for t in tokens {
        let t = t.to_le_bytes();
        v.extend_from_slice(&[0xC7, 0x05, d[0], d[1], d[2], d[3], t[0], t[1], t[2], t[3]]);
    }
    v.push(0xF4);
    v
}

/// `mov ecx, n; loop: (mov dword [db], v)*; dec ecx; jnz loop; hlt`.
fn loop_stores(db: u64, tokens: &[u32], n: u32) -> Vec<u8> {
    let d = u32::try_from(db).expect("below 4 GiB").to_le_bytes();
    let mut v = vec![0xB9];
    v.extend_from_slice(&n.to_le_bytes());
    let top = v.len();
    for t in tokens {
        let t = t.to_le_bytes();
        v.extend_from_slice(&[0xC7, 0x05, d[0], d[1], d[2], d[3], t[0], t[1], t[2], t[3]]);
    }
    v.push(0x49); // dec ecx
    let back = v.len() + 2 - top;
    assert!(back <= 128, "loop body too long for a rel8");
    v.push(0x75); // jnz rel8
    #[allow(clippy::cast_possible_truncation)]
    v.push((256 - back) as u8);
    v.push(0xF4);
    v
}

/// Host-side stand-in for the twins and rings: which host tokens are live, and every ring/serve.
#[derive(Default)]
struct Host {
    freed: Mutex<BTreeSet<u32>>,
    serves: Mutex<Vec<u32>>,
}
impl HostOps for Host {
    fn ring_host(&self, _: u32) {}
    fn run_translated(&self, host_token: u32, _: u64) -> bool {
        self.serves.lock().unwrap().push(host_token);
        true
    }
    fn run_emulated(&self, _: u32, _: u64) {}
    fn apply_register(&self, _: u8, _: u32, _: u64, _: u8) {}
    fn operands_translatable(&self, _: u32, _: u64) -> Translatable {
        Translatable::Yes
    }
    fn forge_completion(&self, _: u32) {}
    fn refuse_and_poison(&self, _: u32) {}
    fn fault_channel(&self, _: u32) {}
    fn map_guest_slice(&self, _: HostSlice) {}
    fn teardown_step(&self, _: Step) {}
}

/// ★ The device's delivery, as `kf-qemu` implements it: the trap's own doorbell arm, then the act
/// it asks for. Records every ring by path and registration.
struct DevSink {
    plane: &'static Plane<'static>,
    host: Arc<Host>,
    worker: Notifier,
    /// (registration tag, host token) of every fast-path ring.
    fast_rings: Mutex<Vec<(u64, u32)>>,
    /// Fast-path rings of a host token already freed — must stay 0.
    fast_late: AtomicU64,
    /// Trap-path rings of a host token already freed (the trap's own load→ring window; reported).
    trap_late: AtomicU64,
    /// Trap exits handled, per value.
    trapped: Mutex<HashMap<u32, u64>>,
}

impl DevSink {
    fn new(plane: &'static Plane<'static>, host: Arc<Host>) -> DevSink {
        DevSink {
            plane,
            host,
            worker: Notifier::create().expect("worker efd"),
            fast_rings: Mutex::new(Vec::new()),
            fast_late: AtomicU64::new(0),
            trap_late: AtomicU64::new(0),
            trapped: Mutex::new(HashMap::new()),
        }
    }

    /// What the device does with a doorbell of `value`, on whichever thread holds it.
    fn act(&self, value: u32) -> (Action, Delivered) {
        let route = self
            .plane
            .token_index
            .of_doorbell(value)
            .and_then(|i| self.plane.tokens.get(i as usize))
            .map(|w| w.load().route);
        let a = self
            .plane
            .trap_write(Class::Doorbell, 0, DB_OFF, u64::from(value), 4);
        let d = match a {
            Action::RingHostInline { .. } => Delivered::Rang { reached: true },
            Action::WakeWorker => {
                let _ = self.worker.signal();
                Delivered::Handed
            }
            _ if matches!(route, Some(Route::Translated | Route::Emulated)) => Delivered::Handed,
            _ => Delivered::Absorbed,
        };
        (a, d)
    }

    /// The trap path (a vCPU exit).
    fn trap(&self, value: u32) {
        *self.trapped.lock().unwrap().entry(value).or_default() += 1;
        if let (Action::RingHostInline { host_token }, _) = self.act(value)
            && self.host.freed.lock().unwrap().contains(&host_token)
        {
            self.trap_late.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Sink for DevSink {
    fn deliver(&self, _idx: u32, value: u32, tag: u64) -> Delivered {
        let (a, d) = self.act(value);
        if let Action::RingHostInline { host_token } = a {
            if self.host.freed.lock().unwrap().contains(&host_token) {
                self.fast_late.fetch_add(1, Ordering::Relaxed);
            }
            self.fast_rings.lock().unwrap().push((tag, host_token));
        }
        d
    }
}

fn plane() -> &'static Plane<'static> {
    let vmm: &'static Vmm = Box::leak(Box::new(Vmm::new()));
    Box::leak(Box::new(Plane::for_device(
        vmm,
        1 << 12,
        0xFFF,
        kf_chip::Family::Ampere,
        16 << 20,
    )))
}

/// The drainer: waits on the fast path's poller (its kick at [`KICK_TAG`]) and delivers.
struct Drainer {
    stop: Arc<AtomicBool>,
    h: Option<std::thread::JoinHandle<()>>,
}
impl Drainer {
    fn start(f: &Arc<DbFast>, kick: &'static Notifier, sink: &Arc<DevSink>) -> Drainer {
        f.poller()
            .watch(kick.as_source_fd(), KICK_TAG)
            .expect("watch kick");
        let stop = Arc::new(AtomicBool::new(false));
        let (f, sink, st) = (Arc::clone(f), Arc::clone(sink), Arc::clone(&stop));
        let h = std::thread::spawn(move || {
            while !st.load(Ordering::Acquire) {
                let mut r = ReadyTokens::new();
                let _ = f.poller().wait(&mut r, PollTimeout::Millis(5));
                let t = Instant::now();
                if r.iter().any(|t| t == KICK_TAG) {
                    let _ = kick.drain();
                }
                f.on_ready(&r, t, &*sink);
            }
        });
        Drainer { stop, h: Some(h) }
    }
}
impl Drop for Drainer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.h.take() {
            let _ = h.join();
        }
    }
}

fn kick() -> &'static Notifier {
    Box::leak(Box::new(Notifier::create().expect("kick")))
}

fn caps() -> Mutex<VmCaps> {
    Mutex::new(VmCaps::from_declared(4096, 64, 64, 64))
}

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
        if idx == ia {
            0xA000 + generation
        } else {
            0xB000 + generation
        }
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
