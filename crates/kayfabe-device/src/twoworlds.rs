//! ★★★★★ **ARE THE TWO WORLDS ACTUALLY DISJOINT? — the measurement the split rests on.**
//!
//! **Owner, 2026-09-13:** *"We had a strict two world seperation since we discovered that bar2
//! is only PT\*/PD\* mappings (control pages) and these are not used in bar1."*
//!
//! `THE_CONSTRAINTS.md` §15 splits framebuffer backing by **aperture**: BAR2 and PRAMIN are
//! served by the fake fb (one sparse memfd), BAR1 by the reserved device-local object. That is
//! sound **iff** no framebuffer address is ever reached through both — otherwise one GPGA has
//! two memories, the guest writes one and reads the other, and nothing faults.
//!
//! # ⊘⊘⊘ WHY THIS IS NOT ALREADY KNOWN, AND WHY THE DESIGN DOC SAYS THE OPPOSITE
//!
//! `gpga_is_one_reserved_object.md` (owner's, 2026-09-10) rules it out **by address**:
//!
//! > *"Below the firmware carve-out the framebuffer is an undifferentiated heap: page tables
//! > and user buffers are **indistinguishable by address**, because RM allocates both from it.
//! > So backing cannot be decided by where something is. It is decided by **who reads it**."*
//!
//! ⇒ The two documents disagree, and they disagree about the load-bearing premise. The doc is
//! right about what the hardware *permits*; the constraint is a claim about what RM *does*.
//! Only a measurement separates them, and neither document contains one.
//!
//! ★ **This module is that measurement and nothing else.** It decides no backing, changes no
//! path, and is read only at teardown. If the intersection is empty over a real workload the
//! aperture split is safe and the far more elaborate promote/demote protocol in the design doc
//! is unnecessary. If it is non-empty, the split is unsound as specified and the rule must key
//! on **use** (the map-into-a-GPU-VAS event), exactly as the doc says.
//!
//! # ⚠ What a zero here does and does not prove
//!
//! ⊘ An empty intersection over one workload is **not** a proof of disjointness for all guests
//! — it is evidence that RM's allocator does not in practice hand the same page to both roles.
//! That is the honest claim, and it is the one the constraint actually needs: a *correct* guest
//! not colliding. A hostile guest is a separate question, answered by the fact that a collision
//! corrupts only itself.
//!
//! # ⊘⊘⊘ WHAT THIS COVERS — say it, because the first version did not and was WRONG
//!
//! `[found w719d, by a subagent auditing the tests]` This census shipped with **one** call site,
//! inside `RegPlane::window_page_backing` — the **mirror/premap** path. Every **trapped** guest
//! access resolves through `RegPlane::window_phys` instead (`fb_read`/`fb_write`), and was
//! therefore **invisible to it**.
//!
//! ⇒ A census blind to a whole path prints the same `0` as a census over a genuinely disjoint
//! workload. That is this module's own named failure shape, committed in the same file as the
//! warning against it.
//!
//! ★ **Both paths now record.** The two call sites are `window_page_backing` (premap) and
//! `window_phys` (trap). ⚠ Noting is **idempotent** — it sets a bit — so a page reached through
//! both paths is counted once, and adding a third call site cannot inflate a count.
//!
//! ⊘ Still NOT covered, and named so it is not rediscovered as a surprise: anything that reaches
//! framebuffer memory without going through either translate — a direct `FbStore` call, or an
//! engine's own DMA, which by construction we never see. ⇒ This measures **what the two APERTURES
//! named**, which is exactly the question constraint 15 asks, and nothing wider.
//!
//! ⚠ And a zero is worthless without a known-positive: [`note_synthetic_collision`] exists so a
//! test can prove this instrument CAN report a collision. A census that has never been shown to
//! fire is not evidence of absence — this tree has paid for that lesson roughly twenty times.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Which of the two worlds an access arrived through.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum World {
    /// BAR1 — the CPU's window onto what the guest calls video memory. Constraint 15 says this
    /// must be the reserved device-local object.
    Bar1,
    /// BAR2 or PRAMIN — the instance window and the bring-up aperture. Constraint 15 says these
    /// are served by the fake fb.
    Control,
}

/// How much framebuffer this instrument can speak about, in bytes. Above it, accesses are
/// counted as [`Census::out_of_range`] rather than silently dropped — an address this cannot
/// index is unmeasured, never "did not collide".
pub const COVERED_BYTES: u64 = 16 << 30;
const PAGE: u64 = 4096;
const PAGES: usize = (COVERED_BYTES / PAGE) as usize;
const WORDS: usize = PAGES / 64;

struct Maps {
    bar1: Vec<AtomicU64>,
    ctrl: Vec<AtomicU64>,
    out_of_range: AtomicUsize,
    /// First few colliding GPGAs, for a report that names addresses rather than a count.
    /// A bare number cannot be chased; an address can.
    examples: std::sync::Mutex<Vec<u64>>,
}

static MAPS: OnceLock<Maps> = OnceLock::new();

fn maps() -> &'static Maps {
    MAPS.get_or_init(|| Maps {
        bar1: (0..WORDS).map(|_| AtomicU64::new(0)).collect(),
        ctrl: (0..WORDS).map(|_| AtomicU64::new(0)).collect(),
        out_of_range: AtomicUsize::new(0),
        examples: std::sync::Mutex::new(Vec::new()),
    })
}

