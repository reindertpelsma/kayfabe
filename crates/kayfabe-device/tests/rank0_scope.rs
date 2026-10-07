//! ★★★★★ **w755 — CONSTRAINT 4, MEASURED AS A LOCK-SCOPE PROPERTY RATHER THAN A LATENCY.**
//!
//! `[measured w754]` the device arm's worst trap was `9 649 us @0xbb0090`, **52 % of it spent
//! WAITING**, with `LOCKCOST rank0 worst_wait=9625us worst_wait_blocked_by=plane.rs:3286`.
//! w754's own conclusion: *"the next cut is lock scope, not thread placement — moving more
//! work off the vCPU cannot help a trap already waiting on a lock a worker holds."*
//!
//! # ⊘ Why this is a TEST and not a benchmark
//!
//! A latency threshold is a statement about a box on a day. The defect underneath it is
//! structural and exactly checkable: `RegPlane::window_page_backing` opened
//! `let mut st = self.state.lock();` **unconditionally**, while only one of its three arms
//! (`Pramin`, the BAR0 window latch) ever reads `st`. The two translated windows — BAR1 and
//! BAR2, the arms the device lane actually runs — held **rank 0** across a full guest
//! page-table walk and the store's `page_backing`, protecting nothing at all.
//!
//! ⇒ *"the BAR1 path acquires rank 0 zero times"* is a property with a yes/no answer that
//! does not move between boxes, and it is the property the milliseconds were a symptom of.
//!
//! # ★ The instrument, and why it cannot be perturbed by the other tests
//!
//! [`kayfabe_util::lock::acquisitions`] is **thread-local** and monotonic, so a delta taken
//! around one call is a statement about that call even while the rest of this binary's tests
//! run in parallel on other threads.
//!
//! ⚠ **Its own known-positive is built in**: the same test asserts the `Pramin` arm takes rank
//! 0 **exactly once**. Without that row, `after - before == 0` would also be what a broken
//! counter, a mis-named rank, or a plane that refused before locking anything would produce —
//! the `a_census_zero_needs_a_known_positive` shape, on the one number the whole test rests
//! on.
//!
//! ⊘ Its own file rather than an addition to `two_worlds_split.rs`, whose docs require that it
//! *"stay the only test in this file that drives `window_page_backing`"* — the `twoworlds`
//! census it reads is process-wide and a second driver in the same binary could perturb its
//! deltas.

use kayfabe_abi::versions::BENCH_DRIVER;
use kayfabe_device::fbwin::SparseFb;
use kayfabe_device::plane::RegPlane;
use kayfabe_device::{FbWindow, NanoClock, SteppingClock, abi, ga10x::GA106};
use kayfabe_util::lock::{LockRank, acquisitions, held_depth, held_rank_mask};

mod tiny;
use tiny::{E_VALID, TinyFmt};

const BAR_REGS: u8 = kayfabe_abi::pcibars::bus_bar::REGS as u8;

/// This file's own frames and tables, so nothing here shares an address with another test.
const BAR1_PHYS: u64 = 0x0500_0000;
const CTRL_PHYS: u64 = 0x0520_0000;
const B1_L1: u64 = 0x0530_0000;
const BAR1_VA: u64 = 0x0080_0000;

fn pde(next: u64) -> u64 {
    (E_VALID as u64) | ((next >> 12) << 12)
}
fn leaf(phys: u64) -> u64 {
    (E_VALID as u64) | ((phys >> 12) << 12)
}

