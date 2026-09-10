//! ★★★★★ **PROMOTE / DEMOTE — which GPGA pages we hold a host copy of, and when we may stop.**
//!
//! `docs/design/gpga_is_one_reserved_object.md`. This is the bookkeeping half; the copies
//! themselves are copy-engine work on the scratchpad channel.
//!
//! # Why a host copy at all
//!
//! `[measured 2026-09-11, RTX 3060]` the processor reads video memory at **~48 MiB/s**, flat
//! from 64-byte to 16-MiB copies, against **3674 MiB/s** for ordinary memory through the
//! identical loop. At that rate a full page-table re-read is ~500 ms and a boot's worth of
//! refreshes is **~10 minutes of bus traffic**. Promotion converts a per-refresh cost into a
//! **per-page-once** cost. It is the difference between booting and not.
//!
//! # The rule this type exists to keep
//!
//! ⊘ **A page is promoted BEFORE it is read, never after.** We learn a page is a page table
//! from its **parent**, which is already promoted, so there is no first sight that needs an
//! aperture read. Two consequences:
//!
//! 1. The processor never reads video memory on our path — an absolute, not a common case.
//! 2. We only ever read **our own copy**, so a guest writing underneath cannot tear a read.
//!    It can only make the copy stale, which the next refresh corrects.

use std::collections::{BTreeMap, BTreeSet};

/// A GPGA page. ⊘ Page-aligned by construction — an unaligned key would let one page appear
/// under two names and be promoted twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Page(u64);

impl Page {
    /// The page containing `gpga`.
    #[must_use]
    pub const fn containing(gpga: u64) -> Self {
        Self(gpga & !0xfff)
    }

    /// Its base address.
    #[must_use]
    pub const fn base(self) -> u64 {
        self.0
    }
}

/// What a page needs before it can be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Already has a host copy — read it directly.
    Ready,
    /// Still only in video memory. ⚠ It must be promoted before **any** entry is read from it.
    Promote,
}

/// The promotion ledger.
#[derive(Debug, Default)]
pub struct Promotions {
    promoted: BTreeSet<u64>,
    /// child page → the parent pages whose entries point at it. The reference count behind
    /// demotion condition (1).
    parents: BTreeMap<u64, BTreeSet<u64>>,
    promotes: u64,
    demotes: u64,
}

impl Promotions {
    /// Nothing promoted.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What must happen before an entry in this page may be read.
    #[must_use]
    pub fn need(&self, p: Page) -> Need {
        if self.promoted.contains(&p.base()) {
            Need::Ready
        } else {
            Need::Promote
        }
    }

    /// Record that a host copy now exists and is authoritative. Idempotent: promoting twice is
    /// a caller re-deriving the same fact, not an error, and must not double-count.
    pub fn promote(&mut self, p: Page) {
        if self.promoted.insert(p.base()) {
            self.promotes += 1;
        }
    }

    /// Release the host copy. ⊘ The caller has already copied back to video memory.
    pub fn demote(&mut self, p: Page) {
        if self.promoted.remove(&p.base()) {
            self.demotes += 1;
        }
    }

    /// `parent`'s entries point at `child`.
    pub fn link(&mut self, child: Page, parent: Page) {
        self.parents
            .entry(child.base())
            .or_default()
            .insert(parent.base());
    }

    /// `parent` no longer points at `child` — seen in a dirty page during a refresh.
    pub fn unlink(&mut self, child: Page, parent: Page) {
        if let Some(set) = self.parents.get_mut(&child.base()) {
            set.remove(&parent.base());
            if set.is_empty() {
                self.parents.remove(&child.base());
            }
        }
    }

    /// How many parents still point at `child`. Demotion condition (1) is this reaching zero.
    #[must_use]
    pub fn referrers(&self, child: Page) -> usize {
        self.parents.get(&child.base()).map_or(0, BTreeSet::len)
    }

