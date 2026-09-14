//! # ⊘⊘⊘ SUPERSEDED 2026-09-14 (w721) — **THIS FILE ASSERTS A DESIGN THE OWNER RETIRED.**
//! ### Read this before the module docs below, and before treating the `#[ignore]`d test as a
//! ### falsifier that is merely waiting to go green.
//!
//! `THE_CONSTRAINTS.md`, *"⊘⊘⊘ SUPERSEDED w721 — THERE IS ONE WORLD, NOT TWO"*, quoting the
//! owner: *"One GPGA store, one RM object, no more fake fb, no more bar1/bar2 traps"* …
//! *"**BAR1, BAR2, PRAMIN, channels and engines are all views of IT**."*
//!
//! ⇒ Under the single store, a framebuffer page written through BAR1 **IS** the page BAR2
//! reads. They are one memory, at one GPGA, by construction.
//!
//! ★★★ **So `a_framebuffer_page_written_through_bar1_is_not_the_page_bar2_reads` is not a
//! test that is red and waiting.** It asserts the **opposite** of the design being built, and
//! making it green would require re-introducing the second memory the single store exists to
//! delete. `SINGLE_STORE_PLAN.md`'s falsifier table still lists it as *"today RED → after
//! GREEN"*; that row predates w721 and is **wrong**.
//!
//! ⊘ And §18 — the constraint this file cites as its reason — now reads *"**SATISFIED BY
//! CONSTRUCTION** under the single store … there is no other memory to substitute"*. A
//! falsifier for §18 that works by proving there **are** two memories cannot survive §18
//! being satisfied by there being one.
//!
//! ## What should happen to this file
//!
//! ⚠ **Not "flip the assertion and move on".** Three of the four tests here pin sub-properties
//! of the two-world split (*"the page tables must stay in the aperture world"*, *"the window →
//! world map must be the one §15 wrote down"*), and those are retired with it. The fourth —
//! *"PRAMIN and BAR2 must stay ONE world"* — survives, trivially, because everything is one
//! world now.
//!
//! ⇒ The **single store's own falsifier** is the inverse property —
//! [`a_framebuffer_page_written_through_bar1_is_the_page_bar2_reads`], added below — and it is
//! a real falsifier rather than a tautology: it fails today, because this plane serves BAR1
//! and BAR2 out of one `PlaneMem::fb` **only by accident of not having split them**, and it
//! would fail again the moment anyone re-introduced a second store for BAR1.
//!
//! ⊘ Deleting the retired tests is **increment 7's** work (`SINGLE_STORE_PLAN.md`: deletions
//! come last, and *"unwire and delete in the same change"*). They are left compiling, and
//! marked, so nobody reads them as current on the way.
//!
//! ---
//!
//! ★★★★★ **THE TWO WORLDS — `THE_CONSTRAINTS.md` §15/§18, pinned where the bytes live.**
//!
//! §15 splits framebuffer backing by **aperture**, into two stores that are not the same
//! memory:
//!
//! | world | backing | serves |
//! |---|---|---|
//! | the reserved object | ONE device-local RM allocation, all guest video memory | BAR1, userspace channels, engines |
//! | the aperture store | ONE sparse memfd of the advertised VRAM size | BAR2, PRAMIN — page tables and control structures only |
//!
//! §18 says why the split is a *correctness* constraint and not a layout preference: a guest
//! vidmem page served out of host system memory is **right in value and wrong in residence**.
//! Every byte reads back correctly and every engine access becomes a PCIe round trip, so the
//! defect is invisible to exactly the tests that would normally catch a backing bug.
//!
//! # ⊘ What is true of this tree TODAY, and why three of these four tests pass
//!
//! `RegPlane::fb_read`, `RegPlane::fb_write` and `RegPlane::window_page_backing` resolve all
//! three windows to a framebuffer-physical address and then go to the **same**
//! `PlaneMem::fb`. There is one store, so there is one world — `[measured w719]`, and §18's
//! own status block says so. ⇒ The falsifier in §1 is `#[ignore]`d because it is a statement
//! about the tree that is coming, not the tree that is here; the rest of this file pins the
//! parts of the split that are already true, so that building the split cannot silently
//! break them on the way.
//!
//! # ★★★ THE THREE THINGS THAT MUST NOT MOVE WHILE BAR1 MOVES
//!
//! The reason this file is not one test: the interesting failure of the §15 work is not
//! *"BAR1 did not move"*. It is **BAR1 moving and taking something with it**, and each of
//! those somethings is silent.
//!
//! 1. **The page tables must stay in the aperture world.** BAR1's own PD/PT pages are written
//!    by the guest through a *control* aperture and read by the walk. Move the walk's byte
//!    source onto the reserved object and the guest's table writes become invisible to it —
//!    and a walk over a zero-filled table does not fault, it answers `Unmapped` on an address
//!    the guest believes it mapped.
//! 2. **PRAMIN and BAR2 must stay ONE world.** They are the same row of §15's table. Two
//!    stores where the table says one, and RM's `kbusVerifyBar2_GM107` — which writes through
//!    PRAMIN and reads through BAR2 — reports `NV_ERR_MEMORY_ERROR` hundreds of operations
//!    after the write that was lost.
//! 3. **The window → world map must be the one §15 wrote down.** It is expressed in exactly
//!    one place in this tree (`plane.rs`'s `crate::twoworlds::note` call), and it is what any
//!    later dispatch will key on.
//!
//! ⊘ **What this file cannot say anything about.** It is a unit of the register plane: the
//! stores here are memfd-shaped `SparseFb`s on both sides of the split, because that is what a
//! test without a GPU can hold. **Residence** — the §18 property, whether the bytes are really
//! in device-local memory — is not a question a `cargo test` can ask, and no assertion here
//! pretends to. What these tests pin is **disjointness**: that the two worlds are not one
//! memory. Residence is measured on hardware, by bandwidth or by the aperture the engine
//! reached, and §18's own "The falsifier this needs" says so.

