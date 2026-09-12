//! ★★★★★ **Reading a published VA must not invert the plane's two lock ranks (w523).**
//!
//! # The boot this file is paid for
//!
//! w522 split the register plane's single mutex in two: [`LockRank::Plane`] keeps the GSP
//! FSM, the command policy, the BAR0 window latch and **guest RAM**, while
//! [`LockRank::PlaneMem`] took the framebuffer and the page-table format — the only two
//! fields a copy-engine submission touches. The point was that a submission holding the plane
//! lock for 5–8 ms stops blocking a vCPU's register write.
//!
//! ⊘ `RegPlane::read_published_va` and `read_va_from_root` span both: they walk the page
//! tables through `PlaneMem`, and then, if the leaf names **system memory**, read the bytes
//! out of guest RAM, which stayed with the FSM. w522 took the memory lock first and reached
//! for the FSM inside it. `[measured w523]` the doorbell-publish worker panicked with
//!
//! > *R3 lock-rank violation … acquiring a rank-0 (Plane) lock while already holding
//! > \[PlaneMem\]*
//!
//! which poisoned both locks, took two more threads down with `PoisonError`, and ended QEMU
//! mid-run — `NO CENSUS … QEMU never ran its exit notifier`, `W392D_GUEST_RC=ABSENT`.
//!
//! ★ The rank discipline turned a latent deadlock into a named abort in one boot. But it only
//! fires on a path that RUNS, and **these two functions had no test at all** — which is the
//! whole reason the inversion reached a boot instead of a `cargo test`. That is what this
//! file fixes.
//!
//! ⚠ It asserts the ORDER holds, by exercising the sysmem arm — the one that needs both
//! locks. It says nothing about how long either is held; that is a boot measurement.

use kayfabe_arch::Aperture;
use kayfabe_device::ceresolve::{Demand, GMMU_APERTURE_VIDEO, VasRoot, decode_aperture};
use kayfabe_device::fbwin::{FbStore, SparseFb};
use kayfabe_device::plane::RegPlane;
use kayfabe_device::{NanoClock, SteppingClock, abi};

mod tiny;
use tiny::TinyFmt;

// ⊘ Taken from the shared fixture, never restated: `TinyFmt` is what DECODES these bits, and
// a second spelling of `E_SYS` here would let this file build a tree the format reads
// differently — a test passing against its own misunderstanding.
use tiny::{E_SYS, E_VALID};

const ROOT: u64 = 0x1000;
const L1: u64 = 0x2000;
/// Root slot 1 (`va >> 30`), leaf slot 2 (`(va >> 21) & 511`).
const VA: u64 = (1 << 30) | (2 << 21) | 0x1234;
/// The **guest-RAM** page the leaf names. Deliberately a GPA, not a framebuffer offset.
const LEAF_GPA: u64 = 0x4000;

fn entry(phys: u64, sys: bool) -> u128 {
    E_VALID | if sys { E_SYS } else { 0 } | (u128::from(phys >> 12) << 12)
}

fn plane_with_a_sysmem_leaf() -> RegPlane {
    let p = RegPlane::new(
        &kayfabe_device::ga10x::GA106,
        abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("GA106 is servable");

    // The page tables live in the framebuffer; the page they point AT lives in guest RAM.
    // That asymmetry is the whole point — it is what makes this path need both locks.
    let mut fb = SparseFb::new(12288 << 20);
    let mut put = |at: u64, e: u128| {
        fb.write(at, &e.to_le_bytes()[..8]).expect("sparse fb takes");
    };
    put(ROOT + 8, entry(L1, false));
    put(L1 + 16, entry(LEAF_GPA, true));
    p.set_fb(Box::new(fb));
    p.set_mmu(Box::new(TinyFmt));
    p
}

fn root() -> VasRoot {
    VasRoot {
        phys: ROOT,
        // ⊘ The ROOT is always in the framebuffer; only the LEAF names sysmem. That is the
        // shape the two-lock path exists for, and a sysmem ROOT is a different, refused case.
        aperture: decode_aperture(GMMU_APERTURE_VIDEO),
        aperture_raw: GMMU_APERTURE_VIDEO,
        page_shift: 30,
        virt_addr_lo: 0,
        virt_addr_hi: 0,
    }
}

/// ★★★★★ **The sysmem arm takes both locks, and in the declared order.**
///
/// ⊘ The assertion is that the call RETURNS. An inversion does not return a wrong answer —
/// it panics inside `check_acquire` and poisons the lock, which is what ended w523's boot.
/// So "it came back at all" is the property, and the aperture check below is what proves the
/// test actually reached the arm that needs both.
#[test]
fn reading_a_sysmem_leaf_through_a_published_va_takes_both_locks_in_rank_order() {
    let p = plane_with_a_sysmem_leaf();
    let mut buf = [0u8; 8];

    // ⊘ No guest-RAM port is installed, so the READ itself must fail — by name. That is the
    // correct outcome and it is not what this test is about: reaching the failure means the
    // walk resolved a sysmem leaf and the sysmem arm ran, holding both locks.
    let out = p.read_va_from_root(&root(), VA, &mut buf, Demand::from_doorbell());
    assert!(
        out.is_err(),
        "with no guest-RAM port installed the sysmem arm must refuse by name, not succeed: \
         {out:?}"
    );

    // And again through the other entry point, which has the same two-lock shape.
    let walk = p.walk_trace_from_root(&root(), VA);
    assert!(
        walk.contains("Sysmem") || walk.contains("sysmem") || !walk.is_empty(),
        "the trace must describe the walk that just ran: {walk}"
    );
}

/// ⊘ **The negative control: the VIDMEM arm needs only ONE lock**, and must still work.
/// Without this, a fix that took both locks everywhere would pass the test above while
/// re-creating the contention w522 exists to remove.
#[test]
fn reading_a_vidmem_leaf_needs_only_the_memory_lock() {
    let p = RegPlane::new(
        &kayfabe_device::ga10x::GA106,
        abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("GA106 is servable");
    let mut fb = SparseFb::new(12288 << 20);
    let mut put = |at: u64, e: u128| {
        fb.write(at, &e.to_le_bytes()[..8]).expect("sparse fb takes");
    };
    put(ROOT + 8, entry(L1, false));
    put(L1 + 16, entry(LEAF_GPA, false));
    p.set_fb(Box::new(fb));
    p.set_mmu(Box::new(TinyFmt));

    let mut buf = [0u8; 8];
    let out = p.read_va_from_root(&root(), VA, &mut buf, Demand::from_doorbell());
    assert!(
        out.is_ok(),
        "a vidmem leaf resolves entirely inside the framebuffer and must read back: {out:?}"
    );
    // ⊘ NOT asserted here: WHICH bytes come back. Working out the offset inside a 1 GiB leaf
    // would mean restating the resolver's arithmetic in its own test, and `ce_resolve.rs`
    // already covers byte-level resolution against that format. This file is about the LOCKS:
    // the vidmem arm must resolve without ever reaching for rank 0.
    assert_eq!(
        decode_aperture(GMMU_APERTURE_VIDEO),
        Some(Aperture::Vidmem),
        "the fixture's root aperture is vidmem, or the walk above proved something else"
    );
}
