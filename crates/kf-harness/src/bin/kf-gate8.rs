//! ★★★★★ **v3 gate 8 — the guest's invalidate completes only after the host mapping is committed.**
//! (`V3_P4_PORT_MAP.md` §3 rows 2+3; `THE_CONSTRAINTS.md` §49.1; `THE_TRANSLATED_PLANE.md` §5.)
//!
//! Gates 2 and 7 proved walk → diff → apply → verdict with the harness calling each step by hand.
//! This gate drives the same planes the way the guest will: a **vCPU thread** writes the three
//! `MMU_INVALIDATE` registers into a [`kf_trap::InvalidatePort`] (PDB lo, PDB hi, TRIGGER) and
//! then spin-reads the trigger exactly as `kgmmuCheckPendingInvalidates` does; the **VA-manager
//! thread** (this one) waits in `epoll` on two fds only — the request notifier and the walk
//! kernel's completion eventfd — and runs [`kf_mem::vasmgr::VaManager`]. No QEMU, no guest.
//!
//! Asserted, per format (VER2 = Turing..Ada, VER3 = Hopper/Blackwell — every family first-class):
//! - **(i) ordering**: every host map/unmap/invalidate of the reconcile ran while the guest's
//!   trigger still read BUSY, and the trigger then cleared; a REAL copy engine copying through the
//!   mapped guest VAs, issued at the moment of the clear, completes by EVENT and lands where the
//!   guest's tables say — and again after the guest REMAPS and invalidates a second time;
//! - **(ii) the trap is short**: the worst trigger-write trap on the vCPU thread (arm + publish +
//!   one wake), against the owner's 1 ms ceiling (`kayfabe-device/src/mmuinval.rs`
//!   `INVALIDATE_HOLD_CEILING_US`);
//! - **(iii) supersession**: two triggers back to back — the first completion is `Superseded`,
//!   never a clear; the second clears;
//! - **(iv) named&missed**: an invalidate naming a PDB no object carries is counted and cleared
//!   with no walk;
//! - **(v) `ALL_PDB`**: reconciles every rooted space;
//! - **the walk never blocked its submitter**: zero `cuCtxSynchronize` calls across the run, the
//!   GPU itself (`cuEventQuery`) reports walks still in flight at the instant `submit` returned,
//!   and every completion arrived as a readiness wake on the walker's fd. The submit cost is MEASURED against
//!   the port map's 50 µs budget (a measurement, not a verdict: the launch count is ~25).
//!
//! ⊘ **UNRUN** as committed (w826): written and compiled on a machine with no GPU. Its first run
//! on the box is its first evidence.

use kf_cuda::abi::{KfFormat, kf_format_ver2, kf_format_ver3};
use kf_cuda::walk::{WalkCfg, WalkEntry, WalkKernel};
use kf_harness::tables::{Tree, Tree3};
use kf_harness::publish::Recorded;
use kf_harness::{CeRig, Ledger as Checks};
use kf_host::HostRm;
use kf_linux_raw::{DevDir, Notifier, PollTimeout, Poller, ReadyTokens};
use kf_mem::ledger::{Desired, HostVas, MapTarget, Mapped};
use kf_mem::vasmgr::{GpuWalker, VaManager, VasKey, WalkDone, Walker};
use kf_trap::mmuinval::TRIGGER_BIT;
use kf_trap::{Invalidate, InvalidatePort, InvalidateRegs, InvalidateRequest, PdbAperture, PortWrite};
use std::cell::RefCell;
use std::sync::mpsc;