    /// ★★★ **Which promoted pages may be demoted**, given the set reached by the walk that just
    /// finished.
    ///
    /// Both conditions are **residuals over a COMPLETED walk**, which is why demotion runs at
    /// the end of a refresh and never during it:
    ///
    /// 1. no parent still references it, and
    /// 2. the walk did not reach it.
    ///
    /// ⊘ **Conservative on purpose.** Getting this wrong costs CHURN, not correctness: the copy
    /// back to video memory happens before the host buffer is released, so the bytes survive
    /// either way and a wrongly demoted page is simply promoted again next refresh. ⇒ When
    /// unsure, do not demote. That is why `reachable` alone can veto, without consulting the
    /// reference table.
    #[must_use]
    pub fn ripe_for_demotion(&self, reachable: &BTreeSet<u64>) -> Vec<Page> {
        self.promoted
            .iter()
            .filter(|b| !reachable.contains(*b))
            .filter(|b| self.parents.get(*b).is_none_or(BTreeSet::is_empty))
            .map(|b| Page(*b))
            .collect()
    }

    /// ★★★ The pages in `wanted` that still need promoting — **one round of the fixpoint**.
    ///
    /// ⚠ Promotion iterates: a newly promoted directory reveals table addresses we did not
    /// know. It terminates because each round promotes **at least one** new page and the page
    /// set is finite, and because the walk itself refuses cycles and dangling pointers. An
    /// unbounded loop here would hang the invalidate the guest is blocked on — the one place we
    /// cannot afford one.
    ///
    /// ⊘ Returned as a **batch**. A copy-engine copy completes asynchronously against a
    /// semaphore, so per-page promotion is one submit-and-wait each; at the 1872 pages measured
    /// in `w422` that is 1872 round trips and the round trip, not the bytes, becomes the cost —
    /// the same shape as the 48 MiB/s finding one level up.
    #[must_use]
    pub fn round(&self, wanted: &[Page]) -> Vec<Page> {
        let mut out: Vec<Page> = wanted
            .iter()
            .copied()
            .filter(|p| self.need(*p) == Need::Promote)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// `(promotes, demotes)` — cumulative. ⊘ A demote count climbing with the promote count is
    /// thrash: a page being demoted and re-promoted across refreshes.
    #[must_use]
    pub fn census(&self) -> (u64, u64) {
        (self.promotes, self.demotes)
    }

    /// How many host copies are live.
    #[must_use]
    pub fn live(&self) -> usize {
        self.promoted.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pg(n: u64) -> Page {
        Page::containing(n * 0x1000)
    }

    /// ⊘ THE NEGATIVE CONTROL. Without it, a `need` hardcoded to `Ready` passes everything
    /// below while letting the walk read video memory — the one thing the design forbids.
    #[test]
    fn an_unpromoted_page_must_be_promoted_before_it_is_read() {
        let p = Promotions::new();
        assert_eq!(p.need(pg(1)), Need::Promote);
        assert_eq!(p.live(), 0);
    }

    #[test]
    fn promotion_is_idempotent_and_counted_once() {
        let mut p = Promotions::new();
        p.promote(pg(1));
        p.promote(pg(1));
        assert_eq!(p.need(pg(1)), Need::Ready);
        assert_eq!(p.census(), (1, 0), "a re-derived fact is not a second promotion");
        assert_eq!(p.live(), 1);
    }

    /// ⊘ Any address in a page names the same page. An unaligned key would let one page be
    /// promoted twice and demoted once.
    #[test]
    fn any_address_in_a_page_names_that_page() {
        let mut p = Promotions::new();
        p.promote(Page::containing(0x4000));
        assert_eq!(p.need(Page::containing(0x4fff)), Need::Ready);
        assert_eq!(p.need(Page::containing(0x5000)), Need::Promote);
    }

    /// ★★★ DEMOTION CONDITION (1): a page a parent still points at is not ripe, even when the
    /// walk did not reach it.
    #[test]
    fn a_referenced_page_is_never_ripe() {
        let mut p = Promotions::new();
        p.promote(pg(7));
        p.link(pg(7), pg(1));
        assert!(
            p.ripe_for_demotion(&BTreeSet::new()).is_empty(),
            "a live parent reference vetoes demotion regardless of reachability"
        );
        p.unlink(pg(7), pg(1));
        assert_eq!(p.referrers(pg(7)), 0);
        assert_eq!(
            p.ripe_for_demotion(&BTreeSet::new()),
            vec![pg(7)],
            "the LAST reference dropping is what makes it dead"
        );
    }

    /// ⊘ Two parents, and only the second unlink matters. A page shared between address spaces
    /// stays alive while either still points at it.
    #[test]
    fn the_last_referrer_decides_not_the_first() {
        let mut p = Promotions::new();
        p.promote(pg(7));
        p.link(pg(7), pg(1));
        p.link(pg(7), pg(2));
        p.unlink(pg(7), pg(1));
        assert_eq!(p.referrers(pg(7)), 1);
        assert!(p.ripe_for_demotion(&BTreeSet::new()).is_empty());
        p.unlink(pg(7), pg(2));
        assert_eq!(p.ripe_for_demotion(&BTreeSet::new()), vec![pg(7)]);
    }

    /// ★★★ DEMOTION CONDITION (2): reachability alone vetoes, WITHOUT consulting the reference
    /// table. That is the conservative bias — when the two disagree, keep the copy.
    #[test]
    fn a_page_the_walk_reached_is_never_ripe_even_with_no_recorded_parents() {
        let mut p = Promotions::new();
        p.promote(pg(9));
        assert_eq!(p.referrers(pg(9)), 0, "no parent recorded at all");
        let reachable: BTreeSet<u64> = [pg(9).base()].into_iter().collect();
        assert!(
            p.ripe_for_demotion(&reachable).is_empty(),
            "the walk reached it, so it is live whatever our bookkeeping says. Being wrong here \
             costs churn; being wrong the other way drops a copy the walk is about to need"
        );
    }

    /// ★★★★★ **WRONG DEMOTION COSTS CHURN, NOT CORRECTNESS** — the property that lets both
    /// conditions be approximations.
    #[test]
    fn a_wrongly_demoted_page_simply_returns_to_needing_promotion() {
        let mut p = Promotions::new();
        p.promote(pg(3));
        p.demote(pg(3));
        assert_eq!(
            p.need(pg(3)),
            Need::Promote,
            "it is back where it started, not lost — the copy-back precedes the release, so the \
             bytes survive and the next refresh promotes it again"
        );
        p.promote(pg(3));
        assert_eq!(p.need(pg(3)), Need::Ready);
        assert_eq!(p.census(), (2, 1), "and the thrash is VISIBLE as a count");
    }

    /// ★★★ THE FIXPOINT TERMINATES. Each round returns strictly fewer outstanding pages once
    /// its batch is promoted, so a walk that keeps discovering children still converges.
    #[test]
    fn the_promotion_fixpoint_converges() {
        let mut p = Promotions::new();
        // A chain: each promoted page reveals the next, as a real directory walk does.
        let mut wanted = vec![pg(1)];
        let mut rounds = 0;
        for depth in 2..=12u64 {
            let batch = p.round(&wanted);
            assert!(!batch.is_empty(), "a round that promotes nothing cannot terminate");
            for b in &batch {
                p.promote(*b);
            }
            rounds += 1;
            wanted = vec![pg(depth)];
        }
        assert_eq!(rounds, 11);
        assert!(
            p.round(&[pg(1), pg(5), pg(11)]).is_empty(),
            "everything already promoted ⇒ the round is empty and the loop ends"
        );
    }

    /// ⊘ A round deduplicates. A walk naturally asks for the same directory many times — once
    /// per entry that points at it — and promoting it twice in one batch would submit two
    /// copies of the same page to the engine.
    #[test]
    fn a_round_asks_for_each_page_once() {
        let p = Promotions::new();
        let batch = p.round(&[pg(4), pg(4), pg(2), pg(4), pg(2)]);
        assert_eq!(batch, vec![pg(2), pg(4)], "deduplicated and ordered");
    }

    /// ⊘ And the batch excludes what is already ready, so a steady state costs no engine work
    /// at all — which is the entire point of promotion over re-reading.
    #[test]
    fn a_steady_state_asks_for_nothing() {
        let mut p = Promotions::new();
        for n in 1..=5 {
            p.promote(pg(n));
        }
        assert!(p.round(&[pg(1), pg(2), pg(3), pg(4), pg(5)]).is_empty());
    }
}
