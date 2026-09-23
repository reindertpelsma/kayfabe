//! The memory map is the whole trap policy, so these are the tests that make it load-bearing.
//!
//! ⊘ `THE_CONSTRAINTS.md` §53, `THE_BAR0_DISPOSITION_MAP.md`.

use kayfabe_doorbell::classgen::Family;
use kayfabe_doorbell::memmap::*;
use kayfabe_doorbell::trappolicy::{doorbell_for, PRAMIN_BASE, PRAMIN_LEN};
use kayfabe_doorbell::vmm::Bar;

const FAMILIES: [Family; 5] =
    [Family::Turing, Family::Ampere, Family::Ada, Family::Hopper, Family::Blackwell];

fn map_for(f: Family) -> MemoryMap {
    memory_map(f, doorbell_for(f), 16 << 20, 256 << 20, 32 << 20)
}

#[test]
fn the_map_tiles_every_bar_because_a_gap_is_an_accidental_read_exit() {
    // ★★★ THE structural property. An uncovered span is not a harmless omission — KVM turns it
    // into an MMIO exit, i.e. a read trap nobody decided to have. Tiling is what lets a VMM
    // install the map blindly with nothing left to infer.
    for f in FAMILIES {
        map_for(f).tiles().unwrap_or_else(|e| panic!("{f:?}: {e}"));
    }
    // ⊘ And at sizes that stress the edges: a BAR too small to contain PRAMIN or the VF page must
    // still tile, by dropping the cut rather than emitting a region past the end.
    for bar0 in [1 << 20, 8 << 20, 16 << 20, 64 << 20] {
        for f in FAMILIES {
            let m = memory_map(f, doorbell_for(f), bar0, 256 << 20, 32 << 20);
            m.tiles().unwrap_or_else(|e| panic!("{f:?} bar0={bar0:#x}: {e}"));
        }
    }
}

#[test]
fn the_current_product_target_has_no_read_exits_at_all() {
    // ★★★ The headline, asserted rather than asserted-in-prose: GSP Turing/Ampere/Ada boots with
    // ZERO disposition-D pages. This is what THE_CONSTRAINTS.md:28 measured at w708-w710
    // (raw client + cup3 + LLM, all TRAP_FILLS=0) and what the source review re-derived.
    for f in [Family::Turing, Family::Ampere, Family::Ada] {
        let m = map_for(f);
        assert_eq!(m.read_exit_pages(), 0, "{f:?} grew a read exit: {:?}", m.read_exit_regions());
        assert!(holes_for(f).is_empty(), "{f:?}");
    }
}

#[test]
fn hopper_and_blackwell_have_a_read_exit_and_each_one_is_NAMED() {
    // ⊘ The exception §52 was forced to concede. It is bounded, and the bound is what matters:
    // one page per family (two for Blackwell, which is split by die group), each boot-only.
    // ⚠ Contradicted by: an unnamed hole, or a hole count that grows without a source citation.
    assert_eq!(map_for(Family::Hopper).read_exit_pages(), 1);
    assert_eq!(map_for(Family::Blackwell).read_exit_pages(), 2);

    for f in [Family::Hopper, Family::Blackwell] {
        for r in map_for(f).read_exit_regions() {
            let Disposition::Hole { why } = r.how else { unreachable!() };
            // A hole must say WHICH register dragged the page in — the page costs 4 KiB of
            // implementation, so a reader must never have to guess what bought it.
            assert!(why.contains("EMEMD"), "{f:?}: hole at {:#x} is not named: {why:?}", r.base);
            assert_eq!(r.len, PAGE, "a hole is exactly one page");
            assert_eq!(r.base % PAGE, 0, "a hole must be page-aligned or KVM cannot express it");
        }
    }
}

#[test]
fn pramin_is_plain_ram_and_the_window_latch_that_moves_it_is_not() {
    // ⊘ §53: PRAMIN is disposition A — no exit in either direction — precisely because the
    // register that re-points it (NV_PBUS_BAR0_WINDOW, 0x1700) lives OUTSIDE PRAMIN and is
    // therefore a plain B register we already trap. The trapped write does the mmap re-point
    // synchronously; the reads that follow hit correct memory with no exit.
    let m = map_for(Family::Ampere);
    for off in [PRAMIN_BASE, PRAMIN_BASE + 0x1000, PRAMIN_BASE + PRAMIN_LEN - 4] {
        assert_eq!(m.disposition_at(Bar(0), off), Some(Disposition::PlainRam), "{off:#x}");
    }
    // ★ KNOWN-POSITIVE: the latch itself must NOT be plain RAM, or the re-point never happens.
    assert_eq!(m.disposition_at(Bar(0), 0x1700), Some(Disposition::ShadowWriteTrapped));
    assert!(m.disposition_at(Bar(0), 0x1700).unwrap().write_exits(), "the latch write must exit");
    assert!(!m.disposition_at(Bar(0), PRAMIN_BASE).unwrap().write_exits(), "PRAMIN must not exit");
}