use kayfabe_abi::versions::BENCH_DRIVER;
use kayfabe_device::fbwin::SparseFb;
use kayfabe_device::plane::{ReadOutcome, RegPlane};
use kayfabe_device::{FbWindow, NanoClock, SteppingClock, abi, ga10x::GA106};

mod tiny;
use tiny::{E_VALID, TinyFmt};

// ───────────────────────────── the three apertures ─────────────────────────────

/// The register aperture, which carries the untranslated BAR0 moving window (PRAMIN).
const BAR_REGS: u8 = kayfabe_abi::pcibars::bus_bar::REGS as u8;
/// The framebuffer aperture — §15's `reserved object` row. `p.read(1, ..)`.
const BAR_FB: u8 = kayfabe_abi::pcibars::bus_bar::FB as u8;
/// The instance window — §15's `aperture store` row, beside PRAMIN. `p.read(2, ..)`.
const BAR_INST: u8 = kayfabe_abi::pcibars::bus_bar::INST as u8;

// ───────────────────────────── the framebuffer this file uses ─────────────────────────────

/// **The one framebuffer page both worlds are aimed at.** 2 MiB-aligned, because
/// [`TinyFmt`]'s only page size is 2 MiB, and far below the firmware carve-out so nothing
/// this port reserves is involved in the answer.
const SHARED_PHYS: u64 = 0x0200_0000;
/// A second frame, for the test that re-points a leaf and checks the walk followed.
const ALT_PHYS: u64 = 0x0220_0000;
/// Two more, used only by the world-attribution census so that test cannot be perturbed by,
/// nor perturb, the pages the others touch.
const CENSUS_BAR1_PHYS: u64 = 0x0400_0000;
const CENSUS_CTRL_PHYS: u64 = 0x0420_0000;

