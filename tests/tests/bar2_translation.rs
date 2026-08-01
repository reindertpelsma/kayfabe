//! ★★★ **`kbusVerifyBar2`'s MMU sub-test, reproduced** — `#149`, the first GMMU
//! translation this port has ever performed.
//!
//! # The measurement this file is written against
//!
//! Boot `l2evict1`, 2026-08-01, rev `9551dd1`, a stock unpatched 580.159.04 guest
//! (`docs/design/boot_measured_2026_08_01.md` §27-§29):
//!
//! ```text
//! NVRM: kbusVerifyBar2_GM107: MMUTest BAR0 window offset 0x70e000 returned garbage 0x0
//! NVRM: … [NV_ERR_MEMORY_ERROR] (0x72) @ kern_bus_gm107.c:360 → :465
//! NVRM: RmInitNvDevice: *** Cannot initialize the device → RmInitAdapter failed! (0x24:0x72:1220)
//! ```
//!
//! `kbusVerifyBar2_GM107:4155-4230` does four things, in this order, and this file does all
//! four:
//!
//! 1. `MEM_WR32(pOffset + index, SAMPLEDATA)` — sixteen bytes through the **BAR2 CPU
//!    mapping**, a GMMU-*translated* aperture;
//! 2. `GPU_REG_RD32(DRF_BASE(NV_PRAMIN) + …)` — the same sixteen bytes back through the
//!    **untranslated** BAR0 moving window, at their physical framebuffer address;
//! 3. `GPU_REG_WR32(temp + index, SAMPLEDATA + 0x10)` — the other direction, through the
//!    window;
//! 4. `MEM_RD32(pOffset + index)` — read back through BAR2.
//!
//! ⇒ **two apertures must agree about one physical byte, in both directions.**
//!
//! # ★★★ WHAT MAKES THIS TEST NON-VACUOUS: the page tables are written THE GUEST'S WAY
//!
//! Nothing here reaches into a store. The whole page-table tree is written **through the
//! BAR0 moving window**, dword by dword, exactly as `kbusSetupBar2GpuVaSpace_GM107` writes
//! it — because that is the fact the rung turns on. If the walk read from a *second* store,
//! or if the window resolved an address the walk did not, every assertion below would still
//! be satisfiable and the guest's own test would still fail. The two apertures agreeing is
//! only meaningful when the bytes had one place to be.
//!
//! # ⊘ What this file does NOT establish
//!
//! - ⊘ **It is not a boot.** It is the same *statement* the guest makes, made against the
//!   same port, with no guest and no hardware. `only_live_boots_are_proof` stands.
//! - ⊘ **The tree here is one shape.** A real guest's BAR2 tree is built by `mmuWalk` and
//!   may use the big-page sub-table, sparse fills and reserved entries this file does not
//!   construct.
//! - ⊘ **No page-table page is ever written through BAR2 itself**, which is what
//!   `kbusUpdateRmAperture_GM107` does after bootstrap. That path is reachable and untested
//!   here.

use kayfabe_abi::versions::{self, BENCH_DRIVER};
use kayfabe_device::plane::{
    BAR2_FOREIGN_APERTURE, BAR2_OUTSIDE_PUBLISHED_SLOT, BAR2_READ_ONLY, BAR2_UNKNOWN_ROOT_LEVEL,
    BAR2_UNROOTED, BAR2_WRITE_REFUSED, NO_MMU_PORT, NanoClock, ReadOutcome, RegPlane,
    SteppingClock,
};
use kayfabe_device::{FbWindow, SparseFb, abi, bar2::BarPdeLog, ga10x::GA106};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

// ───────────────────────────── the chip's own geometry ─────────────────────────────

/// `NV_PBUS_BAR0_WINDOW`, from the chip row rather than from a constant here.
fn win_reg() -> u64 {
    GA106.bar0_window_reg
}

/// `DRF_BASE(NV_PRAMIN)`, ditto.
fn pramin() -> u64 {
    GA106.pramin_window.base
}

/// RM's logical index for the instance/`BAR2` window.
const BAR_INST: u8 = kayfabe_abi::pcibars::bus_bar::INST as u8;
/// …and for the register aperture.
const BAR_REGS: u8 = kayfabe_abi::pcibars::bus_bar::REGS as u8;

/// `SAMPLEDATA` (`ogkm-580: kern_bus_gm107.c:3992`).
const SAMPLEDATA: u32 = 0xabcd_abcd;
/// `FBSIZETESTED` — sixteen bytes.
const FBSIZETESTED: u64 = 0x10;

