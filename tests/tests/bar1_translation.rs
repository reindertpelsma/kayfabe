//! ★★★★ **The framebuffer aperture, translated** — §16.18, and the acceptance is
//! **cross-aperture, never round-trip**.
//!
//! # The measurement this file is written against
//!
//! Boot `s17_e8fde62`, 2026-08-09, with no shadow memslot installed, so the BAR1 census was
//! complete for the whole boot:
//!
//! ```text
//! nvkvm: BAR1 (flat aperture): 3 accesses reached the DISCARDING fallback …
//!   BAR1[0] WRITE off=0x90000 size=4 val=0x20000000
//!   BAR1[1] WRITE off=0x90004 size=4 val=0x2801
//!   BAR1[2] WRITE off=0xa008c size=4 val=0x1
//! ```
//!
//! `[0]`+`[1]` are one qword — `0x0000_2801_2000_0000`, a GPFIFO entry naming a pushbuffer
//! at `gpu_va=0x1_2000_0000` of 40 bytes — and `[2]` is `GP_PUT = 1` at `USERD + 0x8c`.
//! That is `internal_channel_submit_work` verbatim
//! (`ogkm-580: kernel-open/nvidia-uvm/uvm_channel.c:984-1015`): `set_gpfifo_entry` through a
//! **dereferenced CPU pointer**, `mb()`, `write_gpu_put`. The guest's entire submission
//! handshake is three BAR1 writes, and `nvkvm_reservation_write` discarded all three.
//!
//! ★ Three accesses read as *"the guest barely touched BAR1"*. Three accesses **are** the
//! whole event. A small count is not a small event.
//!
//! # ★★★★ THE ROOT COMES FROM THE OTHER DIRECTION, and that is the finding
//!
//! BAR2's root is a fact the **guest sends us**: an eight-byte entry value on
//! `UPDATE_BAR_PDE(NV_RPC_UPDATE_PDE_BAR_2)`. Before this rung, `plane.rs` carried a comment
//! saying BAR1's root *"arrives through the same command"*, `bar2.rs` carried a
//! `BarPdes::bar1` field to latch it, and `#[derive]`-level plumbing existed the whole way
//! down. ⊘ **All of it was for a command that does not exist.**
//!
//! `NV_RM_RPC_UPDATE_BAR_PDE` has exactly **two** call sites in the whole of `ogkm-580` —
//! `src/nvidia/src/kernel/gpu/bus/kern_bus.c:880` and
//! `src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:2137` — and **both pass
//! `NV_RPC_UPDATE_PDE_BAR_2`**. Nothing anywhere passes `NV_RPC_UPDATE_PDE_BAR_1`.
//!
//! What happens instead is `kbusPatchBar1Pdb_GSPCLIENT` (`ogkm-580: kern_bus.c:755-807`):
//! the guest reads **`pGSCI->bar1PdeBase`** out of the `GET_GSP_STATIC_INFO` reply *we*
//! wrote, describes a framebuffer memory descriptor over it, and calls
//! `mmuWalkModifyLevelInstance` to re-root **its own** BAR1 walker onto that address. From
//! then on the guest writes BAR1's page tables into our framebuffer, at an address we chose,
//! and never mentions it again.
//!
//! ⇒ The loop this file closes is **publish → the guest reads → the guest builds there → we
//! walk there**, and [`the_root_the_walk_uses_is_the_one_the_guest_was_TOLD`] is the test
//! that makes a drift between the two ends fail loudly instead of resolving to a plausible
//! wrong address.
//!
//! # ⊘⊘ WHY EVERY ASSERTION HERE IS CROSS-APERTURE
//!
//! The trap this rung had to avoid is not *"BAR1 has nowhere to put bytes"*. It is
//! **giving BAR1 somewhere of its own**. A private store, or the right store at the wrong
//! offset, is *self-consistent*: a read back through the same function agrees with the
//! write, so **read-after-write cannot detect it**. `Bar0Window::target()` — decoded and
//! never consulted — is the standing warning about doing half of this.
//!
//! So no test in this file ever writes through BAR1 and reads through BAR1. Every one
//! writes through one aperture and reads through **a different one that resolves the address
//! by a different mechanism**: the BAR0 moving window (untranslated arithmetic), and BAR2
//! (translated, but rooted at an entry the guest published rather than at an address we
//! published).
//!
//! # ⊘ What this file does NOT establish
//!
//! - ⊘ **It is not a boot.** `only_live_boots_are_proof` stands.
//! - ⊘ It says nothing about whether the guest's ring, once delivered, is *executed*. Q5
//!   stands: the completion negative control came back negative, and getting bytes to the
//!   right framebuffer address does not license writing a semaphore we never observed an
//!   engine write.