/// BAR1's level-1 table page, and BAR2's. Distinct trees on purpose: if the two apertures
/// shared a tree, "they resolved to the same GPGA" would be a statement about one table
/// rather than about two independently-built translations.
const B1_L1: u64 = 0x0300_0000;
const B2_L1: u64 = 0x0310_0000;

/// The BAR1 offset and the BAR2 offset that both land on [`SHARED_PHYS`].
///
/// ⊘ Deliberately **different** offsets. Equal ones would let a plane that ignored the page
/// tables entirely and answered `phys = off` pass every assertion in this file.
const BAR1_VA: u64 = 0x0040_0000;
const BAR2_VA: u64 = 0x0060_0000;

/// The value the **control** world holds at [`SHARED_PHYS`], written through PRAMIN.
const CONTROL_SENTINEL: u32 = 0xC0C0_0001;
/// The value the **device** world holds at the same GPGA, written through BAR1. Distinct in
/// every byte, so a partial-width or endianness defect cannot make the two look equal.
const DEVICE_SENTINEL: u32 = 0x1D1D_BEEF;

// ───────────────────────────── entry encodings ─────────────────────────────

/// A [`TinyFmt`] level-0 entry naming the next table. ⊘ Re-derived here from the fixture's
/// own `E_VALID` rather than copied as a literal, so this file cannot build a tree the format
/// reads differently — the shape `bar1_translation.rs` calls
/// `two_encodings_agreeing_on_the_first_values`.
fn pde(next: u64) -> u64 {
    (E_VALID as u64) | ((next >> 12) << 12)
}

/// A [`TinyFmt`] level-1 leaf naming a 2 MiB vidmem frame.
fn leaf(phys: u64) -> u64 {
    (E_VALID as u64) | ((phys >> 12) << 12)
}

// ───────────────────────────── the plane, and the guest's own hands ─────────────────────

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

/// Point `NV_PBUS_BAR0_WINDOW` at the 64 KiB granule containing `phys`, RM's own way.
fn point_window(p: &RegPlane, phys: u64) {
    let cur = p.read(BAR_REGS, GA106.bar0_window_reg, 4).value() as u32;
    let base = u32::try_from(phys >> 16).expect("a 24-bit window base");
    let val = (cur & !0x00FF_FFFF) | (base & 0x00FF_FFFF);
    p.write(BAR_REGS, GA106.bar0_window_reg, 4, u64::from(val));
}

/// Write one dword **through PRAMIN** — the control world's hand, and the only way this file
/// ever puts a page-table byte in the framebuffer.
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
        "the PRAMIN write must land, and say where; without that every later assertion \
         is about a tree that was never written"
    );
}

fn pramin_rd32(p: &RegPlane, phys: u64) -> u32 {
    point_window(p, phys);
    p.read(BAR_REGS, GA106.pramin_window.base + (phys & 0xFFFF), 4)
        .value() as u32
}

fn pramin_wr_entry(p: &RegPlane, phys: u64, entry: u64) {
    pramin_wr32(p, phys, entry as u32);
    pramin_wr32(p, phys + 4, (entry >> 32) as u32);
}

/// Build BAR1's tree — rooted at the address **this port publishes** in `bar1PdeBase`, which
/// is the only address in either tree the guest does not choose.
fn build_bar1_tree(p: &RegPlane, va: u64, leaf_entry: u64) {
    let root = GA106.bar1_pde_base;
    assert_ne!(
        root, 0,
        "this chip row must publish a bar1PdeBase, or BAR1 has no address model and every \
         assertion in this file would be about a refusal instead of about a translation"
    );
    pramin_wr_entry(p, root + ((va >> 30) & 511) * 8, pde(B1_L1));
    pramin_wr_entry(p, B1_L1 + ((va >> 21) & 511) * 8, leaf_entry);
}