/// Record that `gpga` was reached through `world`.
///
/// ⚠ **This is on the framebuffer translate path.** It is one `fetch_or` on a cached word and
/// takes no lock — the collision-example list is touched only on the transition that first makes
/// a page collide, which by construction happens at most once per page.
///
/// ★ The hot-path discipline is not optional here: `an_instrument_is_on_a_hot_path_unless_checked`
/// records a census in this same crate that re-ran a 4.2M-dword sweep fifteen times a boot and
/// broke the boot it was measuring.
pub fn note(world: World, gpga: u64) {
    let m = maps();
    let page = gpga / PAGE;
    if page as usize >= PAGES {
        m.out_of_range.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let (w, b) = ((page / 64) as usize, page % 64);
    let bit = 1u64 << b;
    let (mine, theirs) = match world {
        World::Bar1 => (&m.bar1[w], &m.ctrl[w]),
        World::Control => (&m.ctrl[w], &m.bar1[w]),
    };
    let was = mine.fetch_or(bit, Ordering::Relaxed);
    // ⊘ Only on the FIRST time this world claims the page, and only if the other world already
    // had it: that makes the example list O(collisions), not O(accesses).
    if was & bit == 0 && theirs.load(Ordering::Relaxed) & bit != 0 {
        if let Ok(mut ex) = m.examples.lock() {
            if ex.len() < 16 {
                ex.push(page * PAGE);
            }
        }
    }
}

/// ★ The known-positive. A test calls this to prove the instrument reports a collision when one
/// exists; without it a reported zero cannot be distinguished from an arm that never ran.
pub fn note_synthetic_collision(gpga: u64) {
    note(World::Control, gpga);
    note(World::Bar1, gpga);
}

/// What the two worlds touched, and where they overlapped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Census {
    /// Distinct 4 KiB framebuffer pages reached through BAR1.
    pub bar1_pages: usize,
    /// Distinct pages reached through BAR2 or PRAMIN.
    pub control_pages: usize,
    /// ★★★ **The number the split rests on.** Non-zero ⇒ one GPGA, two memories.
    pub both_pages: usize,
    /// Accesses above [`COVERED_BYTES`], which this instrument cannot index. Unmeasured — read
    /// a non-zero here as "the census is incomplete", never as "no collision".
    pub out_of_range: usize,
    /// Up to sixteen colliding addresses, so a non-zero result names pages to chase.
    pub examples: Vec<u64>,
}

/// Read the census. Cheap and allocation-light; intended for teardown.
pub fn census() -> Census {
    let m = maps();
    let (mut b, mut c, mut both) = (0usize, 0usize, 0usize);
    for i in 0..WORDS {
        let x = m.bar1[i].load(Ordering::Relaxed);
        let y = m.ctrl[i].load(Ordering::Relaxed);
        b += x.count_ones() as usize;
        c += y.count_ones() as usize;
        both += (x & y).count_ones() as usize;
    }
    Census {
        bar1_pages: b,
        control_pages: c,
        both_pages: both,
        out_of_range: m.out_of_range.load(Ordering::Relaxed),
        examples: m.examples.lock().map(|e| e.clone()).unwrap_or_default(),
    }
}

/// The teardown line. One sentence that says what the number MEANS, because a bare
/// `both_pages=0` invites the reader to supply the interpretation.
pub fn report() -> String {
    let c = census();
    // ⊘⊘ **AN EMPTY INTERSECTION IS VACUOUS IF EITHER SIDE IS EMPTY**, and that is the shape
    // this campaign keeps paying for: a census whose arm never ran prints the same `0` as a
    // healthy one. Both worlds must have been SEEN before "disjoint" means anything.
    let verdict = if c.bar1_pages == 0 && c.control_pages == 0 {
        "⊘ NEITHER WORLD WAS TOUCHED — this census did not run; it is not evidence of disjointness"
    } else if c.bar1_pages == 0 {
        "⊘ BAR1 NEVER TOUCHED — the intersection is empty VACUOUSLY; this says nothing about the split"
    } else if c.control_pages == 0 {
        "⊘ NO CONTROL ACCESS SEEN — the intersection is empty VACUOUSLY; this says nothing about the split"
    } else if c.both_pages == 0 {
        "★ DISJOINT over this workload, both worlds non-empty ⇒ the aperture split in constraint 15 is safe here"
    } else {
        "⊘⊘⊘ NOT DISJOINT — one GPGA has two memories; the split cannot key on aperture alone"
    };
    format!(
        "kayfabe: TWO-WORLDS bar1_pages={} control_pages={} both={} out_of_range={} examples={:x?} — {verdict}",
        c.bar1_pages, c.control_pages, c.both_pages, c.out_of_range, c.examples
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ The known-positive this module's own docs demand: prove a collision is REPORTED before
    /// any zero it prints is allowed to mean anything.
    #[test]
    fn a_page_touched_by_both_worlds_is_reported_as_a_collision() {
        // ⊘ High addresses, so this test cannot be perturbed by, nor perturb, a real workload
        // sharing the process-wide maps.
        let a = 15u64 << 30;
        note(World::Bar1, a);
        assert_eq!(
            census().both_pages,
            0,
            "one world alone must not read as a collision"
        );
        note(World::Control, a);
        let c = census();
        assert!(c.both_pages >= 1, "the collision was not reported");
        assert!(
            c.examples.contains(&a),
            "the colliding address was not named: {:x?}",
            c.examples
        );
    }

    /// ⊘ An address this instrument cannot index must be COUNTED as unmeasured, never dropped —
    /// otherwise an out-of-range workload reads as a clean zero.
    #[test]
    fn an_address_above_the_covered_range_is_counted_not_dropped() {
        let before = census().out_of_range;
        note(World::Bar1, COVERED_BYTES + 4096);
        assert_eq!(census().out_of_range, before + 1);
    }
}