#[test]
fn the_counter_page_is_a_host_mapping_and_it_is_the_only_one() {
    // ⊘ §53 disposition C. The usermode/VF page is the only BAR0 region RM maps to an
    // unprivileged host process (that is WHY it is exposed — it holds the doorbell), so it is the
    // only region we can alias to live host values instead of authoring.
    let m = map_for(Family::Ampere);
    assert_eq!(m.disposition_at(Bar(0), VF_USERMODE_PAGE), Some(Disposition::HostPassthrough));
    assert_eq!(m.disposition_at(Bar(0), VF_USERMODE_PAGE + 0x80), Some(Disposition::HostPassthrough),
        "VF_TIME_0 must be inside it");
    let c = m.regions.iter().filter(|r| r.how == Disposition::HostPassthrough).count();
    assert_eq!(c, 1, "exactly one host-passthrough region; a second needs its own argument");
    // ⊘ And the timer page 0x9000 is NOT it — that is the non-GSP problem, and it is B with a
    // refreshed shadow because RM never maps 0x9000 to userspace.
    assert_eq!(m.disposition_at(Bar(0), 0x9400), Some(Disposition::ShadowWriteTrapped));
}

#[test]
fn bar2_never_exits_and_bar1_exits_only_on_the_doorbell_page() {
    // `[owner]` "no traps for bar1/2 (except doorbell in bar1)".
    for f in FAMILIES {
        let m = map_for(f);
        for r in m.regions.iter().filter(|r| r.bar == Bar(2)) {
            assert_eq!(r.how, Disposition::PlainRam, "{f:?}: BAR2 must never exit");
        }
        let exiting: Vec<_> =
            m.regions.iter().filter(|r| r.bar == Bar(1) && r.how.write_exits()).collect();
        match doorbell_for(f) {
            kayfabe_doorbell::trappolicy::DoorbellPlacement::Bar0 { .. } => {
                assert!(exiting.is_empty(), "{f:?}: BAR1 must not exit with a BAR0 doorbell");
            }
            kayfabe_doorbell::trappolicy::DoorbellPlacement::Bar1 { page_base } => {
                assert_eq!(exiting.len(), 1, "{f:?}");
                assert_eq!((exiting[0].base, exiting[0].len), (page_base, 0x1_0000));
                // ⊘ Writes only — the doorbell is rung by a write. Reads of that page stay free.
                assert!(!exiting[0].how.read_exits(), "{f:?}: the doorbell page must not read-exit");
            }
        }
        // ★ No BAR1/BAR2 region may EVER read-exit, under any family.
        assert!(
            m.regions.iter().filter(|r| r.bar != Bar(0)).all(|r| !r.how.read_exits()),
            "{f:?}: a read exit outside BAR0"
        );
    }
}

#[test]
fn known_positive_the_tiling_check_can_actually_fail() {
    // ⊘⊘⊘ A census zero needs a known-positive, and this session paid for that lesson three times
    // (an 807-file sweep blind by construction; a param readback; a register count that missed
    // everything declared relative to a base). ⇒ Before trusting `tiles()` returning Ok, prove it
    // returns Err on a map that genuinely has a gap.
    let mut m = map_for(Family::Ampere);
    assert!(m.tiles().is_ok());

    let gapped = {
        let mut g = m.clone();
        g.regions.retain(|r| !(r.bar == Bar(0) && r.base == PRAMIN_BASE));
        g
    };
    let e = gapped.tiles().expect_err("⊘ a map missing PRAMIN must NOT tile");
    assert!(e.contains("GAP"), "the error must name the failure mode, got: {e}");

    // And an overlap must fail too, in the other direction.
    m.regions.push(Region { bar: Bar(0), base: 0, len: PAGE, how: Disposition::PlainRam });
    assert!(m.tiles().is_err(), "⊘ a duplicated region must NOT tile");
}

// ---- the seam: can a VMM actually install this map? -------------------------------------------

use kayfabe_doorbell::vmm::{HostMapping, SlotId, VmmError, VmmOps};
use std::sync::Mutex;

/// Where the guest programmed each BAR. ⊘ Deliberately far apart and NOT at 0, so a bug that
/// confuses a BAR-relative offset for a GPA lands outside every BAR instead of accidentally
/// inside one — which is exactly the bug that slipped through the first version of this test.
fn bar_base(bar: kayfabe_doorbell::vmm::Bar) -> Option<u64> {
    match bar.0 {
        0 => Some(0xF000_0000),
        1 => Some(0x10_0000_0000),
        2 => Some(0x20_0000_0000),
        _ => None,
    }
}