/// The physical framebuffer address the measured boot's heap handed the test
/// (`boot_measured_2026_08_01.md` §17: `bar0TestAddr = 0x2efbae000`, window base `0x2efba`,
/// BAR0 offset `0x70e000`).
///
/// ⊘ Not a constant of anything — it is where RM's heap happened to place a 16-byte
/// allocation on one boot. It is used here so the arithmetic this file exercises is the
/// arithmetic that failed, not because any of it depends on the value.
const TEST_PHYS: u64 = 0x2_EFBA_E000;

/// Where this test puts the BAR2 page-table tree. Any four distinct framebuffer pages do.
const PD2_TBL: u64 = 0x0100_0000;
const PD1_TBL: u64 = 0x0100_1000;
const PD0_TBL: u64 = 0x0100_2000;
const PT_TBL: u64 = 0x0100_3000;

/// The BAR2 aperture offset the test buffer is mapped at — i.e. the **virtual address**.
/// Comfortably inside the 32 MiB window, and page-aligned like `kbusMapRmAperture`'s.
const TEST_VA: u64 = 0x0031_2000;

// ───────────────────────────── VER2 entry encodings ─────────────────────────────

/// A single page-directory entry pointing at a vidmem sub-table (`APERTURE = VIDEO`).
fn pde_vid(next: u64) -> u64 {
    ((next >> 12) << 8) | (1 << 1)
}

/// The high qword of a dual `PD0` entry: the SMALL-page sub-table, in vidmem.
fn dual_small_vid(next: u64) -> u64 {
    ((next >> 12) << 8) | (1 << 1)
}

/// A page-table entry mapping a vidmem page.
fn pte_vid(phys: u64) -> u64 {
    ((phys >> 12) << 8) | 1
}

/// A page-table entry mapping a **system-memory** page — the aperture this port refuses to
/// serve through the bus window.
fn pte_sys(phys: u64) -> u64 {
    ((phys >> 12) << 8) | (2 << 1) | 1
}

// ───────────────────────────── the plane, and the guest's own hands ─────────────────

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

/// Point the BAR0 moving window at `phys`, exactly as `GPU_FLD_WR_DRF_NUM(_BASE)` does:
/// a read-modify-write of the whole register.
fn point_window(p: &RegPlane, phys: u64) {
    let cur = p.read(BAR_REGS, win_reg(), 4).value() as u32;
    let base = u32::try_from(phys >> 16).expect("a 24-bit window base");
    let val = (cur & !0x00FF_FFFF) | (base & 0x00FF_FFFF);
    p.write(BAR_REGS, win_reg(), 4, u64::from(val));
}

/// Write one dword **through the BAR0 moving window** at framebuffer address `phys`.
///
/// ★ This is the only way this file ever puts a byte in the framebuffer. See the module
/// docs for why that is what makes the whole file non-vacuous.
fn win_wr32(p: &RegPlane, phys: u64, val: u32) {
    point_window(p, phys);
    let w = p.write(BAR_REGS, pramin() + (phys & 0xFFFF), 4, u64::from(val));
    assert_eq!(
        w.fb_landed,
        Some(phys),
        "the window write must land, and say where"
    );
}

/// Read one dword back through the BAR0 moving window.
fn win_rd32(p: &RegPlane, phys: u64) -> u32 {
    point_window(p, phys);
    p.read(BAR_REGS, pramin() + (phys & 0xFFFF), 4).value() as u32
}

/// Write one 64-bit page-table entry through the window, low half first — which is how a
/// guest with a 32-bit register interface has to do it.
fn win_wr_entry(p: &RegPlane, phys: u64, entry: u64) {
    win_wr32(p, phys, entry as u32);
    win_wr32(p, phys + 4, (entry >> 32) as u32);
}

