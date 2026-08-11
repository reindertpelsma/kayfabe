//! ★★★★★ **§5.12 — ONE MEMORY for a framebuffer range**, and the establishment copy that
//! makes the ordering safe by construction (`fb_cpu_view.md` §4).
//!
//! `w228` backed three framebuffer operands with real card memory at the guest's own
//! addresses — **blank, and with no view the guest shares**. Under execution the engine reads
//! the real object while the guest reads the emulator's fabricated one: anything the guest
//! wrote appears as zeros, anything the engine writes is invisible. ⚠ Silent in **both**
//! directions — no fault, no error, no status.
//!
//! This file is that defect's falsifier at the layer where the bytes live. ⊘ It cannot say
//! anything about RM, the GPU MMU or the isolate; the second half of the join is measured on
//! real hardware and reported in `fb_cpu_view.md`. What it *can* say is the half that no
//! hardware run would isolate: that the store stops answering from its own pages the instant a
//! range is joined, that the bytes already in it come across **first**, and that a device life
//! cannot leak them into the next one.

use kayfabe_device::{
    ALREADY_JOINED, ESTABLISH_FAILED, FbJoined, FbStore, FbWriter, NO_JOIN_SUPPORT, RefusingFb,
    SparseFb,
};

/// The advertised framebuffer. Small; nothing here depends on the size.
const FB: u64 = 0x100_0000;

/// A joined range's base and length — a whole 64 KiB granule, the unit RM can place exactly.
const AT: u64 = 0x40_0000;
const LEN: u64 = 0x1_0000;

/// ★★★ **A stand-in for the isolate's mapping** — a plain `Vec` a second party is imagined to
/// hold.
///
/// ⊘ It is deliberately **not** a second `SparseFb`, and not a mapping of anything. What is
/// under test here is the store's *routing and establishment*, not `mmap`; the production
/// implementation over a `MappedRegion` lives in `kayfabe-qemu-raw` and is four lines whose
/// only content is that the bytes are somebody else's. A fixture that modelled the mapping
/// would be modelling the one part that cannot be got wrong here.
#[derive(Debug)]
struct Elsewhere {
    bytes: Vec<u8>,
    /// ★ When set, every write refuses — the establishment copy's failure arm, which is
    /// otherwise unreachable and is the arm on which the join must NOT be installed.
    refuse_writes: bool,
}

impl Elsewhere {
    fn new(len: u64) -> Elsewhere {
        Elsewhere {
            bytes: vec![0u8; len as usize],
            refuse_writes: false,
        }
    }
}

const OUT: &str = "outside the fixture's extent";

impl FbJoined for Elsewhere {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read(&self, off: u64, buf: &mut [u8]) -> Result<(), &'static str> {
        let at = usize::try_from(off).map_err(|_| OUT)?;
        let end = at.checked_add(buf.len()).ok_or(OUT)?;
        buf.copy_from_slice(self.bytes.get(at..end).ok_or(OUT)?);
        Ok(())
    }

    fn write(&mut self, off: u64, bytes: &[u8]) -> Result<(), &'static str> {
        if self.refuse_writes {
            return Err("this fixture refuses every write");
        }
        let at = usize::try_from(off).map_err(|_| OUT)?;
        let end = at.checked_add(bytes.len()).ok_or(OUT)?;
        self.bytes
            .get_mut(at..end)
            .ok_or(OUT)?
            .copy_from_slice(bytes);
        Ok(())
    }
}

/// A per-word image, so a read that returned a zero fill, a truncated length or a different
/// buffer's bytes cannot match. ⊘ Never a repeated constant: a whole-buffer compare against
/// one repeated word passes on any single correct word.
fn image(base: u32, len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len];
    for (i, w) in v.chunks_exact_mut(4).enumerate() {
        w.copy_from_slice(&base.wrapping_add(i as u32).to_le_bytes());
    }
    v
}

// =====================================================================================
// 1 — ★★★★★ THE ESTABLISHMENT COPY
// =====================================================================================