/// A recording VMM. ⊘ It exists to answer one question the trait alone cannot: **is the seam
/// COMPLETE** — can a map be installed through it without any verb the trait is missing?
#[derive(Default)]
struct RecordingVmm {
    slots: Mutex<Vec<(u64, u64, bool)>>, // (gpa, len, readonly)
}
impl VmmOps for RecordingVmm {
    fn guest_read(&self, _: u64, _: &mut [u8]) -> Result<(), VmmError> { Ok(()) }
    fn guest_write(&self, _: u64, _: &[u8]) -> Result<(), VmmError> { Ok(()) }
    fn install_memslot(&self, gpa: u64, len: u64, _h: HostMapping, ro: bool) -> Result<SlotId, VmmError> {
        let mut s = self.slots.lock().unwrap();
        s.push((gpa, len, ro));
        Ok(SlotId(s.len() as u32 - 1))
    }
    fn remove_memslot(&self, _: SlotId) -> Result<(), VmmError> { Ok(()) }
    fn raise_irq(&self, _: u32) -> Result<(), VmmError> { Ok(()) }
    fn signal_worker(&self) {}
    fn signal_drainer(&self) {}
}

#[test]
fn the_seam_is_complete_enough_to_install_the_whole_map() {
    // ⊘ `VmmOps` had ZERO implementors — a trait nothing implements is an island the orphan gate
    // cannot see, because the gate asks which MODULES are reached, not which traits are inhabited.
    // This test is the smallest thing that proves the seam is implementable and sufficient.
    for f in FAMILIES {
        let m = map_for(f);
        let vmm = RecordingVmm::default();
        let n = install(&m, &vmm, bar_base, |r| Some(HostMapping(r.base))).unwrap();
        let slots = vmm.slots.lock().unwrap();

        // Every region EXCEPT the holes became a memslot.
        let expect = m.regions.len() - m.read_exit_regions().len();
        assert_eq!(slots.len(), expect, "{f:?}");
        assert_eq!(n.len(), expect);

        // ★ And the readonly flag IS the disposition — that is the whole translation.
        for (r, _) in &n {
            let gpa = bar_base(r.bar).unwrap() + r.base;
            let (_, _, ro) = slots.iter().find(|(g, l, _)| *g == gpa && *l == r.len).unwrap();
            match r.how {
                Disposition::PlainRam => assert!(!ro, "PRAMIN/BAR1/BAR2 must be r/w: {r:?}"),
                Disposition::ShadowWriteTrapped | Disposition::HostPassthrough =>
                    assert!(ro, "reads-from-DRAM/writes-exit must be READ-ONLY: {r:?}"),
                Disposition::Hole { .. } => unreachable!("a hole must not be installed"),
            }
        }
    }
}

#[test]
fn a_hole_is_installed_by_NOT_installing_it() {
    // ★★★ The design in one assertion. kayfabe's only read traps are produced by a `continue`.
    // ⚠ KNOWN-POSITIVE built in: Ampere must install its 0x8F2000 span (it is ordinary B there),
    // and Hopper must NOT — so a bug that installed holes anyway would fail on Hopper, and a bug
    // that skipped that address everywhere would fail on Ampere.
    let probe = bar_base(kayfabe_doorbell::vmm::Bar(0)).unwrap() + 0x008F_2000;

    let vmm = RecordingVmm::default();
    let m = map_for(Family::Hopper);
    install(&m, &vmm, bar_base, |r| Some(HostMapping(r.base))).unwrap();
    let covered = |v: &RecordingVmm, a: u64| {
        v.slots.lock().unwrap().iter().any(|(g, l, _)| (*g..*g + *l).contains(&a))
    };
    assert!(!covered(&vmm, probe), "⊘ Hopper's FSP page must have NO memslot — that IS the trap");
    assert!(covered(&vmm, probe - PAGE), "…and the page before it must still be backed");
    assert!(covered(&vmm, probe + PAGE), "…and the page after it");

    let vmm2 = RecordingVmm::default();
    install(&map_for(Family::Ampere), &vmm2, bar_base, |r| Some(HostMapping(r.base))).unwrap();
    assert!(covered(&vmm2, probe), "⊘ on Ampere 0x8F2000 is ordinary B and MUST be backed");
}

#[test]
fn install_refuses_by_name_when_a_region_has_no_backing() {
    // ⊘ `[fable w825]` install() used to `continue` here — silently leaving a hole, which by its
    // own doc is an accidental read exit. ★ KNOWN-POSITIVE: withhold backing for PRAMIN only.
    let m = map_for(Family::Ampere);
    let vmm = RecordingVmm::default();
    let r = install(&m, &vmm, bar_base, |r| {
        if r.bar == Bar(0) && r.base == PRAMIN_BASE { None } else { Some(HostMapping(r.base)) }
    });
    assert!(
        matches!(r, Err(kayfabe_doorbell::vmm::VmmError::Unbacked { bar: 0, base, .. }) if base == PRAMIN_BASE),
        "got {r:?}"
    );
}