use kayfabe_abi::versions::BENCH_DRIVER;
use kayfabe_device::plane::{
    BAR1_FOREIGN_APERTURE, BAR1_READ_ONLY, BAR1_WRITE_REFUSED, NanoClock, ReadOutcome, RegPlane,
    SteppingClock,
};
use kayfabe_device::staticinfo::StaticInfoPolicy;
use kayfabe_device::{FbWindow, SparseFb, abi, ga10x::GA106};

// ───────────────────────────── the chip's own geometry ─────────────────────────────

fn win_reg() -> u64 {
    GA106.bar0_window_reg
}

fn pramin() -> u64 {
    GA106.pramin_window.base
}

/// RM's logical index for the framebuffer aperture — the one this rung teaches to translate.
const BAR_FB: u8 = kayfabe_abi::pcibars::bus_bar::FB as u8;
/// …the instance/`BAR2` window…
const BAR_INST: u8 = kayfabe_abi::pcibars::bus_bar::INST as u8;
/// …and the register aperture, which carries the untranslated moving window.
const BAR_REGS: u8 = kayfabe_abi::pcibars::bus_bar::REGS as u8;

/// The three BAR1 offsets boot `s17_e8fde62` measured, and the two values of the GPFIFO
/// entry. Used verbatim so this file exercises the arithmetic that was actually lost.
const GPFIFO_OFF: u64 = 0x0009_0000;
const GPFIFO_LO: u32 = 0x2000_0000;
const GPFIFO_HI: u32 = 0x0000_2801;
const GP_PUT_OFF: u64 = 0x000A_008C;

/// Where this file puts the framebuffer pages the aperture maps onto. Any pages the chip
/// backs will do; these are two distinct ones so a test can tell them apart.
const RING_PHYS: u64 = 0x0200_0000;
const USERD_PHYS: u64 = 0x0200_1000;

/// Where the BAR1 page tables go. ⊘ These are chosen by the test **because in a real boot
/// they are chosen by the guest's own allocator** — the only address this port gets to
/// decide is the root, and that one comes from the chip row, never from here.
const B1_PD1: u64 = 0x0300_1000;
const B1_PD0: u64 = 0x0300_2000;
const B1_PT: u64 = 0x0300_3000;

/// A second, entirely separate tree, for the BAR2 aperture in the cross-aperture test.
const B2_PD2: u64 = 0x0400_0000;
const B2_PD1: u64 = 0x0400_1000;
const B2_PD0: u64 = 0x0400_2000;
const B2_PT: u64 = 0x0400_3000;
/// The BAR2 virtual address that will be pointed at the *same* framebuffer page BAR1 maps.
const B2_VA: u64 = 0x0021_0000;

// ───────────────────────────── VER2 entry encodings ─────────────────────────────
//
// ⊘ Deliberately re-stated here rather than shared with `bar2_translation.rs`. If the two
// files shared an encoder and the encoder were wrong, both would agree and neither would
// notice — the `two_encodings_agreeing_on_the_first_values` shape.

fn pde_vid(next: u64) -> u64 {
    ((next >> 12) << 8) | (1 << 1)
}

fn dual_small_vid(next: u64) -> u64 {
    ((next >> 12) << 8) | (1 << 1)
}

fn pte_vid(phys: u64) -> u64 {
    ((phys >> 12) << 8) | 1
}