/// Build the VER2 tree that maps [`TEST_VA`] → `leaf`, **through the window**, and return
/// the root `PDE3[0]` value the guest would read back out of its own directory.
fn build_bar2_tree(p: &RegPlane, leaf_entry: u64) -> u64 {
    // PD2[(va>>38) & 511] → PD1
    win_wr_entry(p, PD2_TBL + ((TEST_VA >> 38) & 511) * 8, pde_vid(PD1_TBL));
    // PD1[(va>>29) & 511] → PD0   ⚠ no valid bit: on GA10x this slot could BE a 512 MiB
    // leaf, and the format has to tell the two apart off bit 0.
    win_wr_entry(p, PD1_TBL + ((TEST_VA >> 29) & 511) * 8, pde_vid(PD0_TBL));
    // PD0[(va>>21) & 255] → the SMALL-page table, in the HIGH qword of the 16-byte dual
    // entry. The low qword stays zero: no big-page sub-table.
    let slot = PD0_TBL + ((TEST_VA >> 21) & 255) * 16;
    win_wr_entry(p, slot, 0);
    win_wr_entry(p, slot + 8, dual_small_vid(PT_TBL));
    // PT_SMALL[(va>>12) & 511] → the leaf.
    win_wr_entry(p, PT_TBL + ((TEST_VA >> 12) & 511) * 8, leaf_entry);
    pde_vid(PD2_TBL)
}

/// The wire body of one `UpdateBarPde_v15_00`: `barType` u32, four bytes of padding,
/// `entryValue` u64, `entryLevelShift` u64.
fn update_bar_pde_body(bar_type: u32, entry: u64, level_shift: u64) -> Vec<u8> {
    let mut b = vec![0u8; 24];
    b[0..4].copy_from_slice(&bar_type.to_le_bytes());
    b[8..16].copy_from_slice(&entry.to_le_bytes());
    b[16..24].copy_from_slice(&level_shift.to_le_bytes());
    b
}

/// The big-page table this file uses for the `also` half of a dual entry.
const PT_BIG_TBL: u64 = 0x0100_4000;

/// The high qword of a dual `PD0` entry, but naming a **big**-page sub-table: a different
/// bit range and a different shift (`_ADDRESS_BIG_*`, shift **8**).
fn dual_big_vid(next: u64) -> u64 {
    ((next >> 8) << 4) | (1 << 1)
}

/// Push the root through the **real chain link**, into the plane's own latch.
///
/// ⊘ Not `BarPdeLog::publish`. The point is that the bytes the guest sends decode, that the
/// chain reaches the link at all, and that the link answers — three things a direct
/// `publish` would skip.
fn publish_root(p: &RegPlane, entry: u64) {
    publish_root_at(p, entry, 47);
}

/// The same, with the root's level shift chosen by the caller.
fn publish_root_at(p: &RegPlane, entry: u64, level_shift: u64) {
    let driver = *versions::table_for(BENCH_DRIVER).expect("the bench driver has a table");
    let mut link = kayfabe_device::bar2::BarPdePolicy::new(driver, p.bar_pde_log());
    let reply = link
        .respond(&RpcCommand {
            function: RpcFunction::UpdateBarPde,
            code: kayfabe_abi::generated::rpc::NV_VGPU_MSG_FUNCTION_UPDATE_BAR_PDE,
            sequence: 7,
            // `pRootFmt->virtAddrBitLo` for VER2's root is 47.
            payload: update_bar_pde_body(1, entry, level_shift),
            elements: 1,
            delivered: Vec::new(),
        })
        .expect("the link answers fn 70");
    assert_eq!(reply.rpc_result, 0, "NV_OK");
    assert!(
        reply.body.is_empty(),
        "⊘ nothing of the guest's request comes back — this command has no [OUT] field"
    );
}

// =====================================================================================
// 1. ★★★ THE RUNG — kbusVerifyBar2's MMU sub-test, both directions
// =====================================================================================