const STORE_BYTES: u64 = 256 << 20;
const PT_BASE: u64 = 0x0100_0000;
const PT2_BASE: u64 = 0x0180_0000;
const PT_BYTES: usize = 4 << 20;
const DATA_A: u64 = 0x0200_0000;
const DATA_B: u64 = 0x0210_0000;
const DATA_C: u64 = 0x0220_0000;
const VA_A: u64 = 0x20_0000_0000;
const VA_B: u64 = 0x20_4000_0000;
const VA_C: u64 = 0x21_0000_0000;
const PAGES: u64 = 16;
const BYTES: u32 = (PAGES * 4096) as u32;
/// A PDB inside the store that no object carries — (iv).
const STRANGER_PDB: u64 = 0x0F00_0000;
/// GA10x's usermode base; the offsets only have to be self-consistent here (the port derives
/// all three from it), and this puts the trigger at the measured `bar0+0xb830b0`.
const USERMODE_BASE: u64 = 0xBB_0000;
/// The owner's ceiling on an invalidate-trigger trap (`kayfabe-device/src/mmuinval.rs`).
const TRAP_CEILING_US: u128 = 1_000;
/// `V3_P4_PORT_MAP.md` §3 row 2: the worker's own-stack wall time per walk.
const SUBMIT_BUDGET_US: u64 = 50;
const K_MAIN: VasKey = VasKey(0x1);
const K_SECOND: VasKey = VasKey(0x2);
const TOKEN_WALK: u64 = 1;
const TOKEN_REQ: u64 = 2;

fn main() {
    let mut l = Checks::default();
    if let Err(e) = run(&mut l) {
        l.check("run", false, e);
    }
    let v = l.verdict();
    println!("GATE8_VERDICT={}", if v { "PASS" } else { "FAIL" });
    std::process::exit(if v { 0 } else { 1 });
}

fn pattern(seed: u32) -> Vec<u8> {
    (0..BYTES / 4).flat_map(|i| (seed ^ i).to_le_bytes()).collect()
}

/// The guest's tables in either family's format.
enum GuestTree {
    V2(Tree),
    V3(Tree3),
}

impl GuestTree {
    fn new(v3: bool, base: u64) -> GuestTree {
        if v3 { GuestTree::V3(Tree3::new(base, PT_BYTES)) } else { GuestTree::V2(Tree::new(base, PT_BYTES)) }
    }
    fn map4k(&mut self, va: u64, phys: u64) {
        match self {
            GuestTree::V2(t) => t.map4k(va, phys),
            GuestTree::V3(t) => t.map4k(va, phys),
        }
    }
    fn root(&self) -> u64 {
        match self {
            GuestTree::V2(t) => t.root,
            GuestTree::V3(t) => t.root,
        }
    }
    fn bytes(&self) -> &[u8] {
        match self {
            GuestTree::V2(t) => &t.img.mem,
            GuestTree::V3(t) => &t.img.mem,
        }
    }
}

/// ★ A real host target that also records whether the guest's trigger still read BUSY at every
/// host op — the §49.1 ordering, observed from the host side of the reconcile.
struct Observed<'a> {
    host: Recorded<HostVas<'a>>,
    port: &'a InvalidatePort,
    busy_at_op: RefCell<Vec<bool>>,
}

impl Observed<'_> {
    fn busy(&self) -> bool {
        self.port.read(self.port.regs().trigger).unwrap_or(0) & TRIGGER_BIT != 0
    }
}

impl MapTarget for Observed<'_> {
    fn map(&self, d: &Desired, defer: bool) -> Result<Mapped, String> {
        self.busy_at_op.borrow_mut().push(self.busy());
        self.host.map(d, defer)
    }
    fn unmap(&self, va: u64, defer: bool) -> Result<(), String> {
        self.busy_at_op.borrow_mut().push(self.busy());
        self.host.unmap(va, defer)
    }
    fn invalidate(&self) -> Result<(), String> {
        self.busy_at_op.borrow_mut().push(self.busy());
        self.host.invalidate()
    }
}

/// ★ The production walker, timed: the wall time of each `submit` on THIS thread and the GPU
/// time of each walk, so "the submit returned before the GPU finished" is measured per walk.
struct Timed {
    inner: GpuWalker,
    submit_ns: Vec<u64>,
    gpu_us: Vec<u64>,
    /// Per walk: was the GPU still working on it at the instant `submit` returned?
    in_flight_at_return: Vec<bool>,
}