/// ★★★★★ **The bytes the guest wrote BEFORE the backing existed are in the join afterwards.**
///
/// This is the owner's objection answered structurally. *"Mapping after execution seems racy
/// to me"* is correct: once the engine has written the real object and the guest has written
/// the fabricated one there is **no correct merge** — a merge is a choice about which writes
/// to lose. With the copy at install there is one memory from that instant, and never a merge.
#[test]
fn bytes_already_in_the_store_are_visible_through_the_join_afterwards() {
    let mut fb = SparseFb::new(FB);
    // The guest writes, through the ordinary window path, before anything is joined.
    let early = image(0xa19a_5a5b, 0x2000);
    fb.write_tagged(AT + 0x1000, &early, FbWriter::Executor)
        .expect("inside the advertised framebuffer");

    let elsewhere = Box::new(Elsewhere::new(LEN));
    let est = fb.install_join(AT, elsewhere).expect("the join installs");

    // ★★ Non-vacuity, asserted rather than assumed. An establishment copy of an all-zero
    // range is correct and proves nothing; a report that could not tell the two apart would
    // let a vacuous run read as evidence.
    assert!(
        est.nonzero > 0,
        "the copy must have moved NON-ZERO bytes, or this test proves nothing: {est:?}"
    );
    assert_eq!(est.pages, 2, "two resident pages were copied: {est:?}");
    assert_eq!(est.copied, 0x2000, "and all of their bytes: {est:?}");

    // And they read back through the store, which now answers from the join.
    let mut got = vec![0u8; 0x2000];
    fb.read(AT + 0x1000, &mut got).expect("reads");
    assert_eq!(got, early, "the guest's own earlier bytes came across");
}

/// ★★★ **Only RESIDENT pages are copied, and the count says so.**
///
/// A page the store never held is a page nothing ever wrote, and the fabricated backing is
/// already zero-filled by its own `ftruncate`. ⊘ Counting those zeros as copied bytes would
/// make **every** establishment report non-vacuous, which is exactly the reading that must
/// stay impossible.
#[test]
fn an_untouched_leaf_copies_nothing_and_says_so() {
    let mut fb = SparseFb::new(FB);
    let est = fb
        .install_join(AT, Box::new(Elsewhere::new(LEN)))
        .expect("installs");
    assert_eq!(
        (est.copied, est.nonzero, est.pages),
        (0, 0, 0),
        "nothing was resident, so nothing was copied — and the report must not round that \
         up to 'the copy ran': {est:?}"
    );
}

/// ★★★ **A copy that could not be performed does NOT leave a live join.**
///
/// A live join whose pre-existing bytes never arrived would present the engine a blank pool
/// for a leaf the guest has already written — `w228`'s defect, moved one layer along. ⊘ So the
/// refusal is total: the store keeps its own pages and keeps answering from them.
#[test]
fn a_failed_establishment_copy_installs_no_join_at_all() {
    let mut fb = SparseFb::new(FB);
    let early = image(0x1234_5678, 0x1000);
    fb.write(AT, &early).expect("writes");

    let mut hostile = Elsewhere::new(LEN);
    hostile.refuse_writes = true;
    let e = fb
        .install_join(AT, Box::new(hostile))
        .expect_err("the install must refuse");
    assert_eq!(e.why, ESTABLISH_FAILED, "refused by name");

    assert!(fb.joined_ranges().is_empty(), "no range went live");
    let mut got = vec![0u8; 0x1000];
    fb.read(AT, &mut got).expect("reads");
    assert_eq!(got, early, "the store still answers, from its own pages");
    assert_eq!(
        fb.is_resident(AT),
        Some(true),
        "and it still holds the page it was about to give away"
    );
}

// =====================================================================================
// 2 — ★★★★★ ONE MEMORY, in both directions
// =====================================================================================

/// ★★★★★ **The store stops holding its own copy the instant a range is joined.**
///
/// Both directions, over the store's own read/write path — which is the path the BAR0 moving
/// window takes, so this is what the guest sees.
#[test]
fn a_joined_range_carries_bytes_in_both_directions_and_the_store_keeps_no_copy() {
    let mut fb = SparseFb::new(FB);
    fb.write(AT, &image(0xdead_0000, 0x1000)).expect("writes");
    fb.install_join(AT, Box::new(Elsewhere::new(LEN)))
        .expect("installs");

    // ⊘ THE PAGE IS GONE from the store's own map. A store that kept it would have two
    // memories one layer down: its own stale page and the joined backing.
    assert_eq!(
        fb.is_resident(AT),
        Some(false),
        "the local page must be released at install — keeping it re-creates the defect"
    );
    assert_eq!(
        fb.page_origin(AT),
        None,
        "and its first-writer row with it, or a census would name a page it cannot see"
    );

    // guest → elsewhere, and back.
    let pattern = image(0x0f0f_0001, 0x800);
    fb.write(AT + 0x400, &pattern).expect("writes through");
    let mut got = vec![0u8; 0x800];
    fb.read(AT + 0x400, &mut got).expect("reads back");
    assert_eq!(got, pattern);

    // Everything OUTSIDE the join is untouched and still sparse — the join replaces the
    // store's pages for one range, never its lookup.
    let outside = image(0xbeef_0000, 0x100);
    fb.write(AT + LEN, &outside).expect("writes");
    assert_eq!(fb.is_resident(AT + LEN), Some(true), "still sparse-backed");
    let mut o = vec![0u8; 0x100];
    fb.read(AT + LEN, &mut o).expect("reads");
    assert_eq!(o, outside);
}