/// A leaf naming **system** memory — an aperture this port refuses to serve through a bus
/// window.
fn pte_sys(phys: u64) -> u64 {
    ((phys >> 12) << 8) | (2 << 1) | 1
}

/// A leaf the guest marked read-only (`_PTE_READ_ONLY`, bit 6).
fn pte_vid_ro(phys: u64) -> u64 {
    pte_vid(phys) | (1 << 6)
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

fn point_window(p: &RegPlane, phys: u64) {
    let cur = p.read(BAR_REGS, win_reg(), 4).value() as u32;
    let base = u32::try_from(phys >> 16).expect("a 24-bit window base");
    let val = (cur & !0x00FF_FFFF) | (base & 0x00FF_FFFF);
    p.write(BAR_REGS, win_reg(), 4, u64::from(val));
}

/// Write one dword **through the BAR0 moving window**.
///
/// ★ This is the only way this file ever puts a page-table byte in the framebuffer, and it
/// is what makes the assertions non-vacuous: the tree the walk reads was written by the
/// same hands a guest uses, through an aperture that resolves addresses by arithmetic
/// rather than by a walk.
fn win_wr32(p: &RegPlane, phys: u64, val: u32) {
    point_window(p, phys);
    let w = p.write(BAR_REGS, pramin() + (phys & 0xFFFF), 4, u64::from(val));
    assert_eq!(
        w.fb_landed,
        Some(phys),
        "the window write must land, and say where"
    );
}

fn win_rd32(p: &RegPlane, phys: u64) -> u32 {
    point_window(p, phys);
    p.read(BAR_REGS, pramin() + (phys & 0xFFFF), 4).value() as u32
}

fn win_wr_entry(p: &RegPlane, phys: u64, entry: u64) {
    win_wr32(p, phys, entry as u32);
    win_wr32(p, phys + 4, (entry >> 32) as u32);
}

/// ★★★★ **The root this port publishes, read back out of the reply the guest reads it
/// from** — not out of the chip row directly.
///
/// ⊘ Taking it from `GA106.bar1_pde_base` would test that the walk uses the chip row, which
/// is not the question. The question is whether the walk uses the address **the guest was
/// told**, and the only statement of that is the encoded body of fn 65.
fn published_bar1_root() -> u64 {
    let body = StaticInfoPolicy::new(
        &GA106,
        *kayfabe_abi::versions::table_for(BENCH_DRIVER).expect("the bench driver has a table"),
    )
    .body()
    .expect("GA106's static info encodes");
    let off = kayfabe_abi::gspstaticinfo::BAR1_PDE_BASE_OFF;
    u64::from_le_bytes(body[off..off + 8].try_into().expect("eight bytes"))
}

/// Build the BAR1 tree that maps `va` → `leaf_entry`, **through the window**, rooted at the
/// address this device published.
///
/// ⊘ Unlike BAR2's, there is no root *entry* to return: the root is a real directory page at
/// a real framebuffer address, and slot `(va >> 47) & 511` of it is a page this test writes
/// like any other.
fn build_bar1_tree(p: &RegPlane, root: u64, va: u64, leaf_entry: u64) {
    win_wr_entry(p, root + ((va >> 47) & 511) * 8, pde_vid(B1_PD1));
    win_wr_entry(p, B1_PD1 + ((va >> 38) & 511) * 8, pde_vid(B1_PD0));
    // ⚠ The GA10x root level indexes at 47 and the next two at 38 and 29; PD0's dual entry
    // is 16 bytes and the SMALL-page sub-table lives in its HIGH qword.
    let pd0_parent = B1_PD0 + ((va >> 29) & 511) * 8;
    win_wr_entry(p, pd0_parent, pde_vid(B1_PD0 + 0x1000));
    let slot = (B1_PD0 + 0x1000) + ((va >> 21) & 255) * 16;
    win_wr_entry(p, slot, 0);
    win_wr_entry(p, slot + 8, dual_small_vid(B1_PT));
    win_wr_entry(p, B1_PT + ((va >> 12) & 511) * 8, leaf_entry);
}

/// The same for BAR2, whose root is an entry value the caller publishes.
fn build_bar2_tree(p: &RegPlane, va: u64, leaf_entry: u64) -> u64 {
    win_wr_entry(p, B2_PD2 + ((va >> 38) & 511) * 8, pde_vid(B2_PD1));
    win_wr_entry(p, B2_PD1 + ((va >> 29) & 511) * 8, pde_vid(B2_PD0));
    let slot = B2_PD0 + ((va >> 21) & 255) * 16;
    win_wr_entry(p, slot, 0);
    win_wr_entry(p, slot + 8, dual_small_vid(B2_PT));
    win_wr_entry(p, B2_PT + ((va >> 12) & 511) * 8, leaf_entry);
    pde_vid(B2_PD2)
}

fn publish_bar2_root(p: &RegPlane, entry: u64) {
    let driver =
        *kayfabe_abi::versions::table_for(BENCH_DRIVER).expect("the bench driver has a table");
    let mut link = kayfabe_device::bar2::BarPdePolicy::new(driver, p.bar_pde_log());
    let mut body = vec![0u8; 24];
    body[0..4].copy_from_slice(&1u32.to_le_bytes());
    body[8..16].copy_from_slice(&entry.to_le_bytes());
    body[16..24].copy_from_slice(&47u64.to_le_bytes());
    let reply = kayfabe_gsp::CommandPolicy::respond(
        &mut link,
        &kayfabe_gsp::RpcCommand {
            function: kayfabe_gsp::RpcFunction::UpdateBarPde,
            code: kayfabe_abi::generated::rpc::NV_VGPU_MSG_FUNCTION_UPDATE_BAR_PDE,
            sequence: 7,
            payload: body,
            elements: 1,
            delivered: Vec::new(),
        },
    )
    .expect("the link answers fn 70");
    assert_eq!(reply.rpc_result, 0, "NV_OK");
}

// =====================================================================================
// 1. ★★★★ THE RUNG — the guest's own submission handshake, and the walker reads it back
// =====================================================================================

/// ★★★★ **BAR1 writes it, the WALKER reads it.**
///
/// The three writes boot `s17_e8fde62` measured, replayed at their measured offsets with
/// their measured values, and then read back **through the BAR0 moving window at the
/// framebuffer address the walk produced** — a different aperture, resolving by arithmetic
/// rather than by a page walk.
///
/// ⊘ There is no BAR1 read anywhere in this test, on purpose. A BAR1 read would agree with a
/// BAR1 write through any store at all, including a wrong one.
#[test]
fn the_submission_handshake_lands_where_the_walker_reads() {
    let p = plane();
    let root = published_bar1_root();
    assert_ne!(
        root, 0,
        "this chip row must publish a bar1PdeBase or the rung is untested"
    );
    build_bar1_tree(&p, root, GPFIFO_OFF, pte_vid(RING_PHYS));
    build_bar1_tree_second_page(&p, root, GP_PUT_OFF, pte_vid(USERD_PHYS));

    // BAR1[0] and BAR1[1] — the GPFIFO entry, in two dwords, exactly as
    // `set_gpfifo_entry` writes it through its CPU pointer.
    let w0 = p.write(BAR_FB, GPFIFO_OFF, 4, u64::from(GPFIFO_LO));
    assert_eq!(
        w0.fb_landed,
        Some(RING_PHYS),
        "★★★★ the first half of the GPFIFO entry must LAND, at the address the walk produced"
    );
    assert!(
        w0.fault.is_none() && w0.fb_window.is_none(),
        "and not be reported as dropped"
    );
    let w1 = p.write(BAR_FB, GPFIFO_OFF + 4, 4, u64::from(GPFIFO_HI));
    assert_eq!(w1.fb_landed, Some(RING_PHYS + 4));

    // BAR1[2] — GP_PUT, on the other page.
    let w2 = p.write(BAR_FB, GP_PUT_OFF, 4, 1);
    assert_eq!(w2.fb_landed, Some(USERD_PHYS + (GP_PUT_OFF & 0xFFF)));

    // ★★★★ And now the OTHER aperture. This is the whole acceptance.
    assert_eq!(win_rd32(&p, RING_PHYS), GPFIFO_LO);
    assert_eq!(win_rd32(&p, RING_PHYS + 4), GPFIFO_HI);
    assert_eq!(
        u64::from(win_rd32(&p, RING_PHYS)) | (u64::from(win_rd32(&p, RING_PHYS + 4)) << 32),
        0x0000_2801_2000_0000,
        "★ the qword the two dwords compose is the GPFIFO entry the boot measured"
    );
    assert_eq!(
        win_rd32(&p, USERD_PHYS + (GP_PUT_OFF & 0xFFF)),
        1,
        "GP_PUT = 1"
    );

    let c = p.counters();
    assert_eq!(c.bar1_writes, 3, "three writes served through the GMMU");
    assert_eq!(c.bar1_faults, 0);
    assert_eq!(
        c.fb_window_writes, 0,
        "⊘ and NONE reported as having no address model"
    );
}

/// The second BAR1 mapping needs its own leaf-level page so the two aperture offsets do not
/// share a page table slot. Split out so [`build_bar1_tree`] stays the one shape.
fn build_bar1_tree_second_page(p: &RegPlane, root: u64, va: u64, leaf_entry: u64) {
    // `GPFIFO_OFF` and `GP_PUT_OFF` are both under 2 MiB, so they share every level down to
    // the small-page table and differ only in its index.
    win_wr_entry(p, B1_PT + ((va >> 12) & 511) * 8, leaf_entry);
    let _ = root;
}

// =====================================================================================
// 2. ★★★★ THE ROOT IS THE ONE WE PUBLISHED — the loop, closed
// =====================================================================================

/// ★★★★ **The address the walk starts from is the address the guest was TOLD**, and a drift
/// between the two is what this test exists to fail on.
///
/// ⊘ This is the assertion that makes the rest of the file mean anything. Every other test
/// here builds its tree at `published_bar1_root()`, so if the plane walked from some *other*
/// address they would all fail — but they would fail as *"the aperture resolves nothing"*,
/// which is also what a hundred other defects look like. This one says which.
#[test]
fn the_root_the_walk_uses_is_the_one_the_guest_was_told() {
    let p = plane();
    let published = published_bar1_root();

    assert_eq!(
        published, GA106.bar1_pde_base,
        "★★★★ the byte at GspStaticConfigInfo+1664 IS the chip row's bar1_pde_base; if these \
         ever drift the guest builds its page tables somewhere we do not look, and every \
         aperture access resolves to a plausible wrong address rather than to an error"
    );

    // Build a tree ONLY at the published root, and map one page.
    build_bar1_tree(&p, published, GPFIFO_OFF, pte_vid(RING_PHYS));
    let w = p.write(BAR_FB, GPFIFO_OFF, 4, 0xDEAD_BEEF);
    assert_eq!(w.fb_landed, Some(RING_PHYS));

    // ★ And the bite: a tree at a DIFFERENT root, with nothing at the published one, must
    // resolve nothing. Without this, a walk that ignored the root entirely — say one that
    // scanned the framebuffer for a plausible directory — would pass the assertion above.
    let q = plane();
    build_bar1_tree(&q, published + 0x1_0000, GPFIFO_OFF, pte_vid(RING_PHYS));
    let bad = q.write(BAR_FB, GPFIFO_OFF, 4, 0xDEAD_BEEF);
    assert_eq!(
        bad.fb_landed, None,
        "a tree at the wrong root must not resolve"
    );
    assert_eq!(bad.fault, Some(BAR1_WRITE_REFUSED));
    assert_eq!(q.counters().bar1_faults, 1);
}

// =====================================================================================
// 3. ★★★★ ONE STORE, TWO TRANSLATED APERTURES, TWO DIFFERENT ROOT MECHANISMS
// =====================================================================================

/// ★★★★ **BAR1 writes it; BAR2 — rooted by a completely different mechanism — reads it.**
///
/// The two apertures are pointed at the **same framebuffer page** through **two disjoint
/// page-table trees**, one rooted at an address we published and one at an entry the guest
/// published. If either aperture reached a store of its own, or resolved the same page to a
/// different address, this cannot pass — and unlike a read-after-write, no single wrong
/// function can make both sides agree, because the two sides do not share one.
#[test]
fn bar1_writes_it_and_bar2_reads_it_out_of_the_same_store() {
    let p = plane();
    let root = published_bar1_root();
    build_bar1_tree(&p, root, GPFIFO_OFF, pte_vid(RING_PHYS));
    let b2root = build_bar2_tree(&p, B2_VA, pte_vid(RING_PHYS));
    publish_bar2_root(&p, b2root);

    // BAR1 → the page.
    let w = p.write(BAR_FB, GPFIFO_OFF + 8, 4, 0x1234_5678);
    assert_eq!(w.fb_landed, Some(RING_PHYS + 8));

    // BAR2 → the same page, at the same offset within it, through its own walk.
    let r = p.read(BAR_INST, B2_VA + 8, 4);
    assert_eq!(
        r.value(),
        0x1234_5678,
        "★★★★ the framebuffer aperture and the instance window must agree about one byte"
    );
    assert!(matches!(r, ReadOutcome::Fb { phys, .. } if phys == RING_PHYS + 8));

    // …and the reverse direction, which is the half `kbusVerifyBar2` also checks.
    let w2 = p.write(BAR_INST, B2_VA + 12, 4, 0x9ABC_DEF0);
    assert_eq!(w2.fb_landed, Some(RING_PHYS + 12));
    assert_eq!(win_rd32(&p, RING_PHYS + 12), 0x9ABC_DEF0);

    let c = p.counters();
    assert_eq!(c.bar1_writes, 1);
    assert_eq!(c.bar2_writes, 1);
    assert_eq!(c.bar2_reads, 1);
}

// =====================================================================================
// 4. ⊘ THE REFUSALS, BY A NAME THAT IS TRUE
// =====================================================================================

/// ⊘ An unmapped aperture offset is a **fault**, never a silent zero and never a success.
#[test]
fn an_unmapped_aperture_offset_is_refused_by_name() {
    let p = plane();
    let root = published_bar1_root();
    build_bar1_tree(&p, root, GPFIFO_OFF, pte_vid(RING_PHYS));

    // A virtual address under a different root slot: nothing maps it.
    let w = p.write(BAR_FB, 0x0100_0000_0000, 4, 1);
    assert_eq!(w.fb_landed, None);
    assert_eq!(
        w.fault,
        Some(BAR1_WRITE_REFUSED),
        "★ the fault names BAR1, not BAR2 — a refusal that named the wrong window would send \
         a reader to UPDATE_BAR_PDE for a root that command never carries"
    );
    let refusal = w
        .bar2_refusal
        .expect("a translated refusal carries its address");
    assert_eq!(
        refusal.va, 0x0100_0000_0000,
        "and the VIRTUAL address, not a physical one"
    );
    assert_eq!(p.counters().bar1_faults, 1);
    assert_eq!(
        p.counters().bar2_faults,
        0,
        "⊘ and it is NOT counted as a BAR2 fault"
    );
}

/// ⊘ A leaf naming system memory is refused rather than served out of the framebuffer,
/// which would alias two different memories onto one address.
#[test]
fn a_leaf_in_a_foreign_aperture_is_refused() {
    let p = plane();
    let root = published_bar1_root();
    build_bar1_tree(&p, root, GPFIFO_OFF, pte_sys(RING_PHYS));

    let w = p.write(BAR_FB, GPFIFO_OFF, 4, 1);
    assert_eq!(w.fb_landed, None);
    let refusal = w.bar2_refusal.expect("named");
    assert_eq!(refusal.why, BAR1_FOREIGN_APERTURE);
}

/// ⊘ A mapping the guest itself marked read-only stays read-only. A write through it would
/// hand the guest a stronger mapping than the one it published.
#[test]
fn a_read_only_leaf_refuses_the_write_but_serves_the_read() {
    let p = plane();
    let root = published_bar1_root();
    build_bar1_tree(&p, root, GPFIFO_OFF, pte_vid_ro(RING_PHYS));

    // Seed the page through the window so the read has something true to return.
    win_wr32(&p, RING_PHYS, 0xFEED_FACE);

    let w = p.write(BAR_FB, GPFIFO_OFF, 4, 1);
    assert_eq!(w.fb_landed, None);
    assert_eq!(w.bar2_refusal.expect("named").why, BAR1_READ_ONLY);

    let r = p.read(BAR_FB, GPFIFO_OFF, 4);
    assert_eq!(
        r.value(),
        0xFEED_FACE,
        "★ read-only refuses WRITES, not reads"
    );
    assert_eq!(
        win_rd32(&p, RING_PHYS),
        0xFEED_FACE,
        "and the byte never changed"
    );
}

/// ⊘ A chip row that declares **no** `bar1PdeBase` still gets the honest
/// *"no address model"* answer rather than a walk from framebuffer address zero.
///
/// ★ This is what keeps `WindowRefusal::NoAddressModel` a live arm rather than a variant
/// reachable from nowhere — and it is the arm the `fb_window_writes` counter in every boot
/// report is keyed on.
#[test]
fn a_chip_row_with_no_declared_root_has_no_address_model() {
    let mut rowless: kayfabe_device::ChipProfile = GA106;
    rowless.bar1_pde_base = 0;
    let rowless: &'static kayfabe_device::ChipProfile = Box::leak(Box::new(rowless));

    let p = RegPlane::new(
        rowless,
        abi::gsp_abi_for(BENCH_DRIVER).expect("wire table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("servable");
    p.set_fb(Box::new(SparseFb::new(rowless.fb_length)));
    p.set_mmu(Box::new(kayfabe_chips::Ga10xGmmu::new()));

    let w = p.write(BAR_FB, GPFIFO_OFF, 4, 1);
    assert_eq!(
        w.fb_window,
        Some(FbWindow::FbAperture),
        "reported as dropped, by window"
    );
    assert_eq!(w.fb_landed, None);
    assert!(
        w.fault.is_none(),
        "⊘ 'no model' is not a fault — it is an absence we admit to"
    );
    assert_eq!(p.counters().fb_window_writes, 1);
    assert_eq!(
        p.counters().bar1_faults,
        0,
        "and not miscounted as a translation refusal"
    );
}

/// ★★★★★ **ITEM 2 / `w262` — THE C's GP_PUT WITNESS, PINNED AGAINST THE ABI FROM THIS SIDE.**
///
/// `qemu/hw/misc/nvkvm/nvkvm.c` cannot `use kayfabe_abi::submit::USERD_GP_PUT` — it is C, and
/// the archive it links is a static library with no header for that constant. So its
/// `NVKVM_USERD_GP_PUT` is a **literal `0x8c`**, and a literal that drifts from the ABI is
/// silent: the witness would simply never fire, and *"the guest never advanced a cursor"* and
/// *"we were watching the wrong offset"* are the same empty log.
///
/// ⊘ `a_census_zero_needs_a_known_positive`. This is that known-positive, held on the side
/// that owns the number.
#[test]
fn the_c_gp_put_witness_watches_the_offset_the_abi_names() {
    // The C's literal, transcribed here so a change to either side breaks this line.
    const NVKVM_USERD_GP_PUT: u64 = 0x8c;
    assert_eq!(
        NVKVM_USERD_GP_PUT,
        kayfabe_abi::submit::USERD_GP_PUT,
        "`nvkvm.c`'s NVKVM_USERD_GP_PUT has drifted from `kayfabe_abi::submit::USERD_GP_PUT`. \
         ⊘ The symptom of that drift is an EMPTY witness, which reads exactly like a guest \
         that never advanced a cursor."
    );

    // ★ And the predicate the C applies — `(addr & 0xfff) == USERD_GP_PUT` — must actually
    // select the four stores every boot since `s17_e8fde62` has recorded, and must not select
    // the GPFIFO-entry stores that sit beside them. ⚠ Measured offsets, not invented ones.
    let gp_put_stores = [0xa008c_u64, 0xc008c, 0xe008c, 0x10008c];
    let entry_stores = [
        0x90000_u64,
        0x90004,
        0xb0000,
        0xb0004,
        0xd0000,
        0xd0004,
        0xf0000,
        0xf0004,
        0x90008,
        0x9000c,
        0x90010,
    ];
    for a in gp_put_stores {
        assert_eq!(
            a & 0xfff,
            kayfabe_abi::submit::USERD_GP_PUT,
            "{a:#x} is a measured GP_PUT store and the C's page-offset predicate misses it"
        );
    }
    for a in entry_stores {
        assert_ne!(
            a & 0xfff,
            kayfabe_abi::submit::USERD_GP_PUT,
            "{a:#x} is a measured GPFIFO-ENTRY store and the C's predicate claims it as a \
             cursor advance — which would make the ordering measurement report the entry \
             store's instant instead of the cursor's"
        );
    }

    // ★★★★★ ⊘⊘ **THE FALSE-POSITIVE CLASS, MEASURED — and the eleven rows above do NOT rule
    // it out.** `[measured 2026-08-12, boot `w262b_ring`, GA106 / 580.159.04]` the per-page
    // census recorded a "cursor advance" on page `+0x90000` with **`first_val = 0xd801`**.
    // `0x90000` is a RING page (`BAR1[0] off=0x90000 val=0x20000000`), and `0xd801` has the
    // shape of a GPFIFO entry's HIGH dword — compare the measured `0x2801` and `0x6801`. ⇒ a
    // ring whose 18th entry pair lands at `+0x88 / +0x8c` collides with the predicate.
    //
    // ⚠ The eleven `entry_stores` above are the entries the guest happened to write in the
    // FIRST 16 recorded accesses; they stop at `0x90010`. A gate built only from them reads as
    // *"the predicate is clean"* and is not — `a_correct_citation_narrowed_by_the_reading`.
    // This row is the counterexample, so the claim can never be made again from that evidence.
    let measured_false_positive: u64 = 0x9008c;
    assert_eq!(
        measured_false_positive & 0xfff,
        kayfabe_abi::submit::USERD_GP_PUT,
        "the measured false positive must still be one, or this record has gone stale"
    );
    // ⊘ The only signal that separated it was the VALUE's shape, and a value shape is evidence
    // and not a discriminator. A page offset alone cannot say USERD from ring, and this
    // assertion exists so that a future rung claiming otherwise has to delete it first.
    assert_eq!(
        (
            0xd801_u64 & 0xffff,
            0x2801_u64 & 0xffff,
            0x6801_u64 & 0xffff
        ),
        (0xd801, 0x2801, 0x6801),
        "the three measured GPFIFO-entry high dwords"
    );

    // ⊘⊘ AND THE LIMIT, asserted rather than only written down. The four measured stores are
    // `0x20000` apart; the walling GR channels' declared USERDs (`userd=h0x5c000014/off0x2000,
    // 0x5000, 0x8000, …`, `w261`) are `0x3000` apart in ONE memory object. Nothing joins a
    // BAR1 offset to a channel, so this cannot be closed — but the two strides are a fact, and
    // a rung that later claims "we measured the GR channel's own cursor" has to get past it.
    let bar1_stride = gp_put_stores[1] - gp_put_stores[0];
    let gr_userd_stride = 0x5000_u64 - 0x2000;
    assert_ne!(
        bar1_stride, gr_userd_stride,
        "the recorded BAR1 cursor stride now equals the GR channels' declared USERD stride. \
         ⊘ That would be evidence worth having and it must not arrive silently: it is the \
         one thing that could turn this witness from 'the guest's first cursor advance' into \
         'the GR channel's cursor advance'."
    );
    assert_eq!((bar1_stride, gr_userd_stride), (0x20000, 0x3000));
}