/// Build BAR2's tree and publish its root, the way the guest does: the directory pages go
/// into the framebuffer, and the **root entry** travels as a value on `UPDATE_BAR_PDE`.
fn build_and_publish_bar2_tree(p: &RegPlane, va: u64, leaf_entry: u64) {
    pramin_wr_entry(p, B2_L1 + ((va >> 21) & 511) * 8, leaf_entry);

    let driver = *kayfabe_abi::versions::table_for(BENCH_DRIVER).expect("the bench driver table");
    let mut link = kayfabe_device::bar2::BarPdePolicy::new(driver, p.bar_pde_log());
    let mut body = vec![0u8; kayfabe_device::bar2::UPDATE_BAR_PDE_BODY_SIZE];
    // `NV_RPC_UPDATE_PDE_BAR_2`.
    body[0..4].copy_from_slice(&1u32.to_le_bytes());
    body[8..16].copy_from_slice(&pde(B2_L1).to_le_bytes());
    // `pRootFmt->virtAddrBitLo` — TinyFmt's level 0.
    body[16..24].copy_from_slice(&30u64.to_le_bytes());
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
    .expect("the link answers UPDATE_BAR_PDE");
    assert_eq!(reply.rpc_result, 0, "NV_OK");
}

/// The framebuffer address a windowed read resolved to, or a panic naming the refusal —
/// never a zero. ⊘ A refusal read as "the bytes were zero" is the one way a test in this
/// file could pass while measuring nothing.
fn fb_read(p: &RegPlane, bar: u8, off: u64) -> (u64, u32) {
    match p.read(bar, off, 4) {
        ReadOutcome::Fb { phys, value, .. } => (phys, value as u32),
        other => panic!(
            "BAR{bar} offset {off:#x} was not served out of the framebuffer: {other:?} — \
             this is a SETUP failure, not a finding about the two worlds"
        ),
    }
}

// =====================================================================================
// 1. ★★★★★ THE FALSIFIER — expected RED today, and green when BAR1 moves
// =====================================================================================

/// ★★★★★ **ONE framebuffer address, TWO memories — the property constraint 15 and
/// constraint 18 both reduce to, and the one this tree does not have.**
///
/// # The defect this catches
///
/// Guest video memory served out of the aperture store. `plane.rs`'s `window_page_backing`
/// and `fb_read`/`fb_write` translate `FbWindow::Pramin`, `FbWindow::FbAperture` and
/// `FbWindow::InstanceWindow` to a framebuffer-physical address and then call the **same**
/// `s.fb` for all three, so every guest vidmem page is whatever the aperture store is — a
/// memfd, which is host system memory. §18: *"correct in value and wrong in residence"*.
/// Every engine access to it is a PCIe round trip instead of a local vidmem access, and
/// **`--ce-client` is green either way**, which is how it survived.
///
/// # Why the assertion is shaped this way
///
/// It does not assert *"the two differ"* against an unspecified second value — an
/// uninitialised store reads zero, and `!= 0` would also be satisfied by a plane that
/// refused, faulted, or resolved a different address. It writes a **named** value into each
/// world and then asserts each world still holds **its own**:
///
/// - the control world's sentinel goes in through PRAMIN;
/// - the device world's goes in through BAR1, at a VA the walk resolves to the *same* GPGA,
///   which the setup asserts before anything else;
/// - and BAR2 — the control world's other aperture — must still read the control sentinel.
///
/// ⊘ Today it reads the *device* sentinel, because BAR1's write went into the one store.
///
/// ⚠ **This test's own falsifier.** The `assert_eq!` on `phys` above the aliasing assertion
/// is not decoration: it is what makes a red here mean *"the same address has one memory"*
/// rather than *"the two apertures never met"*. A version of this test that asserted only
/// inequality would go green the day someone broke BAR2's translation.
#[ignore = "★★★★★ CONSTRAINT 18's FALSIFIER — expected RED until BAR1 is served from the \
            reserved device-local object instead of the aperture store. Today `plane.rs`'s \
            `window_page_backing`/`fb_read`/`fb_write` send all three windows to the one \
            `PlaneMem::fb`, so a guest vidmem page IS the aperture memfd (host sysmem) and \
            the two worlds alias. It goes GREEN when BAR1 moves onto the reserved object — \
            run it with `cargo test -p kayfabe-device -- --ignored` and delete this \
            attribute the moment it passes."]