impl Walker for Timed {
    fn submit(&mut self, entries: &[WalkEntry]) -> Result<(), String> {
        let t0 = std::time::Instant::now();
        let r = self.inner.submit(entries);
        self.submit_ns.push(u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX));
        if r.is_ok() {
            self.in_flight_at_return.push(!self.inner.kernel.gpu_done_now().map_err(|e| e.to_string())?);
        }
        r
    }
    fn poll(&mut self) -> Result<Option<WalkDone>, String> {
        let r = self.inner.poll()?;
        if let Some(d) = &r {
            self.gpu_us.push(d.gpu_us);
        }
        Ok(r)
    }
    fn ack(&mut self, generation: u64, codes: Vec<u8>) -> Result<(), String> {
        self.inner.ack(generation, codes)
    }
    fn reset(&mut self, slot: u32) -> Result<(), String> {
        self.inner.reset(slot)
    }
    fn slots(&self) -> u32 {
        self.inner.slots()
    }
}

/// One trigger the vCPU thread fires.
#[derive(Clone, Copy)]
struct Fire {
    pdb: u64,
    all_pdb: bool,
}

/// What the vCPU thread saw.
#[derive(Debug, Default)]
struct VcpuSaw {
    trap_ns: Vec<u128>,
    published: usize,
    cleared: bool,
    spin_us: u128,
}

/// ★ The vCPU: the guest's register sequence for each `fire`, back to back, then the guest's
/// spin on the trigger until it reads 0 (or 5 s — the guest's own 4 s timeout, rounded up).
/// ⊘ Nothing here blocks on the VA manager: a trap is atomics + one channel push + one wake.
fn vcpu(port: &InvalidatePort, tx: &mpsc::Sender<InvalidateRequest>, wake: &Notifier, fires: &[Fire]) -> VcpuSaw {
    let mut saw = VcpuSaw::default();
    let r = port.regs();
    for f in fires {
        let (lo, hi) = Invalidate::encode_pdb(f.pdb, PdbAperture::Vidmem);
        if !f.all_pdb {
            let _ = port.write(r.pdb, lo);
            let _ = port.write(r.upper_pdb, hi);
        }
        let word = TRIGGER_BIT | 1 | if f.all_pdb { 0b10 } else { 0 };
        let t0 = std::time::Instant::now();
        if let PortWrite::Publish(req) = port.write(r.trigger, word) {
            let _ = tx.send(req);
            let _ = wake.signal();
            saw.published += 1;
        }
        saw.trap_ns.push(t0.elapsed().as_nanos());
    }
    let t0 = std::time::Instant::now();
    let deadline = t0 + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if port.read(r.trigger).unwrap_or(TRIGGER_BIT) & TRIGGER_BIT == 0 {
            saw.cleared = true;
            break;
        }
        std::hint::spin_loop();
    }
    saw.spin_us = t0.elapsed().as_micros();
    saw
}

/// What one round did on the manager side.
#[derive(Debug, Default)]
struct Round {
    saw: VcpuSaw,
    walk_wakes: u32,
    collected: u32,
    applied: Vec<(VasKey, usize, usize)>,
    superseded: u64,
    cleared: u64,
}

type Mgr<'a> = VaManager<Timed, Observed<'a>>;