/// ★★ **An access that straddles the join's edge is NOT split between the two stores.**
///
/// A read half-served from the joined mapping and half from a local page is two memories
/// inside one access. It falls through to the sparse path instead, where the joined half reads
/// as this store's own bytes — wrong, and *loudly* wrong at the first comparison, rather than
/// subtly right.
#[test]
fn an_access_straddling_the_joins_edge_is_not_half_served() {
    let mut fb = SparseFb::new(FB);
    fb.install_join(AT, Box::new(Elsewhere::new(LEN)))
        .expect("installs");
    fb.write(AT, &image(0x1111_0000, 0x40))
        .expect("wholly inside — served by the join");

    let mut straddle = vec![0u8; 0x80];
    fb.read(AT + LEN - 0x40, &mut straddle)
        .expect("the sparse path answers it");
    assert_eq!(
        straddle,
        vec![0u8; 0x80],
        "⊘ served from this store's own (absent, therefore zero) pages — never spliced"
    );
}

// =====================================================================================
// 3 — ★★★ THE REFUSALS
// =====================================================================================

/// ★★★ Two joins over one byte is two memories for that byte.
#[test]
fn a_second_join_overlapping_the_first_is_refused_by_name() {
    let mut fb = SparseFb::new(FB);
    fb.install_join(AT, Box::new(Elsewhere::new(LEN)))
        .expect("installs");
    for (at, len) in [(AT, LEN), (AT + 0x1000, LEN), (AT - 0x1000, LEN)] {
        let e = fb
            .install_join(at, Box::new(Elsewhere::new(len)))
            .expect_err("overlap refused");
        assert_eq!(e.why, ALREADY_JOINED, "at 0x{at:x}");
    }
    assert_eq!(fb.joined_ranges(), vec![(AT, LEN)], "exactly one join");
}

/// ★★ A range outside the framebuffer this chip advertises cannot be joined either — the
/// guest was never promised it.
#[test]
fn a_join_outside_the_advertised_framebuffer_is_refused() {
    let mut fb = SparseFb::new(FB);
    let e = fb
        .install_join(FB - 0x1000, Box::new(Elsewhere::new(LEN)))
        .expect_err("refused");
    assert_eq!(e.why, kayfabe_device::fbwin::OUTSIDE_FRAMEBUFFER);
}

/// ★★ A store with no pages of its own has nothing to establish FROM and refuses by name,
/// rather than installing a join whose establishment silently did nothing.
#[test]
fn a_store_that_cannot_join_refuses_by_name() {
    let mut none = RefusingFb;
    let e = none
        .install_join(AT, Box::new(Elsewhere::new(LEN)))
        .expect_err("refused");
    assert_eq!(e.why, NO_JOIN_SUPPORT);
    assert!(none.joined_ranges().is_empty());
}

// =====================================================================================
// 4 — ★★★★★ THE CROSS-LIFE LEAK
// =====================================================================================

/// ★★★★★ **A device reset forgets the joins**, and this is the arm `fb_cpu_view.md` §4.3
/// names as a cross-life leak if it is missed.
///
/// A joined range that survived a device life would be the **previous** guest's framebuffer
/// content — its page tables, instance blocks and semaphores — still mapped by an isolate and
/// readable by the next guest through this very window. ⊘ Unlike a stale local page it is not
/// even this process's memory to have kept.
#[test]
fn device_reset_forgets_joined_ranges_and_not_only_local_pages() {
    let mut fb = SparseFb::new(FB);
    fb.install_join(AT, Box::new(Elsewhere::new(LEN)))
        .expect("installs");
    fb.write(AT, &image(0x5ec8_e700, 0x40)).expect("writes");
    fb.write(AT + LEN, &image(0x1000_0000, 0x40))
        .expect("and one outside");

    fb.device_reset();

    assert!(
        fb.joined_ranges().is_empty(),
        "★ the joins go with the bytes — #130 quantifies over ALL of the device's state"
    );
    assert_eq!(fb.resident_bytes(), 0, "and so do the local pages");
    // The range reads as a scrubbed board again, out of this store's own (absent) pages.
    let mut got = vec![0u8; 0x40];
    fb.read(AT, &mut got).expect("reads");
    assert_eq!(got, vec![0u8; 0x40]);
}