#[test]
fn a_framebuffer_page_written_through_bar1_is_not_the_page_bar2_reads() {
    let p = plane();
    build_bar1_tree(&p, BAR1_VA, leaf(SHARED_PHYS));
    build_and_publish_bar2_tree(&p, BAR2_VA, leaf(SHARED_PHYS));

    // ── setup, asserted before the property: the two apertures really do meet here ──
    let (b1_phys, _) = fb_read(&p, BAR_FB, BAR1_VA);
    let (b2_phys, _) = fb_read(&p, BAR_INST, BAR2_VA);
    assert_eq!(
        b1_phys, SHARED_PHYS,
        "BAR1 must resolve its VA to the shared framebuffer address, or this test is not \
         about aliasing"
    );
    assert_eq!(
        b2_phys, SHARED_PHYS,
        "BAR2 must resolve ITS VA to the same framebuffer address, or the two worlds were \
         never pointed at one page and any difference below would be trivial"
    );

    // ── the control world takes its value, through PRAMIN ──
    pramin_wr32(&p, SHARED_PHYS, CONTROL_SENTINEL);

    // ── the device world takes its own, through BAR1, at the same GPGA ──
    let w = p.write(BAR_FB, BAR1_VA, 4, u64::from(DEVICE_SENTINEL));
    assert_eq!(
        w.fb_landed,
        Some(SHARED_PHYS),
        "the BAR1 write must land at the shared address; a dropped write would make the \
         assertion below pass for the wrong reason"
    );

    // ── THE PROPERTY ──
    let (_, through_bar2) = fb_read(&p, BAR_INST, BAR2_VA);
    assert_eq!(
        through_bar2, CONTROL_SENTINEL,
        "★★★★★ constraint 15/18: the aperture store and the reserved object are TWO \
         memories, so a write through BAR1 to {SHARED_PHYS:#x} must not be visible through \
         BAR2 at the same address — BAR2 read {through_bar2:#x}, which is BAR1's value \
         ({DEVICE_SENTINEL:#x}), so one framebuffer address has one memory and guest vidmem \
         is being served out of the aperture memfd"
    );

    // And the converse, so a fix that merely made BAR2 blind would not pass.
    let (_, through_bar1) = fb_read(&p, BAR_FB, BAR1_VA);
    assert_eq!(
        through_bar1, DEVICE_SENTINEL,
        "★ the device world must still hold ITS value: splitting the worlds may not cost \
         BAR1 the bytes the guest wrote through it"
    );
    assert_eq!(
        pramin_rd32(&p, SHARED_PHYS),
        CONTROL_SENTINEL,
        "★ and PRAMIN, the control world's other aperture, must still hold the control value"
    );
}

// =====================================================================================
// 2. ★★★ WHAT MUST NOT MOVE WHILE BAR1 MOVES — green today, and must stay green
// =====================================================================================

