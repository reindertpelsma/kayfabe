//! ★★★★★ **The publication watermark — which VAS tables have changed since we last published.**
//!
//! # Why this exists
//!
//! `P3 rpc-bind` went red the moment the doorbell publication trigger (**leg 8**) was deleted.
//! The deletion was correct — owner, 2026-09-09: *"no vas publish in doorbells … just discard
//! things that I don't need such as a vas publish path that runs inline or inside a thread of a
//! doorbell thats completely not intended for it."*
//!
//! What it exposed is that the rung had been riding on a **poll**. Leg 8 re-published every VAS
//! on **every doorbell** — thousands of times a boot — so it never needed a correct trigger; it
//! simply came back often enough to catch anything. A poll hides the incompleteness of every
//! real trigger beneath it, and underneath leg 8 the only real trigger was a latch inside the
//! `GPU_PROMOTE_CTX` handler, which fires **4 times in a boot**. RM map RPCs, UVM external
//! allocation maps and the parked-promote re-drive are all writers of a VAS table, and **none
//! of them latched**.
//!
//! ⊘ This is the owner's own ruling made mechanical: *"there is no universal publish trigger ⇒
//! enumerate **WRITERS**, not **SIGNALS**."* The writer-side fact is
//! `kayfabe_mmu::AddressTable::generation`, which moves on a bind and on an unbind and **not**
//! on a bind we refused (`taddr_generation_moves_exactly_on_a_content_change`, w318). Ask every
//! table for its generation and compare against what was last published, and the question
//! *"which call bound this row"* stops mattering — which is the entire point.
//!
//! # Why a map and not a counter
//!
//! ⚠ A single summed counter is **cancelled** by two VASes moving in opposite directions: a
//! bind in one and an unbind in another leave the sum flat while both have rows to publish.
//! That is not a hypothetical — an unbind is exactly what a process teardown produces while
//! its neighbour is still allocating. So the watermark is keyed **per VAS**.

use std::collections::BTreeMap;

/// Identifies one VAS across the whole device. ⊘ `Pdb` alone is not unique: it is a **per-GPU**
/// namespace, so the same PDB value on two GPUs is two different address spaces.
pub type VasKey = (u32, u32, u64);

/// The last-published generation of every VAS we have seen.
#[derive(Debug, Default)]
pub struct PublicationWatermark {
    seen: BTreeMap<VasKey, u64>,
}

impl PublicationWatermark {
    /// Nothing published yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the current generation of every live VAS and return **how many differ** from what
    /// was last published. A nonzero answer means there is publication work to do.
    ///
    /// ★ The `observed` list is the authority on which VASes exist. A VAS that has gone away is
    /// **forgotten** rather than left behind, so a PDB the guest later reuses cannot inherit a
    /// stale watermark and be reported clean while carrying somebody else's rows.
    ///
    /// ⊘ **A first sighting counts as a change.** A VAS we have never published is not clean —
    /// treating "no entry" as "up to date" is precisely the favourable-looking absence that
    /// would reproduce the bug this type exists to fix.
    pub fn take_changed(&mut self, observed: &[(VasKey, u64)]) -> usize {
        let mut changed = 0usize;
        let mut next = BTreeMap::new();
        for &(key, generation) in observed {
            if self.seen.get(&key) != Some(&generation) {
                changed += 1;
            }
            next.insert(key, generation);
        }
        self.seen = next;
        changed
    }