/// ★★★ **The whole statement.** Sixteen bytes written through the translated aperture read
/// back, byte for byte, through the untranslated one — and then the reverse.
#[test]
fn the_mmu_subtest_agrees_about_every_byte_in_both_directions() {
    let p = plane();
    let root = build_bar2_tree(&p, pte_vid(TEST_PHYS));
    publish_root(&p, root);

    // (1) `for (index = 0; index < testMemorySize; index += 4) MEM_WR32(pOffset+index, …)`
    for i in (0..FBSIZETESTED).step_by(4) {
        let w = p.write(BAR_INST, TEST_VA + i, 4, u64::from(SAMPLEDATA));
        assert_eq!(
            w.fb_landed,
            Some(TEST_PHYS + i),
            "★★★ the translated write must LAND, at the address the walk produced"
        );
        assert!(w.bar2_refusal.is_none() && w.fault.is_none());
    }

    // (2) `bar0WindowData = GPU_REG_RD32(temp + index)` — the guest's own judge.
    for i in (0..FBSIZETESTED).step_by(4) {
        assert_eq!(
            win_rd32(&p, TEST_PHYS + i),
            SAMPLEDATA,
            "★★★ 'MMUTest BAR0 window offset … returned garbage' — the guest's own \
             error message, at byte {i}"
        );
    }

    // (3) `GPU_REG_WR32(temp + index, SAMPLEDATA + 0x10)` — the other direction.
    for i in (0..FBSIZETESTED).step_by(4) {
        win_wr32(&p, TEST_PHYS + i, SAMPLEDATA + 0x10);
    }

    // (4) `temp = MEM_RD32(pOffset + index)` — read back through BAR2.
    for i in (0..FBSIZETESTED).step_by(4) {
        let r = p.read(BAR_INST, TEST_VA + i, 4);
        assert_eq!(
            r.value(),
            u64::from(SAMPLEDATA + 0x10),
            "★★★ 'MMUTest BAR2 Read of virtual addr … returned garbage', at byte {i}"
        );
        assert!(
            matches!(
                r,
                ReadOutcome::Fb {
                    window: FbWindow::InstanceWindow,
                    phys,
                    ..
                } if phys == TEST_PHYS + i
            ),
            "and it must be served as framebuffer, at the translated address"
        );
    }

    let c = p.counters();
    assert_eq!(c.bar2_writes, 4, "four dwords through the GMMU");
    assert_eq!(c.bar2_reads, 4);
    assert_eq!(c.bar2_faults, 0, "nothing was refused");
    assert_eq!(
        p.bar_pdes().bar2.map(|r| r.entry),
        Some(root),
        "the latched root is the entry the guest published"
    );
}

/// ★★★ **ONE STORE, and the test that a second one would fail.**
///
/// The tree, the page-table pages and the data page are all in the *same* [`SparseFb`], and
/// the way to see it is a page count: a device with a second store behind the translated
/// aperture would hold **more** pages than the window ever wrote.
#[test]
fn the_translated_aperture_and_the_window_share_one_framebuffer_store() {
    let p = plane();
    let root = build_bar2_tree(&p, pte_vid(TEST_PHYS));
    publish_root(&p, root);

    // Five 4 KiB pages have been touched by the window: four table pages and none of the
    // data page yet (the tree writes never touch TEST_PHYS).
    let before = p.residue().fb_resident_bytes;
    assert_eq!(
        before,
        4 * 4096,
        "four page-table pages, written through PRAMIN"
    );

    // One translated write. If it went anywhere else, this number would not move.
    p.write(BAR_INST, TEST_VA, 4, 0x1234_5678);
    assert_eq!(
        p.residue().fb_resident_bytes,
        before + 4096,
        "★★★ the translated write allocated a page IN THE SAME STORE — a second store \
         would have left this unchanged"
    );
    assert_eq!(
        win_rd32(&p, TEST_PHYS),
        0x1234_5678,
        "and the untranslated window reads the byte the translated write left"
    );
}

/// ★★ The walk really does read the tree **out of the store**, not out of anything it kept:
/// re-pointing the leaf through the window changes where the next translated access lands.
#[test]
fn re_pointing_a_leaf_through_the_window_moves_the_next_translated_access() {
    let p = plane();
    let root = build_bar2_tree(&p, pte_vid(TEST_PHYS));
    publish_root(&p, root);
    p.write(BAR_INST, TEST_VA, 4, 0xAAAA_AAAA);
    assert_eq!(win_rd32(&p, TEST_PHYS), 0xAAAA_AAAA);

    // Re-point the leaf at a different page, through the window, exactly as the guest
    // would. ⊘ Nothing is invalidated: there is no TLB here, and the walk is per access.
    let other = TEST_PHYS + 0x10_0000;
    win_wr_entry(&p, PT_TBL + ((TEST_VA >> 12) & 511) * 8, pte_vid(other));
    p.write(BAR_INST, TEST_VA, 4, 0xBBBB_BBBB);

    assert_eq!(win_rd32(&p, other), 0xBBBB_BBBB, "the new page took it");
    assert_eq!(
        win_rd32(&p, TEST_PHYS),
        0xAAAA_AAAA,
        "★ and the old page still holds what it held — the write did not go to both"
    );
}