/// ★★★★ **BAR1's PAGE TABLES LIVE IN THE APERTURE WORLD** — the walk reads the bytes the
/// control window wrote.
///
/// # The defect this catches
///
/// Moving BAR1 onto the reserved object **and taking its page tables with it**. §15 is
/// explicit that the aperture store holds *"page tables (PT\*/PD\*) and control structures
/// only"*: the tables are control structures, so they stay in the aperture world even though
/// the data they map does not. A split that moved the walk's byte source too would make the
/// guest's own table writes — which arrive through a control aperture — invisible to the walk.
///
/// ⚠ **And that failure is silent in the worst way.** A walk over a table page the guest
/// wrote elsewhere does not fault as "missing table"; it reads zeros, and a zero entry is a
/// well-formed *"this maps nothing"*. The guest gets `Unmapped` on an address it believes it
/// mapped, with nothing anywhere naming the store as the cause.
///
/// ⊘ Every page-table byte here goes in through PRAMIN and **nothing is written through
/// BAR1**, so a plane that gave BAR1 a private store for its tables could not pass by
/// agreeing with itself.
#[test]
fn the_bar1_walk_reads_the_page_tables_the_control_window_wrote() {
    let p = plane();
    build_bar1_tree(&p, BAR1_VA, leaf(SHARED_PHYS));

    let (phys, _) = fb_read(&p, BAR_FB, BAR1_VA);
    assert_eq!(
        phys, SHARED_PHYS,
        "★ the BAR1 translation must resolve through the tree PRAMIN wrote: the walk's byte \
         source is the aperture store, which is where the guest's PD/PT pages are"
    );

    // ★★★ The non-vacuous half. Re-point the leaf through the control window ONLY, and the
    // very next BAR1 access must follow it. Without this, a plane that had cached the first
    // translation — or that resolved `phys` by arithmetic and never read a table at all —
    // would pass the assertion above.
    pramin_wr_entry(&p, B1_L1 + ((BAR1_VA >> 21) & 511) * 8, leaf(ALT_PHYS));
    let (moved, _) = fb_read(&p, BAR_FB, BAR1_VA);
    assert_eq!(
        moved, ALT_PHYS,
        "★★★ a leaf re-pointed through the CONTROL aperture must move BAR1's translation: \
         if it does not, the walk is not reading the guest's tables out of the aperture \
         store and a guest remap is being silently ignored"
    );
}

/// ★★★★ **PRAMIN AND BAR2 ARE ONE WORLD** — the same row of §15's table, and therefore the
/// same memory.
///
/// # The defect this catches
///
/// Splitting the *control* aperture while splitting BAR1 — three stores where §15 says two —
/// or moving BAR2 onto the reserved object instead of BAR1, which is the same edit with the
/// wrong window named. Either way `kbusVerifyBar2_GM107` is the caller that notices: it writes
/// a dword through the BAR0 moving window and reads it back **through BAR2**, and it does so
/// hundreds of operations after `kbusInitBar2` has already built the whole instance tree
/// through that same window without reading any of it back. So a control world that had split
/// in two would surface as `NV_ERR_MEMORY_ERROR` at a verify, with every earlier step having
/// returned `NV_OK`.
///
/// ⊘ Asserted in **both** directions. One direction is satisfied by a store that is
/// write-through in one direction and stale in the other, which is a real shape for a mirror.
#[test]
fn pramin_and_bar2_resolve_one_framebuffer_address_to_one_memory() {
    let p = plane();
    build_and_publish_bar2_tree(&p, BAR2_VA, leaf(SHARED_PHYS));

    // PRAMIN writes, BAR2 reads.
    pramin_wr32(&p, SHARED_PHYS, CONTROL_SENTINEL);
    let (phys, seen) = fb_read(&p, BAR_INST, BAR2_VA);
    assert_eq!(
        phys, SHARED_PHYS,
        "BAR2 must resolve its VA to the address PRAMIN wrote, or the comparison below is \
         between two different pages"
    );
    assert_eq!(
        seen, CONTROL_SENTINEL,
        "★★★ constraint 15 puts BAR2 and PRAMIN in ONE world: a dword written through the \
         BAR0 moving window must be the dword BAR2 reads at that framebuffer address — this \
         is `kbusVerifyBar2_GM107`'s own acceptance, and a second control store fails it"
    );

    // …and back the other way, so a one-directional mirror cannot pass.
    let w = p.write(BAR_INST, BAR2_VA, 4, u64::from(DEVICE_SENTINEL));
    assert_eq!(
        w.fb_landed,
        Some(SHARED_PHYS),
        "the BAR2 write must land at the shared address"
    );
    assert_eq!(
        pramin_rd32(&p, SHARED_PHYS),
        DEVICE_SENTINEL,
        "★★★ and the control world is one memory in both directions: what BAR2 writes, \
         PRAMIN reads"
    );
}

