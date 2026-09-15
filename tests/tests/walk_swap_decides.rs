//! ★★★★★ **§6 STEP 2 — THE DECIDER REACHES THE ADDRESS TABLE, AND THE SHAPE CHECK REFUSES.**
//!
//! `SINGLE_STORE_PLAN.md` §6's swap turns `PtSweepDecider` from an observer into something
//! that can change what is published. Two claims follow from that and neither is checkable in
//! the crate that implements it:
//!
//! 1. **The substitution is LIVE.** A decider that returns a well-shaped replacement changes
//!    the binding that lands in `AddressTable`. ⊘ This is the known-positive the whole
//!    increment rests on: the production swap is observationally neutral *by construction*
//!    (it only substitutes where the two walkers agree, and under agreement the two leaf sets
//!    are the same set), so **nothing about a clean boot can distinguish a working swap from a
//!    decider that is never consulted.** Only a deliberately-disagreeing decider can.
//! 2. **A mis-shaped replacement is REFUSED.** The results are keyed by `(gpu, pdb)` and
//!    COMMIT re-resolves each one, so a replacement that dropped, reordered or re-homed an
//!    address space would publish one VAS's leaves into another's table **and every
//!    downstream counter would still add up**. It must be refused whole.
//!
//! ⊘ The tree, the encoders and the window transport are `cpu_pt_transport.rs`'s, deliberately
//! — a private second set could drift from the format the port actually walks with while every
//! assertion here still passed.

use kayfabe_abi::versions::BENCH_DRIVER;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_device::plane::{NanoClock, RegPlane, SteppingClock};
use kayfabe_device::{SparseFb, abi, ga10x::GA106};
use kayfabe_mocks::MockIsolateFactory;
use kayfabe_rt::device::{LockMode, PtSweepDecider, SharedDevice};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

const GPU: GpuId = GpuId::ZERO;
const ROOT: u64 = 0x2_EFA9_C000;
const L1: u64 = 0x2_EFA9_B000;
const L2: u64 = 0x2_EFA9_A000;
const L3: u64 = 0x2_EFA8_0000;
const L5: u64 = 0x2_EFA7_F000;
const RING_VA: u64 = 0x4_2006_4000;
const RING_PHYS: u64 = 0x237F_E000;
/// Where the hostile decider re-points the leaf. ⊘ A different page, so the test cannot
/// pass by the two answers coinciding.
const DECIDED_PHYS: u64 = 0x1122_3000;
const PDB: Pdb = Pdb(ROOT);

fn pde_vid(next: u64) -> u64 {
    ((next >> 12) << 8) | (1 << 1)
}
fn dual_small_vid(next: u64) -> u64 {
    ((next >> 12) << 8) | (1 << 1)
}
fn pte_sys(phys: u64) -> u64 {
    ((phys >> 12) << 8) | (2 << 1) | 1
}