    /// How many VASes the watermark is currently tracking. Diagnostics only.
    #[must_use]
    pub fn tracked(&self) -> usize {
        self.seen.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: VasKey = (1, 0, 0x20_1000);
    const B: VasKey = (2, 0, 0x30_1000);

    /// ⊘ THE NEGATIVE CONTROL, first: if "changed" were the default answer, every test below
    /// would pass while the trigger published on every pass — leg 8 rebuilt by accident.
    #[test]
    fn a_second_look_at_an_unchanged_world_reports_nothing() {
        let mut w = PublicationWatermark::new();
        assert_eq!(w.take_changed(&[(A, 7)]), 1, "a first sighting is a change");
        assert_eq!(
            w.take_changed(&[(A, 7)]),
            0,
            "nothing bound in between ⇒ nothing to publish. A trigger that never settles is a \
             poll, which is the thing being deleted"
        );
    }

    /// ★★★ THE ONE THE OLD TRIGGER FAILED. The generation moved because *something* bound a
    /// row; which call did it is not asked and must not be.
    #[test]
    fn any_generation_move_is_a_change_whatever_bound_it() {
        let mut w = PublicationWatermark::new();
        w.take_changed(&[(A, 7)]);
        assert_eq!(w.take_changed(&[(A, 8)]), 1, "a bind must be published");
        assert_eq!(
            w.take_changed(&[(A, 9)]),
            1,
            "and the next one too — the promote-only latch saw four of these in a whole boot"
        );
    }

    /// ★★★★★ **WHY A MAP AND NOT A COUNTER.** A bind in one VAS and an unbind in another leave
    /// a SUM of generations flat. Both have rows to publish.
    #[test]
    fn opposite_moves_in_two_vases_do_not_cancel() {
        let mut w = PublicationWatermark::new();
        w.take_changed(&[(A, 10), (B, 10)]);
        let changed = w.take_changed(&[(A, 11), (B, 9)]);
        assert_eq!(
            changed, 2,
            "the sum is unchanged at 20 and BOTH VASes changed. A single counter reports 0 here \
             and publishes neither"
        );
    }

    /// ⊘ A VAS that goes away is forgotten, so a PDB the guest reuses starts unpublished rather
    /// than inheriting the previous tenant's watermark.
    #[test]
    fn a_retired_vas_is_forgotten_not_remembered_as_clean() {
        let mut w = PublicationWatermark::new();
        w.take_changed(&[(A, 5), (B, 5)]);
        assert_eq!(w.take_changed(&[(B, 5)]), 0, "A is gone; B did not change");
        assert_eq!(w.tracked(), 1, "A is no longer tracked");
        assert_eq!(
            w.take_changed(&[(A, 5), (B, 5)]),
            1,
            "A came back at the SAME generation a fresh table would report. Remembering it as \
             clean would leave a reused PDB carrying the previous tenant's rows unpublished — \
             and cross-process leakage is the one outcome this project refuses by name"
        );
    }

    /// ⊘ The same PDB on two GPUs is two address spaces. Keying on the PDB alone would report
    /// one of them clean forever.
    #[test]
    fn the_same_pdb_on_two_gpus_is_two_vases() {
        let mut w = PublicationWatermark::new();
        let gpu0 = (1u32, 0u32, 0x20_1000u64);
        let gpu1 = (1u32, 1u32, 0x20_1000u64);
        assert_eq!(w.take_changed(&[(gpu0, 3), (gpu1, 3)]), 2);
        assert_eq!(
            w.take_changed(&[(gpu0, 4), (gpu1, 3)]),
            1,
            "only GPU 0's copy moved; a PDB-only key cannot express that"
        );
    }

    /// ⊘ An empty world is not a change. A device between processes must not spin the
    /// publication worker.
    #[test]
    fn no_vases_is_not_a_change() {
        let mut w = PublicationWatermark::new();
        w.take_changed(&[(A, 1)]);
        assert_eq!(w.take_changed(&[]), 0, "everything retired; nothing to publish");
        assert_eq!(w.tracked(), 0);
    }

    /// ★ A generation that goes BACKWARDS is still a change. Nothing in the type may assume
    /// monotonicity: a table can be replaced wholesale, and "not equal" is the honest test
    /// where "greater than" would silently skip the replacement.
    #[test]
    fn a_generation_that_moves_backwards_is_still_a_change() {
        let mut w = PublicationWatermark::new();
        w.take_changed(&[(A, 100)]);
        assert_eq!(
            w.take_changed(&[(A, 2)]),
            1,
            "a fresh table in the same slot reads LOW, and it is the case most in need of \
             publishing — a `>` comparison would call it clean"
        );
    }
}