/// ★★★★ **EACH WINDOW IS ATTRIBUTED TO THE WORLD §15 ASSIGNS IT** — BAR1 to the reserved
/// object, PRAMIN and BAR2 to the aperture store.
///
/// # The defect this catches
///
/// The window → world map is expressed in exactly **one** place in this tree — the
/// `crate::twoworlds::note` call inside `RegPlane::window_page_backing` — and it is what any
/// later backing dispatch will key on. A window classified into the wrong world there sends
/// its bytes to the wrong store **with no refusal and no fault**: the access succeeds, the
/// value is plausible, and the only symptom is that two things that should have been the same
/// memory are not (or that two that should have been different are).
///
/// ⚠ PRAMIN is the arm most likely to be got wrong, because it is in BAR**0** and every other
/// classification in this plane keys on the BAR index. §15 puts it with BAR2 regardless.
///
/// ⊘ **Read as deltas, and this must stay the only test in this file that drives
/// `window_page_backing`.** `twoworlds`' maps are process-wide, so an absolute census here
/// would be a statement about whatever else in this binary happened to run first. The four
/// framebuffer addresses it touches are its own for the same reason.
#[test]
fn the_plane_attributes_each_window_to_the_world_constraint_15_assigns_it() {
    let p = plane();
    // A BAR1 mapping and a BAR2 mapping onto two DIFFERENT pages — this test is about which
    // world each window is recorded as, so a shared page would report a collision that says
    // nothing about attribution.
    build_bar1_tree(&p, BAR1_VA, leaf(CENSUS_BAR1_PHYS));
    build_and_publish_bar2_tree(&p, BAR2_VA, leaf(CENSUS_CTRL_PHYS));

    let before = kayfabe_device::twoworlds::census();

    let b1 = p
        .window_page_backing(FbWindow::FbAperture, BAR1_VA, false)
        .expect("the BAR1 page resolves");
    assert_eq!(b1.phys, CENSUS_BAR1_PHYS);
    let b2 = p
        .window_page_backing(FbWindow::InstanceWindow, BAR2_VA, false)
        .expect("the BAR2 page resolves");
    assert_eq!(b2.phys, CENSUS_CTRL_PHYS);
    // PRAMIN, whose window this test points itself — the same 4 KiB page BAR2 just reached,
    // so the two control apertures agreeing is what keeps `control_pages` at one.
    point_window(&p, CENSUS_CTRL_PHYS);
    let pr = p
        .window_page_backing(
            FbWindow::Pramin,
            GA106.pramin_window.base + (CENSUS_CTRL_PHYS & 0xFFFF),
            false,
        )
        .expect("the PRAMIN page resolves");
    assert_eq!(pr.phys, CENSUS_CTRL_PHYS);

    let after = kayfabe_device::twoworlds::census();
    assert_eq!(
        after.bar1_pages - before.bar1_pages,
        1,
        "★★★ BAR1 — and only BAR1 — must be recorded as the reserved-object world: §15 puts \
         the framebuffer aperture there and nothing else"
    );
    assert_eq!(
        after.control_pages - before.control_pages,
        1,
        "★★★ BAR2 and PRAMIN must BOTH be recorded as the aperture-store world, and they \
         reached one page between them, so the control world grew by exactly one — a PRAMIN \
         arm classified by BAR index would land in BAR1's world and make this two"
    );
    assert_eq!(
        after.both_pages - before.both_pages,
        0,
        "⊘ these three accesses reached two distinct pages through the two worlds, so none \
         of them is a collision; a non-zero here means the attribution ran but the addresses \
         this test chose are not the ones it resolved"
    );
    assert_eq!(
        after.out_of_range, before.out_of_range,
        "⊘ every address this test used must be inside the census's coverage: an \
         out-of-range access is UNMEASURED, and a delta of zero above would then be vacuous"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// ★★★★★ THE SINGLE STORE'S OWN FALSIFIER — the inverse of the retired one above.
// ═══════════════════════════════════════════════════════════════════════════════════════

/// ★★★★★ **ONE GPGA, ONE MEMORY** — `THE_CONSTRAINTS.md`'s *"SUPERSEDED w721 — there is one
/// world, not two"*: *"BAR1, BAR2, PRAMIN, channels and engines are all views of **it**."*
///
/// A framebuffer page written through BAR1 **is** the page BAR2 reads, at the same GPGA. That
/// is the property the single store delivers and the one the reserved object exists for.
///
/// # ⊘ Why this is a falsifier and not a tautology
///
/// It would be a tautology if it merely restated *"this plane happens to have one `PlaneMem::fb`
/// today"*. It is not, for two reasons:
///
/// 1. ⚠ **It is the exact property the `#[ignore]`d test above asserts the negation of**, and
///    that test is still in the tree and still named in `SINGLE_STORE_PLAN.md`'s falsifier
///    table as something to make green. The two cannot both pass. Having both, with one of
///    them marked superseded, is what stops the retired one from being "fixed" back into life.
/// 2. ★ It **fails the moment anyone gives BAR1 a second store** — which is exactly the edit
///    the retired design calls for, and exactly the accident that a `page_backing` switch
///    touching only one of the two translate paths would produce.
///
/// ⊘ **What it cannot say**: whether the bytes are really device-local. That is §18's
/// residence property, it is not a question `cargo test` can ask, and the module docs above
/// already say so. This pins **identity** — one address, one memory — not residence.
#[test]
fn a_framebuffer_page_written_through_bar1_is_the_page_bar2_reads() {
    let p = plane();
    build_bar1_tree(&p, BAR1_VA, leaf(SHARED_PHYS));
    build_and_publish_bar2_tree(&p, BAR2_VA, leaf(SHARED_PHYS));

    // ── setup, asserted before the property: both apertures really do name one GPGA ──
    let (b1_phys, _) = fb_read(&p, BAR_FB, BAR1_VA);
    let (b2_phys, _) = fb_read(&p, BAR_INST, BAR2_VA);
    assert_eq!(
        b1_phys, SHARED_PHYS,
        "BAR1 must resolve its VA to the shared framebuffer address, or this test is not \
         about one memory"
    );
    assert_eq!(
        b2_phys, SHARED_PHYS,
        "BAR2 must resolve ITS VA to the same framebuffer address, or the agreement below \
         would be about two unrelated addresses and would mean nothing"
    );

    // ── write through BAR1 ──
    let w = p.write(BAR_FB, BAR1_VA, 4, u64::from(DEVICE_SENTINEL));
    assert_eq!(
        w.fb_landed,
        Some(SHARED_PHYS),
        "the BAR1 write must land at the shared address; a DROPPED write would make the \
         assertion below fail for a reason that has nothing to do with the store"
    );

    // ── THE PROPERTY: BAR2 reads what BAR1 wrote ──
    let (_, through_bar2) = fb_read(&p, BAR_INST, BAR2_VA);
    assert_eq!(
        through_bar2, DEVICE_SENTINEL,
        "★★★★★ THE SINGLE STORE: BAR1 and BAR2 are views of ONE reserved object, so a write \
         through BAR1 at {SHARED_PHYS:#x} must be visible through BAR2 at the same address. \
         BAR2 read {through_bar2:#x}. If this fails, something gave BAR1 a store of its own — \
         which is the two-world design w721 retired, and it makes the reserved object \
         impossible: there would be two memories for one GPGA and nothing could say which the \
         engine reads."
    );

    // ── and PRAMIN, the third view, sees it too ──
    assert_eq!(
        pramin_rd32(&p, SHARED_PHYS),
        DEVICE_SENTINEL,
        "★ PRAMIN is a view of the same object. A split that left PRAMIN behind would be the \
         same defect one aperture over, and it is the one RM's own `kbusVerifyBar2_GM107` \
         catches hundreds of operations after the write that was lost."
    );
}