fn plane() -> RegPlane {
    let p = RegPlane::new(
        &GA106,
        abi::gsp_abi_for(BENCH_DRIVER).expect("the bench driver has a wire table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("GA106 is servable");
    p.set_fb(Box::new(SparseFb::new(GA106.fb_length)));
    p.set_mmu(Box::new(kayfabe_chips::Ga10xGmmu::new()));
    p
}

fn point_window(p: &RegPlane, phys: u64) {
    let bar = kayfabe_abi::pcibars::bus_bar::REGS as u8;
    let cur = p.read(bar, GA106.bar0_window_reg, 4).value() as u32;
    let base = u32::try_from(phys >> 16).expect("a 24-bit window base");
    p.write(
        bar,
        GA106.bar0_window_reg,
        4,
        u64::from((cur & !0x00FF_FFFF) | (base & 0x00FF_FFFF)),
    );
}

fn win_wr32(p: &RegPlane, phys: u64, val: u32) {
    point_window(p, phys);
    let w = p.write(
        kayfabe_abi::pcibars::bus_bar::REGS as u8,
        GA106.pramin_window.base + (phys & 0xFFFF),
        4,
        u64::from(val),
    );
    assert_eq!(w.fb_landed, Some(phys), "the window write must land");
}

fn win_wr_entry(p: &RegPlane, phys: u64, entry: u64) {
    win_wr32(p, phys, entry as u32);
    win_wr32(p, phys + 4, (entry >> 32) as u32);
}

fn build_tree(p: &RegPlane) {
    win_wr_entry(p, ROOT + ((RING_VA >> 47) & 3) * 8, pde_vid(L1));
    win_wr_entry(p, L1 + ((RING_VA >> 38) & 511) * 8, pde_vid(L2));
    win_wr_entry(p, L2 + ((RING_VA >> 29) & 511) * 8, pde_vid(L3));
    let slot = L3 + ((RING_VA >> 21) & 255) * 16;
    win_wr_entry(p, slot, 0);
    win_wr_entry(p, slot + 8, dual_small_vid(L5));
    win_wr_entry(p, L5 + ((RING_VA >> 12) & 511) * 8, pte_sys(RING_PHYS));
}

fn device() -> Guarded<SharedDevice> {
    let arch = std::sync::Arc::new(kayfabe_mocks::MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    Guarded::new("walk_swap_decides", gpu, rec).map(|g| SharedDevice::new(g, LockMode::Sharded))
}

fn owner(d: &SharedDevice) -> kayfabe_core::ProcId {
    d.live_pids()
        .into_iter()
        .find(|&pid| {
            d.with_proc(pid, |p| p.vas_by_pdb(GPU, PDB).is_some())
                .unwrap_or(false)
        })
        .expect("some proc owns the address space")
}

/// What the address table says [`RING_VA`] maps to, or `None` if it is a miss.
fn bound_phys(d: &SharedDevice, pid: kayfabe_core::ProcId) -> Option<u64> {
    d.with_proc(pid, |p| {
        p.vas_by_pdb(GPU, PDB)
            .and_then(|v| v.table.resolve(PDB, GpuVa(RING_VA)).ok().map(|(b, _off)| b.phys()))
    })
    .flatten()
}

/// A decider that re-points every leaf it is given at [`DECIDED_PHYS`]. ⊘ Not the production
/// swap — deliberately the opposite of it: the production swap only substitutes where the two
/// walkers AGREE, and this one substitutes something that could never agree. It exists to
/// make the substitution path observable at all.
struct RepointEverything {
    /// How many results it rewrote — so a test can tell "it decided nothing" from "it decided
    /// and the decision had no effect", which are different failures.
    rewrote: usize,
    /// Drop the last result from the replacement, to exercise the shape check.
    truncate: bool,
}

impl PtSweepDecider for RepointEverything {
    fn decide(
        &mut self,
        _fmt: &dyn kayfabe_rt::device::SweepFmt,
        _fb: &mut dyn kayfabe_mmu::walker::FbRead,
        results: &[kayfabe_fwd::PtDecodeResult],
    ) -> Option<Vec<kayfabe_fwd::PtDecodeResult>> {
        let mut out: Vec<kayfabe_fwd::PtDecodeResult> = results.to_vec();
        for r in &mut out {
            let Ok(d) = &mut r.decode else { continue };
            for l in &mut d.leaves {
                l.phys = DECIDED_PHYS;
                self.rewrote += 1;
            }
            for (_, pd) in &mut d.decodes {
                for l in &mut pd.leaves {
                    l.phys = DECIDED_PHYS;
                }
            }
        }
        if self.truncate {
            out.pop();
        }
        Some(out)
    }
}

/// ⊘ **THE CONTROL.** The disarmed decider publishes what the host walk found.
#[test]
fn the_disarmed_decider_publishes_the_host_walks_answer() {
    let g = device();
    let p = plane();
    build_tree(&p);
    let d: &SharedDevice = &g;
    let pid = owner(d);
    let f = kayfabe_chips::Ga10xGmmu::new();
    let mut fb = p.pt_bytes();
    let (_plan, out) = d
        .sweep_pt_tables_deciding(
            pid,
            &f,
            &mut fb,
            kayfabe_mmu::reach::PublishedUnbind::Refuse,
            &mut (),
        )
        .expect("the proc is live");
    assert!(out.bound >= 1, "the sweep bound the ring's leaf: {out:?}");
    assert_eq!(
        bound_phys(d, pid),
        Some(RING_PHYS),
        "the HOST walk's target is what is published"
    );
}

/// ★★★★★ **THE KNOWN-POSITIVE.** A well-shaped replacement reaches `AddressTable`.
///
/// ⊘ Without this test the whole increment is unfalsifiable: the production swap substitutes
/// only where the two walkers agree, so a boot cannot tell a live substitution from a decider
/// whose return value is dropped on the floor.
#[test]
fn a_well_shaped_replacement_changes_what_is_published() {
    let g = device();
    let p = plane();
    build_tree(&p);
    let d: &SharedDevice = &g;
    let pid = owner(d);
    let f = kayfabe_chips::Ga10xGmmu::new();
    let mut fb = p.pt_bytes();
    let mut dec = RepointEverything { rewrote: 0, truncate: false };
    let (_plan, out) = d
        .sweep_pt_tables_deciding(
            pid,
            &f,
            &mut fb,
            kayfabe_mmu::reach::PublishedUnbind::Refuse,
            &mut dec,
        )
        .expect("the proc is live");
    assert!(dec.rewrote >= 1, "the decider must have had a leaf to rewrite");
    assert!(out.bound >= 1, "something was bound: {out:?}");
    assert_eq!(
        bound_phys(d, pid),
        Some(DECIDED_PHYS),
        "the DECIDER's target is what is published — the substitution is live"
    );
}

/// ★★★ **A MIS-SHAPED REPLACEMENT IS REFUSED WHOLE**, and the host walk's answer is what
/// commits. ⊘ Refused *whole* rather than per-entry: a partial acceptance is exactly the
/// re-homing this check exists to prevent.
#[test]
fn a_replacement_that_names_different_tasks_is_refused_and_the_host_walk_commits() {
    let g = device();
    let p = plane();
    build_tree(&p);
    let d: &SharedDevice = &g;
    let pid = owner(d);
    let f = kayfabe_chips::Ga10xGmmu::new();
    let mut fb = p.pt_bytes();
    let mut dec = RepointEverything { rewrote: 0, truncate: true };
    let (plan, out) = d
        .sweep_pt_tables_deciding(
            pid,
            &f,
            &mut fb,
            kayfabe_mmu::reach::PublishedUnbind::Refuse,
            &mut dec,
        )
        .expect("the proc is live");
    assert!(!plan.tasks.is_empty(), "the sweep had work to do");
    assert!(out.bound >= 1, "the host walk's binding still committed: {out:?}");
    assert_eq!(
        bound_phys(d, pid),
        Some(RING_PHYS),
        "a replacement of the wrong shape must NOT reach the table"
    );
}