/// ★★ The offset within the page is carried from the leaf's own **size**, so an access that
/// is not page-aligned lands where it should.
#[test]
fn the_offset_within_the_leaf_page_comes_from_the_leafs_own_size() {
    let p = plane();
    let root = build_bar2_tree(&p, pte_vid(TEST_PHYS));
    publish_root(&p, root);
    for off in [0u64, 4, 0x100, 0xFFC] {
        p.write(BAR_INST, TEST_VA + off, 4, 0x1000_0000 + off);
        assert_eq!(
            win_rd32(&p, TEST_PHYS + off),
            0x1000_0000 + off as u32,
            "at page offset {off:#x}"
        );
    }
}

// =====================================================================================
// 2. ⊘ The refusals — every one named, none of them a zero
// =====================================================================================

/// ★★★ Before the guest publishes a root there is **nothing to walk**, and the aperture
/// says so by name rather than reading zero or identity-mapping.
///
/// ⊘ Identity is the tempting wrong answer: the C artifact falls back to it whenever its
/// snooped `bar2_virtual` flag is clear (`C: nvkvm_gpu_emul.c:6588`). An identity aperture
/// would put `kbusVerifyBar2`'s write at framebuffer address `TEST_VA` — a real, writable,
/// completely wrong page — and the guest's read-back would find zero with no other symptom.
#[test]
fn an_unrooted_aperture_refuses_by_name_and_never_identity_maps() {
    let p = plane();
    let r = p.read(BAR_INST, TEST_VA, 4);
    assert!(matches!(r, ReadOutcome::TranslationRefused { va, why, .. }
                 if va == TEST_VA && why == BAR2_UNROOTED));
    let w = p.write(BAR_INST, TEST_VA, 4, 0xDEAD_BEEF);
    assert_eq!(w.fault, Some(BAR2_WRITE_REFUSED));
    assert_eq!(w.bar2_refusal.map(|r| r.why), Some(BAR2_UNROOTED));
    assert!(w.fb_landed.is_none());
    assert_eq!(
        p.residue().fb_resident_bytes,
        0,
        "★★★ and NOTHING was written anywhere — an identity fallback would have \
         allocated a page here"
    );
    assert_eq!(p.counters().bar2_faults, 2);
}

