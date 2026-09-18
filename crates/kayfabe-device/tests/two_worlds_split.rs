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

// ⊘⊘⊘ **PLACED HERE, AFTER THE IMPORTS, AND NOT ABOVE THE FIRST `#[test]`.** `[measured w763]`
// the first attempt anchored on the first `#[test]` and landed BETWEEN a `#[ignore = "..."]`
// and the function it decorates — silently detaching it, so a falsifier documented as
// "expected RED until BAR1 is served from the reserved object" ran and failed, and read as a
// regression this commit had caused. ⚠ An insertion before an ITEM can separate that item
// from its attributes; an insertion after the imports cannot.
/// ⊘ The `twoworlds` census is a PROCESS GLOBAL and four tests in this binary move it. Held
/// across each one's whole before/act/after window, so no two can interleave. See the note on
/// `the_plane_attributes_each_window_to_the_world_constraint_15_assigns_it`.
static CENSUS: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    publish_bar2_root(p);
}

/// Publish BAR2's **root entry**, the way the guest does — as a value on `UPDATE_BAR_PDE`,
/// never as a page. ⊘ Split out of [`build_and_publish_bar2_tree`] so the single-store tests
/// can put the directory page into the reserved object by a path the host cannot see and
/// still publish the root the same way.
fn publish_bar2_root(p: &RegPlane) {
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
/// ⊘⊘⊘ **THIS COMMENT USED TO STATE AN INVARIANT THAT NOTHING ENFORCED, AND IT WAS ALREADY
/// FALSE WHEN IT WAS WRITTEN.** It said *"this must stay the only test in this file that
/// drives `window_page_backing`"* — while THREE other tests in the same binary already did,
/// all of them on `FbWindow::FbAperture`, which is the very counter this asserts a delta of 1
/// on. `[measured w763]` it finally interleaved and reported `left: 2`.
///
/// ⇒ **Deltas do not make a process-global census safe; they make it safe against what ran
/// BEFORE, and say nothing about what runs BESIDE.** Cargo runs a file's tests on several
/// threads of one process. ⚠ Third instance of this class in one session, with
/// `sweep_defer_census` and the `KAYFABE_FB_STORE` fixtures — a shared global read as if it
/// were local.
///
/// ★ The rule the comment wanted is now ENFORCED, not asserted: every test that drives
/// `window_page_backing` takes `CENSUS` across its whole window. The four
/// framebuffer addresses it touches are its own for the same reason.
#[test]
fn the_plane_attributes_each_window_to_the_world_constraint_15_assigns_it() {
    let _census = CENSUS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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

// ═══════════════════════════════════════════════════════════════════════════════════════
// ★★★ §w724g's KNOWN-POSITIVE — the refusal can FIRE, so a boot's zero means something.
// ═══════════════════════════════════════════════════════════════════════════════════════

/// ★★★★★ **THE ARM ACTUALLY REFUSES.** `SINGLE_STORE_PLAN.md` §w724g asks for one boot with
/// the trap path armed to refuse, to convert *"`TRAP_FILLS` reads zero"* into *"the trap path
/// is unreachable"*.
///
/// # ⊘⊘ Why a known-positive is not optional here
///
/// The boot's whole result is a **zero**. This tree's single most repeated defect is a
/// diagnostic that prints zero because **its arm never ran** — and a zero from an arm that was
/// never reached is indistinguishable, in every report anyone reads, from a zero from an arm
/// that ran and found nothing.
///
/// ⇒ This test makes the refusal fire on demand, on the same code path a trapped guest access
/// takes (`fb_read` → `window_phys`), and asserts it is **that** refusal by its sentence rather
/// than merely "something was refused".
#[test]
fn the_armed_trap_path_refuses_by_name_and_counts_it() {
    use kayfabe_device::plane::{FB_TRAP_REFUSED, FbTrapPolicy};

    let p = plane();
    build_bar1_tree(&p, BAR1_VA, leaf(SHARED_PHYS));

    // ── the control, FIRST: unarmed, this same access resolves. Without it a red below
    //    could mean "the tree was never built" rather than "the arm refused". ──
    assert_eq!(p.fb_trap_policy(), FbTrapPolicy::Serve, "default is serve");
    let (phys, _) = fb_read(&p, BAR_FB, BAR1_VA);
    assert_eq!(
        phys, SHARED_PHYS,
        "unarmed, the trap path must still translate — otherwise the refusal below is not \
         evidence about the ARM"
    );
    assert_eq!(
        p.fb_trap_refusals(),
        0,
        "nothing refused on the control arm"
    );

    // ── arm it ──
    p.set_fb_trap_policy(FbTrapPolicy::RefuseByName);
    let out = p.read(BAR_FB, BAR1_VA, 4);
    match out {
        kayfabe_device::plane::ReadOutcome::TranslationRefused { window, va, why } => {
            assert_eq!(window, kayfabe_device::FbWindow::FbAperture);
            assert_eq!(va, BAR1_VA);
            assert_eq!(
                why, FB_TRAP_REFUSED,
                "★ it must be THE refusal, by its own sentence. A translation fault that \
                 happened to share the variant would make a boot's zero mean something else \
                 entirely."
            );
        }
        other => panic!(
            "the armed trap path must refuse by name; it answered {other:?}. If this ever \
             passes silently, `FB_TRAP_REFUSALS=0` in a boot stops being evidence."
        ),
    }
    assert_eq!(
        p.fb_trap_refusals(),
        1,
        "★ and it must be COUNTED — the boot reads the counter, not the log"
    );

    // ── BAR2 too: both translated windows are armed, and PRAMIN deliberately is not ──
    let _ = p.read(BAR_INST, BAR2_VA, 4);
    assert_eq!(
        p.fb_trap_refusals(),
        2,
        "the instance window is armed as well"
    );

    // ⊘ PRAMIN is NOT armed: it is a control aperture the guest reads through immediately
    // after re-pointing it, it is the one sanctioned expensive trap, and it is not what
    // increment 7 deletes. Refusing it would fail the boot for an unrelated reason.
    pramin_wr32(&p, SHARED_PHYS, CONTROL_SENTINEL);
    assert_eq!(
        pramin_rd32(&p, SHARED_PHYS),
        CONTROL_SENTINEL,
        "★ PRAMIN must keep working with the arm on, or the measurement boot dies of \
         something that has nothing to do with BAR1/BAR2 publication"
    );
    assert_eq!(
        p.fb_trap_refusals(),
        2,
        "and PRAMIN must not have been counted as a refusal"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// ★★★★★ §3 CUT A — WHERE A `device`-STORE BOOT DIES, PINNED OFFLINE.
// ═══════════════════════════════════════════════════════════════════════════════════════

/// ★★★★★ **THE CUT-A WALL, AS A CHECKED PROPERTY RATHER THAN A READING OF THE CALL GRAPH.**
///
/// `SINGLE_STORE_PLAN.md`'s w735 block predicts that a boot on `KAYFABE_FB_STORE=device` dies
/// at the **first framebuffer access of any kind**, because `DeviceFb` refuses every
/// host-side `read` and `write` by name and the BAR1/BAR2 translate walks the page tables
/// through exactly those calls.
///
/// # ⊘⊘ Why this is worth a test and not a paragraph
///
/// That prediction is the reason **no bench box was rented for cut A** — a boot that dies
/// before installing one memslot measures nothing about the memslot half. A prediction load-
/// bearing enough to decide a spend should be checkable, and this plane can check the half
/// that does not need a GPU: *"a translate through this store is refused, and refused **by
/// the store's own name** rather than by some upstream `NoAddressModel`."*
///
/// ⊘ What it still cannot say: **when** in a real boot the first such access happens. That is
/// the third row of the prediction table and it stays a reading.
#[test]
fn a_bar1_translate_through_the_single_store_is_refused_and_the_store_is_what_refused() {
    let _census = CENSUS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    use kayfabe_device::DeviceFb;
    use kayfabe_device::fbwin::DEVICE_FB_READ_REFUSED;

    // ── the control, first: the SAME tree, on the arena store, translates ──
    let arena = plane();
    build_bar1_tree(&arena, BAR1_VA, leaf(SHARED_PHYS));
    let (phys, _) = fb_read(&arena, BAR_FB, BAR1_VA);
    assert_eq!(
        phys, SHARED_PHYS,
        "★ the control must TRANSLATE, or the refusal below would be about a tree that was \
         never built and this test would pass for the wrong reason"
    );

    // ── now the same plane with the single store installed ──
    let p = plane();
    build_bar1_tree(&p, BAR1_VA, leaf(SHARED_PHYS));
    // ⊘ Swapped in AFTER the tree is written, because writing it needs a store that accepts
    // writes. In a real boot the guest writes its tables through a device-view memslot and
    // nothing host-side ever sees them — which is precisely why cut B exists.
    p.set_fb(Box::new(DeviceFb::new(GA106.fb_length)));

    // ⊘ `window_page_backing` takes an `FbWindow`, not a BAR number: it is the PREMAP path
    // (`BarMirror::fill_now`'s phase 1), which is the one that would be asked for a memslot.
    let before = DEVICE_FB_READ_REFUSED.load(core::sync::atomic::Ordering::Relaxed);
    let r = p.window_page_backing(kayfabe_device::FbWindow::FbAperture, BAR1_VA, true);
    match r {
        Err(kayfabe_device::WindowRefusal::Translated { .. }) => {
            // ⊘⊘⊘ **AND THE STORE'S SENTENCE IS NOT IN IT — measured, w735.**
            //
            // The refusal that arrives reads *"the page-table decoder refused a level of this
            // walk"*: `FbRead::read_in` answers a **`bool`**, so `FbRefused::why` dies at
            // `m.fb.read(..).is_ok()` and the walker turns the miss into `WalkFault::Unbacked`.
            // ⇒ a cut-A boot's visible diagnosis names the **decoder**, which is working, and
            // not the store. That is a symptom naming the wrong subsystem — this tree's most
            // expensive recurring shape — so the store says its own name ONCE, on its own line,
            // and the count below is what joins the two.
            //
            // ⚠ Asserted **by the counter**, not by the sentence, precisely because the
            // sentence cannot travel. Changing `FbRead::read_in` to carry it is cut B's call:
            // every consumer of that trait would have to grow a reason it currently discards.
            assert_eq!(
                DEVICE_FB_READ_REFUSED.load(core::sync::atomic::Ordering::Relaxed),
                before + 1,
                "★ the refusal must have come from THE STORE. Without this the assertion \
                 above passes for any `Translated` refusal at all — including one from a \
                 tree that was never built, which is exactly the false green the control at \
                 the top of this test exists to rule out."
            );
        }
        other => panic!(
            "★★★ THE PREDICTION IS WRONG, AND THAT IS THE FINDING. A BAR1 translate through \
             the single store was expected to be refused by `DEVICE_HOST_READ_UNBUILT`; it \
             answered {other:?}. If it SUCCEEDED, the walk is not reading through \
             `FbStore::read` and cut B's consumer list is incomplete — re-read \
             `SINGLE_STORE_PLAN.md`'s w735 table before costing anything."
        ),
    }
}

/// ⊘ **And the store still NAMES the page, which is the half that works.** Stated beside the
/// refusal so nobody reads cut A as *"the device store does nothing"*.
#[test]
fn the_single_store_still_names_every_page_for_the_memslot_path() {
    use kayfabe_device::{DeviceFb, FbPageBacking, FbStore};
    let mut fb = DeviceFb::new(GA106.fb_length);
    assert_eq!(
        fb.page_backing(SHARED_PHYS, true),
        FbPageBacking::Device { at: SHARED_PHYS },
        "★ the memslot half needs only an ADDRESS and gets one without reading a byte — which \
         is why `fill_now` can arm a view and install a guest memslot over real video memory \
         while every host-side read is still refused"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// ★★★★★ §3 CUT B — ARM-THEN-RETRY, AT THE PLANE. `SINGLE_STORE_PLAN.md` cut B items 2–5.
// ═══════════════════════════════════════════════════════════════════════════════════════

mod fakeport;

/// Build BAR1's tree **inside the reserved object**, the way the guest really does it: through
/// a path the host has no CPU view of.
///
/// ⊘ Deliberately not [`build_bar1_tree`], which writes through PRAMIN and therefore through
/// the store. Under the single store the guest writes its BAR1 tables through a device-view
/// memslot and **nothing host-side sees them** — which is the whole reason cut B exists, and
/// a fixture that wrote them through the store would be testing the arena's path.
fn build_bar1_tree_in_the_object(port: &fakeport::FakePort, va: u64, leaf_entry: u64) {
    let root = GA106.bar1_pde_base;
    assert_ne!(root, 0, "this chip row must publish a bar1PdeBase");
    port.poke(root + ((va >> 30) & 511) * 8, &pde(B1_L1).to_le_bytes());
    port.poke(B1_L1 + ((va >> 21) & 511) * 8, &leaf_entry.to_le_bytes());
}

fn device_plane_with_port(port: &std::sync::Arc<fakeport::FakePort>) -> RegPlane {
    let p = plane();
    p.set_fb(Box::new(kayfabe_device::DeviceFb::with_port(
        GA106.fb_length,
        port.clone() as std::sync::Arc<dyn kayfabe_device::DeviceFbPort>,
    )));
    p
}

/// ★★★★★ **CUT B ITEM 2 — `PlanePtBytes::read_in` ARMS AND RETRIES, SYNCHRONOUSLY.**
///
/// `SINGLE_STORE_PLAN.md`'s w735 consumer table, first row: this reader takes the plane locks
/// *inside*, per read, so it is **lock-free at entry** and can arm without deferring anything.
/// ⇒ the caller above it never learns a page had to be armed.
///
/// ★ The known-positive is the **counter**, not just the `true`: a read that succeeded on the
/// first attempt would also return `true`, and this must fail if the retry stops running.
#[test]
fn a_page_table_read_arms_its_own_page_and_retries_without_deferring_anything() {
    use kayfabe_mmu::walker::FbRead;
    let port = std::sync::Arc::new(fakeport::FakePort::new(GA106.fb_length));
    port.poke(SHARED_PHYS, &[0xAB; 8]);
    let p = device_plane_with_port(&port);

    let before = kayfabe_device::plane::fb_demand_counts();
    let mut buf = [0u8; 8];
    assert!(
        p.pt_bytes()
            .read_in(SHARED_PHYS, kayfabe_arch::Aperture::Vidmem, &mut buf),
        "★★★ THE READ MUST SUCCEED — nothing was armed when it started, so the only way it \
         can is by arming and trying again. A `false` here means cut B item 2 is not wired."
    );
    assert_eq!(buf, [0xAB; 8], "and it must be the object's own bytes");
    let after = kayfabe_device::plane::fb_demand_counts();
    assert!(
        after.5 > before.5,
        "★ THE KNOWN-POSITIVE: `read_retried_ok` must move. Without it this test passes for \
         any read that happens to succeed, including one from a store that never refused."
    );
    assert!(
        after.1 > before.1,
        "and a page must actually have been armed"
    );
}

/// ⊘⊘⊘ **CUT B ITEM 5 — A DECLINED DRAIN ENDS THE RETRY, IT DOES NOT SPIN IT.**
///
/// The shell declines when the caller is a vCPU or inside an MMIO trap. ⚠ The dangerous shape
/// is not the failure; it is a **bounded loop that keeps going** because it cannot tell
/// *"declined"* from *"armed nothing this time"* — on a vCPU, inside an MMIO exit.
#[test]
fn a_declined_drain_ends_the_retry_rather_than_spinning_it() {
    use kayfabe_mmu::walker::FbRead;
    let port = std::sync::Arc::new(fakeport::FakePort::new(GA106.fb_length));
    port.poke(ALT_PHYS, &[0xCD; 8]);
    port.set_declining(true);
    let p = device_plane_with_port(&port);

    let mut buf = [0u8; 8];
    assert!(
        !p.pt_bytes()
            .read_in(ALT_PHYS, kayfabe_arch::Aperture::Vidmem, &mut buf),
        "a declined drain arms nothing, so the read must fail"
    );
    let (drains, declined, armed, ..) = port.counts();
    assert_eq!(armed, 0);
    assert_eq!(drains, 0, "no drain ever ran");
    assert_eq!(
        declined, 1,
        "★★★ THE KNOWN-POSITIVE AND THE BOUND IN ONE NUMBER: exactly ONE decline. A retry \
         that could not tell a decline from an empty drain would have produced one per \
         allowed attempt, which on a vCPU is the spin this design exists to avoid."
    );
}

/// ★★★★★ **CUT B ITEM 3 — A BAR1 TRANSLATE RESOLVES ONCE A LOCK-FREE CALLER ARMS.**
///
/// This is `BarMirror::fill_now`'s phase 1 and `resolve_arming`'s loop, written out at the
/// seam a test can reach: `window_page_backing` walks **under the plane lock** and cannot arm,
/// so it refuses; the caller is lock-free and arms; the next attempt resolves.
///
/// ⊘ The walk is one page-table level deeper on each pass, which is why the retry bound is the
/// format's depth and not a number somebody picked.
#[test]
fn a_bar1_translate_resolves_after_a_lock_free_caller_arms_the_pages_it_missed() {
    let _census = CENSUS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let port = std::sync::Arc::new(fakeport::FakePort::new(GA106.fb_length));
    let p = device_plane_with_port(&port);
    build_bar1_tree_in_the_object(&port, BAR1_VA, leaf(SHARED_PHYS));

    // ── the first attempt must REFUSE, or this test proves nothing about arming ──
    assert!(
        p.window_page_backing(FbWindow::FbAperture, BAR1_VA, true)
            .is_err(),
        "★ the control half: with nothing armed, the walk cannot read the guest's tables"
    );

    // ── `BarMirror::resolve_arming`'s loop, lock-free, bounded ──
    let mut res = Err(kayfabe_device::WindowRefusal::NoAddressModel);
    let mut used = 0u32;
    for _ in 0..kayfabe_mmu::walker::MAX_WALK_DEPTH {
        if !p.arm_fb_demand().progressed() {
            break;
        }
        used += 1;
        res = p.window_page_backing(FbWindow::FbAperture, BAR1_VA, true);
        if res.is_ok() {
            break;
        }
    }
    let r = res.expect(
        "★★★ the translate must resolve once its page-table pages are armed. If it does not, \
         cut B item 3's demand set is not recording what the walk missed.",
    );
    assert_eq!(r.phys, SHARED_PHYS, "and it must land where the tree says");
    assert_eq!(
        r.backing,
        kayfabe_device::FbPageBacking::Device { at: SHARED_PHYS },
        "and still name the page to the memslot path — cut B does not change cut A's half"
    );
    assert!(
        used >= 2,
        "★ THE KNOWN-POSITIVE FOR THE RETRY BOUND BEING A DEPTH: a two-level tree needs TWO \
         arming passes, because the second level's address is only known once the first is \
         readable. `used={used}` of a bound of {}",
        kayfabe_mmu::walker::MAX_WALK_DEPTH
    );
}

/// ⊘⊘⊘ **CUT B ITEM 4 — AND THE DEFECT IT UNCOVERED: AN ENUMERATION THAT CAME BACK EMPTY AND
/// CALLED IT `Ok`.**
///
/// `SINGLE_STORE_PLAN.md` cut B item 4 says *"`window_leaves` refuses the whole subtree at the
/// first unbacked page"*. ⊘ **It does not refuse.** `decode_subtree` returns `Err` for budget
/// exhaustion **and nothing else**; an unreadable page is a per-branch `WalkFault` and the walk
/// continues. So the failing shape is `Ok` with a **short** list — and with an unreadable
/// ROOT, `Ok` with an **empty** one, which reads as *"the guest has mapped nothing"*.
///
/// ⚠ It could never fire under the arena store, whose `read` answers every in-range address.
/// That is why it survived: the one arm that can produce it is the one that did not exist.
#[test]
fn an_unarmed_enumeration_comes_back_short_and_says_so_instead_of_reading_as_empty() {
    let port = std::sync::Arc::new(fakeport::FakePort::new(GA106.fb_length));
    let p = device_plane_with_port(&port);
    build_bar1_tree_in_the_object(&port, BAR1_VA, leaf(SHARED_PHYS));

    let e = p
        .window_leaves(FbWindow::FbAperture, 4096)
        .expect("⊘ NOT a refusal — that is the finding: an unreadable root is `Ok`");
    assert!(
        e.leaves.is_empty(),
        "with nothing armed, the root page cannot be read and no leaf can be found"
    );
    assert!(
        e.faults > 0,
        "★★★★★ THE KNOWN-POSITIVE, AND THE WHOLE POINT OF THE FIELD: without `faults` this \
         empty list is indistinguishable from a guest that mapped nothing, and premap would \
         publish zero pages and count zero refusals."
    );

    // ── now the caller's half: arm, retry, and the leaves arrive ──
    let mut got = e;
    for _ in 0..kayfabe_mmu::walker::MAX_WALK_DEPTH {
        if got.faults == 0 {
            break;
        }
        if !p.arm_fb_demand().progressed() {
            break;
        }
        got = p
            .window_leaves(FbWindow::FbAperture, 4096)
            .expect("still Ok");
    }
    assert_eq!(got.faults, 0, "every branch must be readable once armed");
    assert_eq!(
        got.leaves.len(),
        1,
        "★ and the leaf the tree declares must be there — the premap path's whole input"
    );
    assert_eq!(got.leaves[0].phys, SHARED_PHYS);
}

/// ⊘ **THE ARENA ARM IS UNTOUCHED, AND THIS IS WHAT SAYS SO.**
///
/// Every assertion above is about the `device` arm. `KAYFABE_FB_STORE` unset means a
/// `SparseFb`, which answers every in-range address — so the fault count must be **zero** and
/// the arming path must never even find a port. ⚠ Asserted rather than assumed: *"the default
/// arm is byte-identical"* is a claim with a known way of being wrong, and `faults` is a new
/// field on a struct the default arm also returns.
#[test]
fn the_arena_arm_enumerates_with_no_faults_and_has_no_byte_port_at_all() {
    let p = plane();
    build_bar1_tree(&p, BAR1_VA, leaf(SHARED_PHYS));
    let e = p
        .window_leaves(FbWindow::FbAperture, 4096)
        .expect("the arena arm enumerates");
    assert_eq!(
        e.faults, 0,
        "★ a fault on the arena arm would be a real page-table finding, and there is none here"
    );
    assert_eq!(e.leaves.len(), 1);
    assert!(
        p.fb_demand_port().is_none(),
        "⊘ and the default store hands out NO byte port, so `arm_fb_demand` can never arm \
         anything on this arm — which is what `no_port` in the FB-DEMAND census counts"
    );
    let before = kayfabe_device::plane::fb_demand_counts();
    assert!(!p.arm_fb_demand().progressed());
    let after = kayfabe_device::plane::fb_demand_counts();
    assert!(
        after.4 > before.4,
        "★ and it is counted as `no_port`, never as a refusal or a decline — three states, \
         three numbers"
    );
    assert_eq!(after.0, before.0, "and no drain ran");
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// ★★★★★ §3 CUT C — THE WRITE HALF. `SINGLE_STORE_PLAN.md`'s cut-C block.
// ═══════════════════════════════════════════════════════════════════════════════════════

/// Build BAR2's directory page **inside the reserved object**, and publish its root the way
/// the guest does.
///
/// ⊘ [`build_and_publish_bar2_tree`]'s twin for the single store, for
/// [`build_bar1_tree_in_the_object`]'s reason exactly: writing the page through PRAMIN would
/// write it through the store, which is the arena's path and not the one under test.
fn build_bar2_tree_in_the_object(
    p: &RegPlane,
    port: &fakeport::FakePort,
    va: u64,
    leaf_entry: u64,
) {
    port.poke(B2_L1 + ((va >> 21) & 511) * 8, &leaf_entry.to_le_bytes());
    publish_bar2_root(p);
}

/// Run `f` until it says it is good, arming between attempts — the **caller's** half of cut
/// B, reproduced here so a test drives the same shape production does.
///
/// ⚠ **A FIXED TRIP COUNT** (`THE_CONSTRAINTS.md` §20, invariant 1), and it ends the moment a
/// drain arms nothing: exactly [`BarMirror::arm_then_retry`]'s two rules. ⊘ A test helper that
/// looped until success would pass on a build where arming does nothing at all, by spinning.
fn with_arming<T>(p: &RegPlane, mut f: impl FnMut() -> (T, bool)) -> T {
    const TRIPS: u32 = 8;
    let (mut value, mut good) = f();
    for _ in 0..TRIPS {
        if good || !p.arm_fb_demand().progressed() {
            break;
        }
        let (v, g) = f();
        value = v;
        good = g;
    }
    value
}

/// ★★★★★ **CUT C's FIRST DEFECT — `decode_subtree_from_entry` DROPPED ITS FAULTS, AND BAR2
/// IS THE ONLY WINDOW THAT USES IT.**
///
/// # ⊘⊘⊘ What was wrong, and why cut B's own fix could not see it
///
/// Cut B item 4 made [`RegPlane::window_leaves`] **return** `faults`, so that an enumeration
/// whose page-table pages could not be read out of the single store stops reading as *"the
/// guest has mapped nothing"*. `[established from the source, w739]` for BAR1 that worked —
/// `decode_subtree` pushes a `WalkFault` per unreadable branch. For **BAR2 it could not**:
/// BAR2's root is a raw PDE *entry*, so it goes through `decode_subtree_from_entry`, and that
/// function extended `leaves` and `visited` from each sub-walk and **dropped `sub.faults` on
/// the floor**. ⇒ `faults` was structurally pinned at `0` on the one window `kbusVerifyBar2`
/// exercises.
///
/// ★★★ **And that is what made cut B inert on BAR2.** `BarMirror::premap_window` treats
/// `Ok && faults == 0` as *good* and stops; a first attempt that came back `Ok` with an
/// **EMPTY** leaf list therefore armed nothing and retried nothing.
/// `[measured w738, the cut-B boot]` `premap[runs=6 filled=0 … bar2_visited=0 pt_faults=0]`
/// with `named=0` and `BAR2 (translated): … 14 REFUSED by name` — the census cut B added to
/// close the empty-artefact class, reporting a zero its own plumbing guaranteed.
///
/// ⚠ **The known-positive is the fault, not the empty list.** An empty list is what the defect
/// produced *and* what a genuinely unmapped BAR2 produces; only `faults > 0` separates them,
/// which is the whole point of the field.
#[test]
fn an_unreadable_bar2_directory_page_reports_a_fault_rather_than_an_empty_tree() {
    let port = std::sync::Arc::new(fakeport::FakePort::new(GA106.fb_length));
    let p = device_plane_with_port(&port);
    build_bar2_tree_in_the_object(&p, &port, BAR2_VA, leaf(SHARED_PHYS));

    // ── nothing is armed, so the directory page cannot be read out of the object ──
    let e = p
        .window_leaves(FbWindow::InstanceWindow, 4096)
        .expect("the enumeration is `Ok` — a page it cannot read is per-branch, never terminal");
    assert!(
        e.leaves.is_empty(),
        "setup: with nothing armed the walk reaches no leaf, or the assertion below is not \
         about an unreadable page"
    );
    assert!(
        e.faults >= 1,
        "★★★★★ THE DEFECT: an enumeration that could not read BAR2's directory page must say \
         so. It answered `Ok` with {} leaves and faults={} — an EMPTY LIST THAT READS AS 'the \
         guest has mapped nothing', published as fact. `premap_window` treats `faults == 0` as \
         good, so this zero is what stopped cut B from ever arming anything on BAR2 \
         (w738: `bar2_visited=0 pt_faults=0 named=0` on a boot with fourteen refused BAR2 \
         accesses).",
        e.leaves.len(),
        e.faults
    );

    // ── and the repair is distinguishable from the defect: arm, and the SAME call succeeds ──
    let e2 = with_arming(&p, || {
        let got = p
            .window_leaves(FbWindow::InstanceWindow, 4096)
            .expect("still `Ok`");
        let good = got.faults == 0;
        (got, good)
    });
    assert_eq!(
        e2.faults, 0,
        "★ after arming, the same enumeration must come back clean — otherwise `faults > 0` \
         above would be a permanent state and the retry would be a spin"
    );
    assert_eq!(
        e2.leaves.len(),
        1,
        "★★★ and it must now find the leaf the guest really mapped. This is the number \
         `premap[filled=]` is made of, and it was ZERO on the w738 boot."
    );
    assert_eq!(e2.leaves[0].phys, SHARED_PHYS);
}

/// ⊘ **THE ARENA CONTROL FOR THE FIX ABOVE.** `SparseFb` answers every in-range address, so a
/// BAR2 enumeration on the default arm must report **no** fault — the propagated vector must
/// be empty, not merely returned.
///
/// ⚠ Without this, *"faults now travel"* could be satisfied by a change that reports a fault
/// for every branch on every arm, which would make the control print `PREMAP ⊘⊘ came back
/// SHORT` on a boot where nothing is short.
#[test]
fn the_arena_arm_enumerates_bar2_with_no_faults_at_all() {
    let p = plane();
    build_and_publish_bar2_tree(&p, BAR2_VA, leaf(SHARED_PHYS));
    let e = p
        .window_leaves(FbWindow::InstanceWindow, 4096)
        .expect("the arena arm enumerates BAR2");
    assert_eq!(
        e.faults, 0,
        "★ the default arm must stay byte-identical: a fault here would be new output on a \
         boot nothing changed for"
    );
    assert_eq!(e.leaves.len(), 1, "and it must still find the leaf");
    assert_eq!(e.leaves[0].phys, SHARED_PHYS);
}

/// ★★★★★ **CUT C's SECOND DEFECT, AS A GATE — THE REPAIR PATH IS REACHABLE ONLY ON THE
/// `device` ARM, AND IT IS REACHABLE.**
///
/// `Regs::read` queued a fill only on `ReadOutcome::Fb` and `Regs::write` only on
/// `fb_landed.is_some()` — *"the access worked"*. Under the single store the **first** access
/// to any BAR1/BAR2 page is refused, so the one path that could install a memslot was
/// reachable only from the state in which the page was already fine.
/// `[measured w738]` `BAR-MIRROR FILLS queued=0 run=0 dropped=0` beside fourteen refused BAR2
/// accesses and `named=0`.
///
/// ⊘ `BarMirror` needs a `QemuMachine` and cannot be built in a `cargo test`, so what is
/// pinned here is the **gate** the shell asks before it queues: the predicate is `false` on
/// the arena arm — which is what makes the default arm byte-identical by construction rather
/// than by inspection — and `true` the moment a byte port is installed.
#[test]
fn the_refused_access_repair_gate_is_false_on_the_arena_arm_and_true_on_the_device_arm() {
    let arena = plane();
    assert!(
        !arena.fb_has_demand_port(),
        "⊘ THE CONTROL: with `KAYFABE_FB_STORE` unset the store hands out no byte port, so \
         cut C's repair arm is UNREACHABLE and the arena arm's fill traffic is unchanged"
    );

    let port = std::sync::Arc::new(fakeport::FakePort::new(GA106.fb_length));
    let dev = device_plane_with_port(&port);
    assert!(
        dev.fb_has_demand_port(),
        "★★★ THE KNOWN-POSITIVE: the gate must actually open on the single store. A predicate \
         that were always false would make cut C's whole repair path dead code, and the only \
         symptom would be a boot that still refuses every page — which is what w738 measured \
         and is indistinguishable from the defect it fixes."
    );
    assert!(
        dev.fb_demand_port().is_some(),
        "and the cached flag must agree with the store itself; two projections of one fact \
         that can disagree is this tree's named shape"
    );
}

/// ★★★★★ **THE SINGLE STORE'S FALSIFIER, ON THE `device` ARM — and it was VACUOUS there
/// until now.**
///
/// [`a_framebuffer_page_written_through_bar1_is_the_page_bar2_reads`] is named in
/// `SINGLE_STORE_PLAN.md` as the falsifier the single store must satisfy. ⊘ **It runs on
/// `SparseFb`** — the *arena* store, the one the single store replaces — so it says nothing
/// whatever about `DeviceFb`, and a `device` arm that split BAR1 from BAR2 would leave it
/// green. `[established from the source, w739]` This is that test's device-arm twin, and it
/// is what makes the name mean something on the arm it is quoted about.
///
/// # ⊘ What it pins, and what it still cannot
///
/// **Identity**: one framebuffer address, one memory, across all three windows — a write
/// through BAR1 is what BAR2 and PRAMIN read. ⊘ **Not residence**: [`fakeport::FakePort`]'s
/// bytes are host memory in this process, and whether the real port's bytes are device-local
/// is §18's property and not a question `cargo test` can ask. The module docs above say the
/// same of the arena twin.
///
/// ★ It is a **write-half** test by construction: the BAR1 write must reach
/// `DeviceFb::write` → `DeviceFbPort::write_armed` and land in the object, or the BAR2 read
/// below answers zero — which is exactly `kbusVerifyBar2_GM107`'s `returned garbage 0x0`,
/// reproduced offline.
#[test]
fn a_framebuffer_page_written_through_bar1_is_the_page_bar2_reads_on_the_device_arm() {
    let port = std::sync::Arc::new(fakeport::FakePort::new(GA106.fb_length));
    let p = device_plane_with_port(&port);
    build_bar1_tree_in_the_object(&port, BAR1_VA, leaf(SHARED_PHYS));
    build_bar2_tree_in_the_object(&p, &port, BAR2_VA, leaf(SHARED_PHYS));

    // ── setup, asserted first: both apertures really do name ONE framebuffer address ──
    let b1 = with_arming(&p, || {
        let r = p.read(BAR_FB, BAR1_VA, 4);
        let good = matches!(r, ReadOutcome::Fb { .. });
        (r, good)
    });
    let ReadOutcome::Fb { phys: b1_phys, .. } = b1 else {
        panic!(
            "BAR1 must translate through the single store after arming; it answered {b1:?}. \
             That is a SETUP failure — cut B item 3's arm-then-retry, not the write half."
        )
    };
    assert_eq!(b1_phys, SHARED_PHYS);
    let b2 = with_arming(&p, || {
        let r = p.read(BAR_INST, BAR2_VA, 4);
        let good = matches!(r, ReadOutcome::Fb { .. });
        (r, good)
    });
    let ReadOutcome::Fb { phys: b2_phys, .. } = b2 else {
        panic!("BAR2 must translate to the same address; it answered {b2:?}")
    };
    assert_eq!(
        b2_phys, SHARED_PHYS,
        "★ the two apertures must resolve to ONE framebuffer address, or the agreement below \
         is about two unrelated addresses and means nothing"
    );

    // ── THE WRITE HALF: through BAR1, into the reserved object ──
    let w = with_arming(&p, || {
        let o = p.write(BAR_FB, BAR1_VA, 4, u64::from(DEVICE_SENTINEL));
        let good = o.fb_landed.is_some();
        (o, good)
    });
    assert_eq!(
        w.fb_landed,
        Some(SHARED_PHYS),
        "★★★★★ THE WRITE MUST LAND IN THE OBJECT. A dropped write here is precisely what the \
         w738 boot measured — `BAR2 (translated): … 14 REFUSED by name`, `the bytes did NOT \
         land anywhere`, `DEVICE-FB wanted_by_write=0` — and it is what `kbusVerifyBar2` \
         reports, ninety lines later, as `returned garbage 0x0`."
    );
    assert_eq!(
        port.peek(SHARED_PHYS, 4),
        DEVICE_SENTINEL.to_le_bytes().to_vec(),
        "★ and it must be in the OBJECT's own bytes, read past every armed-run check — not \
         merely reported as landed"
    );

    // ── THE PROPERTY: BAR2 reads what BAR1 wrote ──
    let r2 = with_arming(&p, || {
        let r = p.read(BAR_INST, BAR2_VA, 4);
        let good = matches!(r, ReadOutcome::Fb { .. });
        (r, good)
    });
    let ReadOutcome::Fb { value, .. } = r2 else {
        panic!("BAR2 must still translate; it answered {r2:?}")
    };
    assert_eq!(
        value as u32, DEVICE_SENTINEL,
        "★★★★★ THE SINGLE STORE, ON THE `device` ARM: BAR1 and BAR2 are views of ONE reserved \
         object, so a write through BAR1 at {SHARED_PHYS:#x} must be visible through BAR2 at \
         the same address. BAR2 read {value:#x}. A zero here IS `kbusVerifyBar2`'s `garbage \
         0x0`: two memories for one address, which is the defect the reserved object exists \
         to delete."
    );

    // ── and PRAMIN, the third view, sees it too ──
    let pr = with_arming(&p, || {
        point_window(&p, SHARED_PHYS);
        let r = p.read(
            BAR_REGS,
            GA106.pramin_window.base + (SHARED_PHYS & 0xFFFF),
            4,
        );
        let good = matches!(r, ReadOutcome::Fb { .. });
        (r, good)
    });
    let ReadOutcome::Fb { value: pv, .. } = pr else {
        panic!("PRAMIN must read the object; it answered {pr:?}")
    };
    assert_eq!(
        pv as u32, DEVICE_SENTINEL,
        "★ PRAMIN is the third view of the same object, and it is the one RM reads back \
         through in `kbusVerifyBar2_GM107`: it writes through BAR2 and reads through the BAR0 \
         window. A split that left PRAMIN behind is the same defect one aperture over."
    );
}

/// ⊘⊘ **THE WRITE HALF REFUSES BY NAME WHEN NOTHING IS ARMED — AND RECORDS THE DEMAND.**
///
/// ⚠ `[measured w738]` `DEVICE-FB wanted_by_write=0` was read as *"no host-side write was
/// ever needed"*. It meant the opposite: `RegPlane::fb_write` refused at **translation**, so
/// the store was never asked at all. ⇒ this pins the half that IS the store's — that a write
/// which reaches it with nothing armed lands **nowhere**, says so, and leaves a want behind
/// for a lock-free caller — so that a future `wanted_by_write=0` is evidence about the
/// caller and not about this.
#[test]
fn an_unarmed_write_lands_nowhere_and_leaves_a_want_behind() {
    use core::sync::atomic::Ordering::Relaxed;
    use kayfabe_device::DeviceFbPort;
    use kayfabe_device::fbwin::DEVICE_FB_WANTED_BY_WRITE;

    let port = std::sync::Arc::new(fakeport::FakePort::new(GA106.fb_length));
    let mut fb = kayfabe_device::DeviceFb::with_port(
        GA106.fb_length,
        port.clone() as std::sync::Arc<dyn kayfabe_device::DeviceFbPort>,
    );
    let before = DEVICE_FB_WANTED_BY_WRITE.load(Relaxed);
    let err = kayfabe_device::FbStore::write(&mut fb, SHARED_PHYS, &[0xEE; 4])
        .expect_err("nothing is armed, so the write must be refused");
    assert_eq!(err.phys, SHARED_PHYS);
    assert_eq!(
        port.peek(SHARED_PHYS, 4),
        vec![0u8; 4],
        "⊘ THE ONE ANSWER THAT MUST NOT EXIST: a refused write that changed bytes anyway. \
         There is no success-shaped answer and there must be no success-shaped side effect."
    );
    assert!(
        DEVICE_FB_WANTED_BY_WRITE.load(Relaxed) > before,
        "★ and the demand must be RECORDED, or `wanted_by_write` stays a number that cannot \
         distinguish `no write was wanted` from `the write never reached the store`"
    );
    assert_eq!(port.wanted_now(), 1, "one run is now wanted");

    // ── the known-positive: after a drain the SAME write lands, in the object ──
    assert!(
        port.drain().progressed(),
        "the drain must arm the wanted run"
    );
    kayfabe_device::FbStore::write(&mut fb, SHARED_PHYS, &[0xEE; 4])
        .expect("★ armed, the same write must land — otherwise the refusal above is permanent");
    assert_eq!(port.peek(SHARED_PHYS, 4), vec![0xEE; 4]);
}