fn plane() -> RegPlane {
    let p = RegPlane::new(
        &GA106,
        abi::gsp_abi_for(BENCH_DRIVER).expect("the bench driver has a wire table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("GA106 is servable");
    p.set_fb(Box::new(SparseFb::new(GA106.fb_length)));
    p.set_mmu(Box::new(TinyFmt));
    p
}

fn point_window(p: &RegPlane, phys: u64) {
    let cur = p.read(BAR_REGS, GA106.bar0_window_reg, 4).value() as u32;
    let base = u32::try_from(phys >> 16).expect("a 24-bit window base");
    let val = (cur & !0x00FF_FFFF) | (base & 0x00FF_FFFF);
    p.write(BAR_REGS, GA106.bar0_window_reg, 4, u64::from(val));
}

fn pramin_wr32(p: &RegPlane, phys: u64, val: u32) {
    point_window(p, phys);
    let w = p.write(
        BAR_REGS,
        GA106.pramin_window.base + (phys & 0xFFFF),
        4,
        u64::from(val),
    );
    assert_eq!(
        w.fb_landed,
        Some(phys),
        "the PRAMIN write must land, and say where — otherwise the tree under test was \
         never written and every measurement below is about a refusal"
    );
}

fn pramin_wr_entry(p: &RegPlane, phys: u64, entry: u64) {
    pramin_wr32(p, phys, entry as u32);
    pramin_wr32(p, phys + 4, (entry >> 32) as u32);
}

fn build_bar1_tree(p: &RegPlane, va: u64, leaf_entry: u64) {
    let root = GA106.bar1_pde_base;
    assert_ne!(
        root, 0,
        "this chip row must publish a bar1PdeBase, or BAR1 has no address model and this \
         file would be measuring a refusal's lock scope instead of a translation's"
    );
    pramin_wr_entry(p, root + ((va >> 30) & 511) * 8, pde(B1_L1));
    pramin_wr_entry(p, B1_L1 + ((va >> 21) & 511) * 8, leaf_entry);
}

/// ★★★★★ **THE GATE. A translated-window resolution must acquire rank 0 ZERO times, and the
/// PRAMIN arm — which genuinely reads the latch — must acquire it exactly once.**
#[test]
fn a_translated_window_resolution_takes_no_rank_zero_lock() {
    let p = plane();
    build_bar1_tree(&p, BAR1_VA, leaf(BAR1_PHYS));

    // ── the property ────────────────────────────────────────────────────────────────────
    let before = acquisitions(LockRank::Plane);
    let r = p
        .window_page_backing(FbWindow::FbAperture, BAR1_VA, false)
        .expect("the BAR1 page resolves");
    let taken = acquisitions(LockRank::Plane) - before;

    assert_eq!(
        r.phys, BAR1_PHYS,
        "★ NON-VACUITY: the walk must actually have run. A resolution that refused, or that \
         answered `phys = off`, would take no locks either and would satisfy the rank-0 \
         assertion below while measuring nothing"
    );
    assert_eq!(
        taken, 0,
        "★★★★★ CONSTRAINT 4 REGRESSED — the BAR1 path acquired rank 0 {taken} time(s). \
         `[measured w754]` a vCPU trap waited 9 625 us on rank 0 held at plane.rs:3286, and \
         the reason was that `window_page_backing` took `self.state.lock()` for all three \
         windows while only PRAMIN reads it. Whatever re-widened that hold has put the \
         milliseconds back."
    );

    // ── the known-positive: the same counter, on the arm that SHOULD take it ─────────────
    point_window(&p, CTRL_PHYS);
    let before = acquisitions(LockRank::Plane);
    let pr = p
        .window_page_backing(
            FbWindow::Pramin,
            GA106.pramin_window.base + (CTRL_PHYS & 0xFFFF),
            false,
        )
        .expect("the PRAMIN page resolves");
    let taken_pramin = acquisitions(LockRank::Plane) - before;

    assert_eq!(pr.phys, CTRL_PHYS, "the PRAMIN window resolved elsewhere");
    assert_eq!(
        taken_pramin, 1,
        "★★★ THE KNOWN-POSITIVE FAILED, so the zero above proves nothing. PRAMIN reads the \
         BAR0 window latch out of rank-0 state and must take it exactly once; a {taken_pramin} \
         here means this counter cannot see the acquisition it is being used to rule out"
    );

    // ⊘ And nothing leaked: a guard held past the call would make every later measurement in
    // this thread a statement about a lock that never came back.
    assert_eq!(held_depth(), 0, "a ranked guard outlived the call");
    assert_eq!(held_rank_mask(), 0, "a rank bit never cleared");
}

/// ★★★ **The BAR2 arm is the same shape and is asserted separately, not folded into the row
/// above.**
///
/// ⊘ `bar1_translate` and `bar2_translate` are different functions reached through different
/// match arms. A single test on BAR1 would let BAR2 regress alone — and BAR2 is the instance
/// window, the path `kbusVerifyBar2` drives inside `RmInitAdapter`, so a rank-0 hold there is
/// on the critical path of the boot itself.
#[test]
fn the_instance_window_also_takes_no_rank_zero_lock() {
    let p = plane();
    let before = acquisitions(LockRank::Plane);
    // ⚠ Graded on the LOCK, not on the outcome: with no BAR2 root published this resolution
    // refuses, and that is fine — a refusal that took rank 0 would be the same defect.
    let _ = p.window_page_backing(FbWindow::InstanceWindow, 0x0010_0000, false);
    let taken = acquisitions(LockRank::Plane) - before;
    assert_eq!(
        taken, 0,
        "the BAR2/instance-window path acquired rank 0 {taken} time(s); see \
         `a_translated_window_resolution_takes_no_rank_zero_lock` for what that costs"
    );
    assert_eq!(held_depth(), 0, "a ranked guard outlived the call");
}