/// ⊘ A device whose shell never installed a page-table format says **that**, and not
/// "unmapped" — a wiring question and a guest-behaviour question are different findings.
#[test]
fn a_plane_with_no_page_table_format_names_the_missing_port() {
    let p = RegPlane::new(
        &GA106,
        abi::gsp_abi_for(BENCH_DRIVER).expect("bench table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("servable");
    p.set_fb(Box::new(SparseFb::new(GA106.fb_length)));
    // ⊘ deliberately no `set_mmu`
    publish_root(&p, pde_vid(PD2_TBL));
    let r = p.read(BAR_INST, TEST_VA, 4);
    assert!(matches!(r, ReadOutcome::TranslationRefused { why, .. } if why == NO_MMU_PORT));
}

/// ★★ A virtual address the guest's own tables do not map is a **miss**, and a miss is a
/// fault — `mode2_address_table.md`, arriving from the page tables rather than the table.
#[test]
fn an_unmapped_virtual_address_is_a_loud_miss() {
    let p = plane();
    let root = build_bar2_tree(&p, pte_vid(TEST_PHYS));
    publish_root(&p, root);
    // One page along: same tables, an empty leaf slot.
    let r = p.read(BAR_INST, TEST_VA + 0x1000, 4);
    let ReadOutcome::TranslationRefused { va, why, .. } = r else {
        panic!("an unmapped address must not resolve: {r:?}");
    };
    assert_eq!(va, TEST_VA + 0x1000);
    assert!(
        why.contains("map nothing"),
        "the sentence must say the tables map nothing, got {why:?}"
    );
}

/// ★★ A range the guest **declared** sparse is a different answer from one it never wrote.
#[test]
fn a_sparse_declaration_is_reported_as_a_declaration() {
    let p = plane();
    let root = build_bar2_tree(&p, pte_vid(TEST_PHYS));
    publish_root(&p, root);
    // `kgmmuFmtFamiliesInit_GM200`'s sparse PTE: valid clear, volatile set.
    win_wr_entry(&p, PT_TBL + (((TEST_VA + 0x1000) >> 12) & 511) * 8, 1 << 3);
    let ReadOutcome::TranslationRefused { why, .. } = p.read(BAR_INST, TEST_VA + 0x1000, 4) else {
        panic!("sparse does not resolve");
    };
    assert!(
        why.contains("SPARSE"),
        "sparse and unmapped must not collapse into one sentence, got {why:?}"
    );
}

/// ⊘ A leaf in an aperture this port does not serve through the bus window is refused **by
/// name**, never answered out of the framebuffer store.
///
/// ★★ This is the aliasing rule `kayfabe_arch::Aperture` exists for: sysmem offset `X` and
/// vidmem offset `X` are different bytes, and answering one for the other is a wrong page
/// handed to a guest with no error anywhere.
#[test]
fn a_system_memory_leaf_is_refused_and_not_answered_out_of_the_framebuffer() {
    let p = plane();
    let root = build_bar2_tree(&p, pte_sys(TEST_PHYS));
    publish_root(&p, root);
    let ReadOutcome::TranslationRefused { why, .. } = p.read(BAR_INST, TEST_VA, 4) else {
        panic!("a sysmem leaf must not be served from the framebuffer");
    };
    assert_eq!(why, BAR2_FOREIGN_APERTURE);
    let w = p.write(BAR_INST, TEST_VA, 4, 1);
    assert_eq!(w.bar2_refusal.map(|r| r.why), Some(BAR2_FOREIGN_APERTURE));
    assert_eq!(
        p.residue().fb_resident_bytes,
        4 * 4096,
        "only the four table pages — nothing was written at the sysmem address"
    );
}

/// ★★★ The guest published **one** root slot — its own. An address indexing another one
/// belongs to the firmware's half of the address space
/// (`ogkm-580: kern_bus.c:810-820`), and this port has published none of its own.
#[test]
fn an_address_outside_the_one_published_root_slot_is_refused() {
    let p = plane();
    let root = build_bar2_tree(&p, pte_vid(TEST_PHYS));
    publish_root(&p, root);
    // Root slot 1 — `PDE3[1]`, virtual address bit 47.
    let va = (1u64 << 47) | TEST_VA;
    let ReadOutcome::TranslationRefused { why, .. } = p.read(BAR_INST, va, 4) else {
        panic!("the firmware's own half must not resolve against the guest's root");
    };
    assert_eq!(why, BAR2_OUTSIDE_PUBLISHED_SLOT);
}

/// ★★ A **write** to a mapping the guest itself marked read-only does not land. Reads of it
/// still do — the two are different rights and only one is refused.
#[test]
fn a_write_to_a_read_only_leaf_is_refused_and_a_read_of_it_is_not() {
    let p = plane();
    // `NV_MMU_VER2_PTE_READ_ONLY`, bit 6.
    let root = build_bar2_tree(&p, pte_vid(TEST_PHYS) | (1 << 6));
    publish_root(&p, root);
    win_wr32(&p, TEST_PHYS, 0x5555_5555);

    assert_eq!(
        p.read(BAR_INST, TEST_VA, 4).value(),
        0x5555_5555,
        "a read-only mapping still READS"
    );
    let w = p.write(BAR_INST, TEST_VA, 4, 0xFFFF_FFFF);
    assert_eq!(w.bar2_refusal.map(|r| r.why), Some(BAR2_READ_ONLY));
    assert_eq!(
        win_rd32(&p, TEST_PHYS),
        0x5555_5555,
        "★ and the byte is untouched — a refusal that still wrote would be the worst of both"
    );
}

/// ⊘ A guest-built **cycle** in its own page tables terminates. The tables are guest bytes
/// and a directory may point at itself; a walker that followed it would never return, from
/// inside a vCPU's MMIO callback.
#[test]
fn a_page_directory_that_points_at_itself_terminates() {
    let p = plane();
    win_wr_entry(&p, PD2_TBL + ((TEST_VA >> 38) & 511) * 8, pde_vid(PD2_TBL));
    publish_root(&p, pde_vid(PD2_TBL));
    let r = p.read(BAR_INST, TEST_VA, 4);
    assert!(
        matches!(r, ReadOutcome::TranslationRefused { .. }),
        "a cycle must refuse rather than hang: {r:?}"
    );
}

// =====================================================================================
// 3. The publication latch itself
// =====================================================================================

/// ★★ `entryValue = 0` is a **real message** — the guest publishes it to unroot the
/// aperture on teardown (`ogkm-580: kern_bus_gm107.c:2137`). It must be latched, and it
/// must make every access miss.
#[test]
fn publishing_a_zero_root_unroots_the_aperture_rather_than_being_ignored() {
    let p = plane();
    let root = build_bar2_tree(&p, pte_vid(TEST_PHYS));
    publish_root(&p, root);
    assert!(p.write(BAR_INST, TEST_VA, 4, 1).fb_landed.is_some());

    publish_root(&p, 0);
    assert_eq!(
        p.bar_pdes().bar2.map(|r| r.entry),
        Some(0),
        "latched, not ignored"
    );
    assert_eq!(p.bar_pde_counts().0, 2, "★ two publications, not one");
    assert!(matches!(
        p.read(BAR_INST, TEST_VA, 4),
        ReadOutcome::TranslationRefused { .. }
    ));
}

/// ★★ A malformed body is refused **and not latched** — a root taken from a body that could
/// not carry one would root the aperture at whatever the padding happened to hold.
#[test]
fn a_short_or_unknown_body_is_refused_and_publishes_nothing() {
    let p = plane();
    let driver = *versions::table_for(BENCH_DRIVER).expect("bench table");
    let mut link = kayfabe_device::bar2::BarPdePolicy::new(driver, p.bar_pde_log());
    let cmd = |payload: Vec<u8>| RpcCommand {
        function: RpcFunction::UpdateBarPde,
        code: kayfabe_abi::generated::rpc::NV_VGPU_MSG_FUNCTION_UPDATE_BAR_PDE,
        sequence: 3,
        payload,
        elements: 1,
        delivered: Vec::new(),
    };
    // Short: the entry and its level cannot both be in it.
    assert_ne!(link.respond(&cmd(vec![0u8; 12])).unwrap().rpc_result, 0);
    // `NV_RPC_UPDATE_PDE_BAR_INVALID` — a value the enum defines and nobody should send.
    assert_ne!(
        link.respond(&cmd(update_bar_pde_body(2, pde_vid(PD2_TBL), 47)))
            .unwrap()
            .rpc_result,
        0
    );
    assert_eq!(p.bar_pdes(), Default::default(), "nothing was latched");
    assert_eq!(p.bar_pde_counts(), (0, 2), "two refusals, no publications");
}

/// ★ BAR1's root is recorded too, even though nothing translates that window yet — the
/// guest publishes both through the same command, and recording only one would make *"did
/// the guest ever publish a BAR1 root?"* unanswerable from a boot log.
#[test]
fn the_bar1_root_is_recorded_even_though_nothing_translates_that_window() {
    let p = plane();
    let driver = *versions::table_for(BENCH_DRIVER).expect("bench table");
    let mut link = kayfabe_device::bar2::BarPdePolicy::new(driver, p.bar_pde_log());
    link.respond(&RpcCommand {
        function: RpcFunction::UpdateBarPde,
        code: kayfabe_abi::generated::rpc::NV_VGPU_MSG_FUNCTION_UPDATE_BAR_PDE,
        sequence: 5,
        payload: update_bar_pde_body(0, pde_vid(PD2_TBL), 47),
        elements: 1,
        delivered: Vec::new(),
    })
    .expect("answered");
    assert_eq!(p.bar_pdes().bar1.map(|r| r.entry), Some(pde_vid(PD2_TBL)));
    assert_eq!(p.bar_pdes().bar2, None, "and it did NOT root BAR2");
    assert!(
        matches!(
            p.read(kayfabe_abi::pcibars::bus_bar::FB as u8, 0x9008C, 4),
            ReadOutcome::FbWindow(FbWindow::FbAperture)
        ),
        "⊘ BAR1 still has no address model; recording its root is not serving it"
    );
}

/// ★★★ A device reset **forgets the root**, and the reason is not tidiness: a root that
/// survived a device life would send the next guest's aperture accesses into the previous
/// guest's page tables.
#[test]
fn a_device_reset_forgets_the_published_root() {
    let p = plane();
    let root = build_bar2_tree(&p, pte_vid(TEST_PHYS));
    publish_root(&p, root);
    assert!(p.write(BAR_INST, TEST_VA, 4, 1).fb_landed.is_some());

    p.device_reset();

    assert_eq!(p.bar_pdes(), Default::default());
    assert_eq!(p.bar_pde_counts(), (0, 0));
    assert!(
        matches!(
            p.read(BAR_INST, TEST_VA, 4),
            ReadOutcome::TranslationRefused { why, .. } if why == BAR2_UNROOTED
        ),
        "★★★ and the aperture is unrooted, not still pointing at the last guest's tree"
    );
    assert_eq!(
        p.residue().fb_resident_bytes,
        0,
        "…and the framebuffer the tree lived in is gone too"
    );
}

/// ★★ The latch is in the **residue**, so `#130`'s "indistinguishable from first boot" is
/// quantified over it rather than over a list somebody remembered to extend.
#[test]
fn the_published_root_is_part_of_the_residue() {
    let cold = plane().residue();
    let p = plane();
    publish_root(&p, pde_vid(PD2_TBL));
    assert_ne!(
        p.residue().bar_pdes,
        cold.bar_pdes,
        "a device that has been given a root differs from a cold one"
    );
    p.device_reset();
    assert_eq!(p.residue().bar_pdes, cold.bar_pdes);
}

/// ⊘ A shared handle, so a shell that replaces the whole command chain can keep publishing
/// into the same latch. Without it, `set_policy` would silently unroot the aperture.
#[test]
fn the_publication_latch_survives_the_command_chain_being_replaced() {
    let p = plane();
    let root = build_bar2_tree(&p, pte_vid(TEST_PHYS));
    let log: BarPdeLog = p.bar_pde_log();
    let driver = *versions::table_for(BENCH_DRIVER).expect("bench table");
    p.set_policy(Box::new(kayfabe_device::bar2::BarPdePolicy::new(
        driver,
        log.clone(),
    )));
    publish_root(&p, root);
    assert!(
        p.write(BAR_INST, TEST_VA, 4, 0x99).fb_landed.is_some(),
        "the plane reads the same latch the replacement chain writes"
    );
}

/// ★★★ **The `also` half is FOLLOWED, not merely decoded** — a mapping that lives under the
/// dual entry's SECOND sub-table resolves.
///
/// The small-page sub-table is present and **empty**, so a walker that stopped at
/// [`kayfabe_arch::PteDecode::Pde::edge`] would report this address unmapped and be
/// perfectly self-consistent about it. That is `#13`'s shape at the 16-byte level, and it
/// is the reason the point query tries both halves rather than the first.
#[test]
fn a_big_page_mapping_under_the_dual_entrys_second_sub_table_resolves() {
    let p = plane();
    // The upper directories, as usual.
    win_wr_entry(&p, PD2_TBL + ((TEST_VA >> 38) & 511) * 8, pde_vid(PD1_TBL));
    win_wr_entry(&p, PD1_TBL + ((TEST_VA >> 29) & 511) * 8, pde_vid(PD0_TBL));
    // A dual entry with BOTH halves live: an empty small table, and a big table that maps
    // the address.
    let slot = PD0_TBL + ((TEST_VA >> 21) & 255) * 16;
    win_wr_entry(&p, slot, dual_big_vid(PT_BIG_TBL));
    win_wr_entry(&p, slot + 8, dual_small_vid(PT_TBL));
    // A 64 KiB leaf. The page offset is sixteen bits wide, not twelve.
    let big_page = 0x2_EFBA_0000u64;
    win_wr_entry(
        &p,
        PT_BIG_TBL + ((TEST_VA >> 16) & 31) * 8,
        pte_vid(big_page),
    );
    publish_root(&p, pde_vid(PD2_TBL));

    let w = p.write(BAR_INST, TEST_VA, 4, 0xC0FF_EE00);
    assert_eq!(
        w.fb_landed,
        Some(big_page + (TEST_VA & 0xFFFF)),
        "★★★ the write must land under the BIG-page leaf, at a 64 KiB page offset"
    );
    assert_eq!(win_rd32(&p, big_page + (TEST_VA & 0xFFFF)), 0xC0FF_EE00);
}

/// ⊘ A root published at a level shift this chip's format has no level for is refused **by
/// name**, not decoded at a guessed level.
///
/// ★★ The guest sends `pRootFmt->virtAddrBitLo` beside the entry
/// (`ogkm-580: kern_bus.c:880`) precisely because the eight bytes alone do not say which
/// format row they belong to — and a dual entry and a single one do not share a layout, so
/// decoding at the wrong level reads the wrong fields and yields a plausible address.
#[test]
fn a_root_published_at_a_level_shift_this_format_does_not_have_is_refused() {
    let p = plane();
    let root = build_bar2_tree(&p, pte_vid(TEST_PHYS));
    // 33 is not any VER2 level's `virtAddrBitLo`.
    publish_root_at(&p, root, 33);
    let ReadOutcome::TranslationRefused { why, .. } = p.read(BAR_INST, TEST_VA, 4) else {
        panic!("a root at an unknown level must not resolve");
    };
    assert_eq!(why, BAR2_UNKNOWN_ROOT_LEVEL);
}