/// ★ One round: the vCPU thread fires `fires` and spins; this thread serves the two fds until
/// the vCPU is done and the manager is idle.
fn round(
    m: &mut Mgr<'_>,
    port: &InvalidatePort,
    poller: &Poller,
    req_wake: &Notifier,
    fires: &[Fire],
) -> Result<Round, String> {
    let (tx, rx) = mpsc::channel::<InvalidateRequest>();
    let before = m.stats.clone();
    let mut out = Round::default();
    let done = std::sync::atomic::AtomicBool::new(false);
    let saw = std::thread::scope(|s| -> Result<VcpuSaw, String> {
        let h = s.spawn(|| {
            let saw = vcpu(port, &tx, req_wake, fires);
            done.store(true, std::sync::atomic::Ordering::Release);
            saw
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let finished = done.load(std::sync::atomic::Ordering::Acquire);
            if finished && !m.in_flight() && m.pending() == 0 {
                break;
            }
            if std::time::Instant::now() > deadline {
                break;
            }
            let mut ready = ReadyTokens::new();
            poller.wait(&mut ready, PollTimeout::Millis(20)).map_err(|e| format!("epoll: {e:?}"))?;
            for t in ready.iter() {
                if t == TOKEN_REQ {
                    let _ = req_wake.drain();
                    while let Ok(req) = rx.try_recv() {
                        m.on_invalidate(req, port.trigger());
                    }
                } else if t == TOKEN_WALK {
                    out.walk_wakes += 1;
                    let r = m.on_walk_ready(port.trigger());
                    if r.collected {
                        out.collected += 1;
                    }
                    for (k, a) in r.applied {
                        out.applied.push((k, a.mapped, a.unmapped));
                    }
                }
            }
        }
        h.join().map_err(|_| "the vCPU thread panicked".to_string())
    })?;
    out.saw = saw;
    out.superseded = m.stats.superseded - before.superseded;
    out.cleared = m.stats.cleared - before.cleared;
    Ok(out)
}

fn run(l: &mut Checks) -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_arch::ids::GpuId(0), &kf_chip::choose_host_classes).map_err(|e| e.to_string())?;
    let res = rm.reserve_gpga(STORE_BYTES).map_err(|e| format!("reserve: {e:?}"))?;
    l.measure("store", format!("store {:#x} {} MiB contiguous_aligned={}", res.handle, STORE_BYTES >> 20, res.contiguous_aligned));
    phase(l, &rm, res.handle, "ver2", kf_format_ver2(), false)?;
    phase(l, &rm, res.handle, "ver3", kf_format_ver3(), true)?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn phase(l: &mut Checks, rm: &HostRm, store: u32, tag: &str, fmt: KfFormat, v3: bool) -> Result<(), String> {
    let fd = rm.export_to_new_fd(store).map_err(|e| format!("{tag} export: {e:?}"))?;
    let mut kernel = WalkKernel::bring_up(WalkCfg::default(), fmt).map_err(|e| format!("{tag}: {e}"))?;
    kernel.import_store(fd.fd_number(), STORE_BYTES).map_err(|e| format!("{tag}: {e}"))?;
    let sync0 = kernel.ctx_sync_calls();

    // The guest kernel's tables: VA_A -> DATA_A, VA_B -> DATA_B; a SECOND space VA_C -> DATA_A.
    let mut tree = GuestTree::new(v3, PT_BASE);
    let mut tree2 = GuestTree::new(v3, PT2_BASE);
    for i in 0..PAGES {
        tree.map4k(VA_A + i * 4096, DATA_A + i * 4096);
        tree.map4k(VA_B + i * 4096, DATA_B + i * 4096);
        tree2.map4k(VA_C + i * 4096, DATA_A + i * 4096);
    }
    let w = |at: u64, b: &[u8]| kernel.write_store(at, b).map_err(|e| format!("{tag}: {e}"));
    w(PT_BASE, tree.bytes())?;
    w(PT2_BASE, tree2.bytes())?;
    w(DATA_A, &pattern(0xA5A5_0000))?;
    w(DATA_B, &vec![0u8; BYTES as usize])?;
    w(DATA_C, &vec![0u8; BYTES as usize])?;

    let port = InvalidatePort::new(InvalidateRegs::from_usermode_base(USERMODE_BASE).ok_or("regs")?);
    let space = rm.alloc_vaspace().map_err(|e| format!("{tag} vaspace: {e:?}"))?;
    let space2 = rm.alloc_vaspace().map_err(|e| format!("{tag} vaspace2: {e:?}"))?;
    let walker = Timed { inner: GpuWalker { kernel }, submit_ns: vec![], gpu_us: vec![], in_flight_at_return: vec![] };
    let mut m: Mgr<'_> = VaManager::new(walker, STORE_BYTES, Box::new(|_, _| None));
    let obs = |sp| Observed { host: Recorded::new(HostVas { rm, space: sp, store, ram_obj: None }), port: &port, busy_at_op: RefCell::new(vec![]) };
    m.table.insert(K_MAIN, obs(space));
    m.table.insert(K_SECOND, obs(space2));
    m.table.set_root(K_MAIN, tree.root(), PdbAperture::Vidmem).map_err(|e| format!("{e:?}"))?;
    m.table.set_root(K_SECOND, tree2.root(), PdbAperture::Vidmem).map_err(|e| format!("{e:?}"))?;

    let poller = Poller::create().map_err(|e| format!("epoll: {e:?}"))?;
    let req_wake = Notifier::create().map_err(|e| format!("notifier: {e:?}"))?;
    poller
        .watch(std::os::fd::AsFd::as_fd(m.walker().inner.kernel.completion_fd()), TOKEN_WALK)
        .map_err(|e| format!("watch walk fd: {e:?}"))?;
    poller.watch(req_wake.as_source_fd(), TOKEN_REQ).map_err(|e| format!("watch req: {e:?}"))?;
    let main_fire = [Fire { pdb: tree.root(), all_pdb: false }];

    // ── Round 1: the first invalidate maps the guest's two ranges. ───────────────────────────
    let r1 = round(&mut m, &port, &poller, &req_wake, &main_fire)?;
    println!("MEASURE {tag}_round1 {r1:?}");
    let mapped1: usize = r1.applied.iter().filter(|a| a.0 == K_MAIN).map(|a| a.1).sum();
    l.check(
        if v3 { "ver3_first_invalidate_maps_the_guest_ranges" } else { "ver2_first_invalidate_maps_the_guest_ranges" },
        mapped1 == 2 && r1.cleared == 1,
        format!("mapped={mapped1} (want 2 coalesced) cleared={} stats={:?}", r1.cleared, m.stats),
    );
    l.check(
        if v3 { "ver3_vcpu_saw_the_clear" } else { "ver2_vcpu_saw_the_clear" },
        r1.saw.cleared,
        format!("spin {} us, published {}", r1.saw.spin_us, r1.saw.published),
    );
    l.check(
        if v3 { "ver3_completion_arrived_as_an_fd_wake" } else { "ver2_completion_arrived_as_an_fd_wake" },
        r1.collected == 1 && r1.walk_wakes >= 1,
        format!("walk wakes={} collected={}", r1.walk_wakes, r1.collected),
    );

    // (i) the CE copy, issued now that the trigger has cleared.
    let mut rig = CeRig::new(rm, space)?;
    let s = rig.copy(rm, VA_A, VA_B, BYTES)?;
    l.check(if v3 { "ver3_copy1_completes_by_event" } else { "ver2_copy1_completes_by_event" }, s.seen_at_wake && s.ce_released, format!("{s:?}"));
    let rd = |m: &Mgr<'_>, at: u64| -> Result<Vec<u8>, String> {
        let mut got = vec![0u8; BYTES as usize];
        m.walker().inner.kernel.read_store(at, &mut got).map_err(|e| e.to_string())?;
        Ok(got)
    };
    l.check(
        if v3 { "ver3_copy1_lands_where_the_guest_mapped_it" } else { "ver2_copy1_lands_where_the_guest_mapped_it" },
        rd(&m, DATA_B)? == pattern(0xA5A5_0000),
        "DATA_B == DATA_A pattern",
    );

    // ── Round 2: the guest REMAPS VA_B onto DATA_C and invalidates again. ────────────────────
    for i in 0..PAGES {
        tree.map4k(VA_B + i * 4096, DATA_C + i * 4096);
    }
    m.walker().inner.kernel.write_store(PT_BASE, tree.bytes()).map_err(|e| e.to_string())?;
    m.walker().inner.kernel.write_store(DATA_B, &vec![0u8; BYTES as usize]).map_err(|e| e.to_string())?;
    let r2 = round(&mut m, &port, &poller, &req_wake, &main_fire)?;
    println!("MEASURE {tag}_round2 {r2:?}");
    let (mp, um): (usize, usize) = r2.applied.iter().filter(|a| a.0 == K_MAIN).fold((0, 0), |(x, y), a| (x + a.1, y + a.2));
    l.check(
        if v3 { "ver3_remap_reconciles_before_the_clear" } else { "ver2_remap_reconciles_before_the_clear" },
        mp == 1 && um == 1 && r2.cleared == 1 && r2.saw.cleared,
        format!("mapped={mp} unmapped={um} cleared={}", r2.cleared),
    );
    let s = rig.copy(rm, VA_A, VA_B, BYTES)?;
    l.check(if v3 { "ver3_copy2_completes_by_event" } else { "ver2_copy2_completes_by_event" }, s.seen_at_wake && s.ce_released, format!("{s:?}"));
    l.check(
        if v3 { "ver3_copy2_follows_the_remap" } else { "ver2_copy2_follows_the_remap" },
        rd(&m, DATA_C)? == pattern(0xA5A5_0000) && rd(&m, DATA_B)?.iter().all(|&b| b == 0),
        "DATA_C == DATA_A pattern, DATA_B still zero",
    );

    // ── Round 3 (iii): two triggers back to back. ────────────────────────────────────────────
    // ⚠ The second arm lands microseconds after the first, while the first walk (~0.4 ms p50)
    // is still in flight — so the first completion must find it re-armed. A GPU that walked in
    // under the vCPU's inter-write gap would make this read cleared=2 superseded=0: a FAIL here
    // that names the timing, not the CAS.
    let r3 = round(&mut m, &port, &poller, &req_wake, &[main_fire[0], main_fire[0]])?;
    println!("MEASURE {tag}_round3 {r3:?}");
    l.check(
        if v3 { "ver3_a_rearmed_trigger_is_superseded_not_cleared" } else { "ver2_a_rearmed_trigger_is_superseded_not_cleared" },
        r3.saw.published == 2 && r3.superseded >= 1 && r3.cleared == 1 && r3.saw.cleared,
        format!("published={} superseded={} cleared={}", r3.saw.published, r3.superseded, r3.cleared),
    );

    // ── Round 4 (iv): a PDB no object carries. ───────────────────────────────────────────────
    let (missed0, walks0) = (m.stats.named_missed, m.stats.walks_submitted);
    let r4 = round(&mut m, &port, &poller, &req_wake, &[Fire { pdb: STRANGER_PDB, all_pdb: false }])?;
    l.check(
        if v3 { "ver3_named_missed_is_counted_and_cleared_without_a_walk" } else { "ver2_named_missed_is_counted_and_cleared_without_a_walk" },
        m.stats.named_missed == missed0 + 1 && m.stats.walks_submitted == walks0 && r4.cleared == 1 && r4.saw.cleared,
        format!("named_missed {}->{} walks {}->{}", missed0, m.stats.named_missed, walks0, m.stats.walks_submitted),
    );

    // ── Round 5 (v): ALL_PDB reconciles every rooted space — the second one's first map. ─────
    let r5 = round(&mut m, &port, &poller, &req_wake, &[Fire { pdb: 0, all_pdb: true }])?;
    println!("MEASURE {tag}_round5 {r5:?}");
    let second = m.table.target(K_SECOND).and_then(|g| g.host.resolve(VA_C, u64::from(BYTES)));
    l.check(
        if v3 { "ver3_all_pdb_reconciles_every_rooted_space" } else { "ver2_all_pdb_reconciles_every_rooted_space" },
        r5.cleared == 1 && second == Some((false, DATA_A)) && r5.applied.len() == 2,
        format!("applied={:?} second_resolves={second:?}", r5.applied),
    );

    // ── The whole phase: ordering, trap time, and the walk never blocking. ───────────────────
    let rounds = [&r1, &r2, &r3, &r4, &r5];
    let trap_max_us = rounds.iter().flat_map(|r| r.saw.trap_ns.iter()).max().copied().unwrap_or(0) / 1000;
    l.measure("trigger_trap", format!("{tag} worst={trap_max_us}us ceiling={TRAP_CEILING_US}us"));
    l.check(
        if v3 { "ver3_trigger_trap_is_under_the_ceiling" } else { "ver2_trigger_trap_is_under_the_ceiling" },
        trap_max_us < TRAP_CEILING_US,
        format!("worst trigger trap {trap_max_us} us"),
    );
    let t = m.walker();
    let sync = t.inner.kernel.ctx_sync_calls() - sync0;
    // ⊘ Not "submit time < GPU time": when launching is the bottleneck the GPU trails the CPU by
    // one launch and the two are equal by construction. The falsifier is the GPU's own answer
    // (`cuEventQuery`, non-blocking) at the instant `submit` returned: a submit that waited for
    // the walk would ALWAYS find it done.
    let in_flight = t.in_flight_at_return.iter().filter(|&&b| b).count();
    let returned_early = !t.in_flight_at_return.is_empty() && in_flight > 0;
    let mut sorted = t.submit_ns.clone();
    sorted.sort_unstable();
    let p50 = sorted.get(sorted.len() / 2).copied().unwrap_or(0) / 1000;
    let max = sorted.last().copied().unwrap_or(0) / 1000;
    l.measure(
        "walk_submit",
        format!(
            "{tag} walks={} submit_us p50={p50} max={max} budget={SUBMIT_BUDGET_US} ({}) gpu_us={:?} graph={} param_updates={} submit_ns={:?}",
            sorted.len(),
            if max <= SUBMIT_BUDGET_US { "met" } else { "MISSED" },
            t.gpu_us,
            t.inner.kernel.submits_as_graph(),
            t.inner.kernel.graph_param_updates(),
            t.submit_ns
        ),
    );
    l.check(
        if v3 { "ver3_the_walk_never_blocked_its_submitter" } else { "ver2_the_walk_never_blocked_its_submitter" },
        sync == 0 && returned_early,
        format!(
            "cuCtxSynchronize calls={sync}; walks still on the GPU when submit returned: {in_flight}/{} (submit_ns={:?} gpu_us={:?})",
            t.in_flight_at_return.len(),
            t.submit_ns,
            t.gpu_us
        ),
    );
    l.check(
        if v3 { "ver3_every_host_op_ran_while_the_guest_saw_busy" } else { "ver2_every_host_op_ran_while_the_guest_saw_busy" },
        ops_all_busy(&m),
        format!("stats={:?}", m.stats),
    );
    l.check(
        if v3 { "ver3_nothing_refused" } else { "ver2_nothing_refused" },
        m.stats.refusals.is_empty() && m.stats.unreconciled == 0 && m.stats.walks_refused == 0,
        format!("refusals={:?}", m.stats.refusals),
    );
    let _ = rig.channel();
    Ok(())
}

/// Whether every host op either target saw ran with the trigger still busy — and at least one
/// did (a vacuous "all" over zero ops would pass for any reason).
fn ops_all_busy(m: &Mgr<'_>) -> bool {
    let mut n = 0usize;
    let mut all = true;
    for k in [K_MAIN, K_SECOND] {
        if let Some(t) = m.table.target(k) {
            let v = t.busy_at_op.borrow();
            n += v.len();
            all &= v.iter().all(|&b| b);
        }
    }
    n > 0 && all
}
