//! ★ **MEASUREMENT ONLY — feature `diag-trap-exp`, never in a production build.**
//!
//! `STATUS: RESEARCH, 2026-10-19.` Two experiments that ask "how much of the Steel Nomad gap is
//! the cost of trapped BAR0 writes?" (`traces/trap_experiments_20261019/README.md`):
//!
//! 1. **Delay injection** — a trapped interrupt-tree / doorbell write busy-waits N extra
//!    microseconds ([`parse_delay_us`], [`Delays`], [`delay_for`]).
//! 2. **The interrupt-page spinner** — the interrupt page (BAR0 `0xB81000..0xB82000`) is plain RAM
//!    (no exit on a guest write); one host thread polls it and answers with [`SpinState::step`].
//!    ⊘ This deliberately breaks the no-polling rule; it is an experiment, not a design.
//!
//! Everything here is pure: the thread, the clock and the memory are in `kf-qemu`.

use crate::cpuintr::{
    LEAF_EN_CLEAR_OFF, LEAF_EN_SET_OFF, LEAF_OFF, LEAF_TRIGGER_OFF, MAX_LEAF, TOP_EN_CLEAR_OFF,
    TOP_EN_SET_OFF, TOP_OFF, leaf_regs_for,
};

/// The longest injected delay: a typo cannot stall a vCPU for seconds.
pub const MAX_DELAY_US: u32 = 1000;
/// First byte of the interrupt page in BAR0 (`PRIV` base `0xB80000` + `0x1000`).
pub const INTR_PAGE: u64 = 0x00B8_1000;
/// One past the last byte of the interrupt page.
pub const INTR_PAGE_END: u64 = 0x00B8_2000;

/// Parse a `KF3_DIAG_TRAP_DELAY_*_US` value. Default (unset, empty, garbage, negative, zero) is 0 =
/// off; a larger value is clamped to [`MAX_DELAY_US`].
#[must_use]
pub fn parse_delay_us(v: Option<&str>) -> u32 {
    let _ = v;
    u32::MAX
}

/// The two delays, in microseconds (0 = off).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Delays {
    /// Added to every trapped write in the interrupt page.
    pub intr_us: u32,
    /// Added to every trapped doorbell write.
    pub db_us: u32,
}

/// The extra microseconds a trapped BAR0 write at `off` costs. `doorbell_off` is the doorbell's
/// BAR0 offset. Only the interrupt page and the doorbell are ever delayed.
#[must_use]
pub fn delay_for(d: &Delays, doorbell_off: u64, off: u64) -> u32 {
    let _ = (d, doorbell_off, off);
    u32::MAX
}

/// The interrupt page's registers, one value per word the spinner owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Regs {
    /// `CPU_INTR_LEAF(i)`.
    pub leaf: [u32; MAX_LEAF],
    /// `CPU_INTR_LEAF_EN_SET(i)`.
    pub en_set: [u32; MAX_LEAF],
    /// `CPU_INTR_LEAF_EN_CLEAR(i)`.
    pub en_clear: [u32; MAX_LEAF],
    /// `CPU_INTR_TOP(0)`.
    pub top: u32,
    /// `CPU_INTR_TOP_EN_SET(0)`.
    pub top_en_set: u32,
    /// `CPU_INTR_TOP_EN_CLEAR(0)`.
    pub top_en_clear: u32,
    /// `CPU_INTR_LEAF_TRIGGER`.
    pub trigger: u32,
}

const REL: u64 = INTR_PAGE - 0xB8_0000; // PRIV-relative start of the page (0x1000)

impl Regs {
    /// Read the page: `rd(page_offset)` returns the 32-bit word at that byte offset in the page.
    pub fn load(n_leaf: usize, mut rd: impl FnMut(usize) -> u32) -> Regs {
        let _ = (n_leaf, &mut rd);
        Regs::default()
    }

    /// Call `f(page_offset, old, new)` for every owned word whose value differs.
    pub fn changes(&self, new: &Regs, n_leaf: usize, mut f: impl FnMut(usize, u32, u32)) {
        let _ = (new, n_leaf, &mut f);
    }
}

/// `TOP(0)` derived from the leaves: subtree `s` pends iff leaf `2s` or `2s+1` is nonzero.
#[must_use]
pub fn derive_top(leaf: &[u32], n_leaf: usize) -> u32 {
    let _ = (leaf, n_leaf);
    u32::MAX
}

/// The spinner's whole state: the vector mask it has learned. Bounded: 17 words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpinState {
    n_leaf: usize,
    p: [u32; MAX_LEAF],
    top_p: u32,
}

impl SpinState {
    /// A fresh state for a family (8 leaves on Turing/Ampere/Ada, 16 on Hopper/Blackwell).
    #[must_use]
    pub fn new(family: kf_chip::Family) -> SpinState {
        let _ = family;
        SpinState {
            n_leaf: 0,
            p: [0; MAX_LEAF],
            top_p: 0,
        }
    }

    /// Leaves this family has.
    #[must_use]
    pub fn n_leaf(&self) -> usize {
        self.n_leaf
    }

    /// The learned vector mask of leaf `i`.
    #[must_use]
    pub fn mask(&self, i: usize) -> u32 {
        self.p[i]
    }

    /// One poll: `cur` is what the page holds; returns the words the page should hold and whether to
    /// raise the irqfd once.
    pub fn step(&mut self, cur: &Regs) -> (Regs, bool) {
        let _ = cur;
        (Regs::default(), true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_chip::Family;

    #[test]
    fn delay_parse_default_off() {
        for v in [None, Some(""), Some("0"), Some("abc"), Some("-5"), Some("1.5")] {
            assert_eq!(parse_delay_us(v), 0, "{v:?}");
        }
        assert_eq!(parse_delay_us(Some(" 10 ")), 10);
        assert_eq!(parse_delay_us(Some("20")), 20);
    }

    #[test]
    fn delay_parse_is_bounded() {
        assert_eq!(parse_delay_us(Some("1000")), MAX_DELAY_US);
        assert_eq!(parse_delay_us(Some("1001")), MAX_DELAY_US);
        assert_eq!(parse_delay_us(Some("99999999999999999999")), MAX_DELAY_US);
        assert_eq!(parse_delay_us(Some("999")), 999);
    }

    #[test]
    fn delay_applies_only_to_the_listed_ranges() {
        let d = Delays {
            intr_us: 20,
            db_us: 10,
        };
        let db = 0xBB_0090;
        assert_eq!(delay_for(&d, db, INTR_PAGE), 20);
        assert_eq!(delay_for(&d, db, INTR_PAGE + 0x1640 - 0x1000), 20);
        assert_eq!(delay_for(&d, db, INTR_PAGE_END - 4), 20);
        assert_eq!(delay_for(&d, db, INTR_PAGE - 4), 0);
        assert_eq!(delay_for(&d, db, INTR_PAGE_END), 0);
        assert_eq!(delay_for(&d, db, db), 10);
        assert_eq!(delay_for(&d, db, db + 4), 0);
        assert_eq!(delay_for(&d, db, db - 4), 0);
        assert_eq!(delay_for(&d, db, 0x110c00), 0, "a GSP register is never delayed");
        assert_eq!(delay_for(&d, db, 0xB8_30B0), 0, "MMU_INVALIDATE is never delayed");
    }

    #[test]
    fn delay_arms_are_independent_and_default_zero() {
        let db = 0xBB_0090;
        let off = Delays::default();
        for o in [INTR_PAGE, db, 0, 0x1234] {
            assert_eq!(delay_for(&off, db, o), 0);
        }
        let only_intr = Delays {
            intr_us: 10,
            db_us: 0,
        };
        assert_eq!(delay_for(&only_intr, db, db), 0);
        assert_eq!(delay_for(&only_intr, db, INTR_PAGE), 10);
        let only_db = Delays {
            intr_us: 0,
            db_us: 10,
        };
        assert_eq!(delay_for(&only_db, db, INTR_PAGE), 0);
        assert_eq!(delay_for(&only_db, db, db), 10);
    }

    #[test]
    fn page_offsets_match_the_register_map() {
        // Every owned word lands where cpuintr's decode says the register is.
        let mut seen = Vec::new();
        let mut r = Regs::default();
        r.leaf[1] = 1;
        r.en_set[2] = 1;
        r.en_clear[3] = 1;
        r.top = 1;
        r.top_en_set = 1;
        r.top_en_clear = 1;
        r.trigger = 1;
        r.changes(&Regs::default(), 8, |o, old, new| seen.push((o, old, new)));
        let rel = |x: u64| (x - 0x1000) as usize;
        for (o, name) in [
            (rel(LEAF_OFF + 4), "leaf1"),
            (rel(LEAF_EN_SET_OFF + 8), "en_set2"),
            (rel(LEAF_EN_CLEAR_OFF + 12), "en_clear3"),
            (rel(TOP_OFF), "top"),
            (rel(TOP_EN_SET_OFF), "top_en_set"),
            (rel(TOP_EN_CLEAR_OFF), "top_en_clear"),
            (rel(LEAF_TRIGGER_OFF), "trigger"),
        ] {
            assert!(seen.iter().any(|&(so, _, _)| so == o), "{name} at {o:#x}");
        }
        assert_eq!(seen.len(), 7);
        // load is the inverse: reading each word back by its offset rebuilds the struct.
        let back = Regs::load(8, |o| {
            seen.iter()
                .find(|&&(so, _, _)| so == o)
                .map_or(0, |&(_, old, _)| old)
        });
        assert_eq!(back, r);
        // REL is the page start.
        assert_eq!(REL, 0x1000);
    }

    #[test]
    fn changes_reports_only_what_differs_and_only_owned_leaves() {
        let a = Regs::default();
        let mut b = a;
        assert_eq!(count_changes(&a, &b, 8), 0);
        b.leaf[7] = 4;
        assert_eq!(count_changes(&a, &b, 8), 1);
        b.leaf[8] = 4; // not an Ampere leaf
        assert_eq!(count_changes(&a, &b, 8), 1);
        assert_eq!(count_changes(&a, &b, 16), 2);
    }

    fn count_changes(a: &Regs, b: &Regs, n: usize) -> usize {
        let mut c = 0;
        a.changes(b, n, |_, _, _| c += 1);
        c
    }

    fn fresh(fam: Family) -> SpinState {
        SpinState::new(fam)
    }

    #[test]
    fn leaf_counts_per_family() {
        for (f, n) in [
            (Family::Turing, 8),
            (Family::Ampere, 8),
            (Family::Ada, 8),
            (Family::Hopper, 16),
            (Family::Blackwell, 16),
        ] {
            assert_eq!(fresh(f).n_leaf(), n, "{f:?}");
        }
    }

    #[test]
    fn learns_the_mask_from_en_set_and_ors_it_over_time() {
        let mut s = fresh(Family::Ampere);
        let mut cur = Regs::default();
        cur.en_set[4] = 0b10;
        let (new, _) = s.step(&cur);
        assert_eq!(s.mask(4), 0b10);
        assert_eq!(new.leaf[4], 0b10, "forward everything enabled-ever");
        assert_eq!(new.en_set[4], 0b10, "read-back of the enable mask");
        // a later enable of another vector ORs in; the earlier one stays
        let mut cur2 = new;
        cur2.en_set[4] = 0b100;
        let (new2, _) = s.step(&cur2);
        assert_eq!(s.mask(4), 0b110);
        assert_eq!(new2.leaf[4], 0b110);
        assert_eq!(new2.en_set[4], 0b110);
        assert_eq!(new2.en_clear[4], 0b110, "EN_CLEAR reads back the enable mask too");
        assert_eq!(s.mask(0), 0, "other leaves untouched");
    }

    #[test]
    fn en_clear_and_top_en_clear_are_ignored() {
        let mut s = fresh(Family::Ampere);
        let mut cur = Regs::default();
        cur.en_set[0] = 0b1;
        cur.top_en_set = 0b1;
        let (mut page, _) = s.step(&cur);
        page.en_clear[0] = 0b1; // the guest disables the vector
        page.top_en_clear = 0b1;
        let (page2, raise) = s.step(&page);
        assert!(!raise);
        assert_eq!(s.mask(0), 0b1, "a disable does not unlearn");
        assert_eq!(page2.en_clear[0], 0b1, "restored to the mask");
        assert_eq!(page2.top_en_clear, 0b1);
        assert_eq!(page2.leaf[0], 0b1, "still forwarded");
    }

    #[test]
    fn restores_pending_right_after_the_guest_ack_store() {
        let mut s = fresh(Family::Ampere);
        let mut cur = Regs::default();
        cur.en_set[1] = 0b1011;
        let (mut page, _) = s.step(&cur);
        assert_eq!(page.leaf[1], 0b1011);
        // the ISR acks bit 1: W1C is a plain store of the value into the RAM word
        page.leaf[1] = 0b10;
        let (page2, raise) = s.step(&page);
        assert!(!raise, "an ack never raises");
        assert_eq!(page2.leaf[1], 0b1011, "pending is restored");
        // the ISR acks everything
        let mut page3 = page2;
        page3.leaf[1] = 0xFFFF_FFFF;
        let (page4, _) = s.step(&page3);
        assert_eq!(page4.leaf[1], 0b1011, "never more than the learned mask");
        // a guest store of 0 is not an ack of anything, and still restores
        let mut page5 = page4;
        page5.leaf[1] = 0;
        assert_eq!(s.step(&page5).0.leaf[1], 0b1011);
    }

    #[test]
    fn nothing_is_pending_before_anything_is_enabled() {
        let mut s = fresh(Family::Ampere);
        let (page, raise) = s.step(&Regs::default());
        assert_eq!(page, Regs::default());
        assert!(!raise);
    }

    #[test]
    fn top_is_derived_from_the_leaves() {
        // subtree s = leaves 2s and 2s+1
        let cases: [(usize, u32); 8] = [(0, 1), (1, 1), (2, 2), (3, 2), (4, 4), (5, 4), (6, 8), (7, 8)];
        for (leaf, top) in cases {
            let mut l = [0u32; MAX_LEAF];
            l[leaf] = 0x8000_0000;
            assert_eq!(derive_top(&l, 8), top, "leaf {leaf}");
        }
        let mut l = [0u32; MAX_LEAF];
        assert_eq!(derive_top(&l, 8), 0);
        l[0] = 1;
        l[7] = 1;
        assert_eq!(derive_top(&l, 8), 0b1001, "subtrees 0 and 3");
        // Hopper: leaf 15 is subtree 7; Ampere does not look at it
        let mut h = [0u32; MAX_LEAF];
        h[15] = 1;
        assert_eq!(derive_top(&h, 16), 1 << 7);
        assert_eq!(derive_top(&h, 8), 0);
    }

    #[test]
    fn step_writes_top_from_the_forced_leaves() {
        let mut s = fresh(Family::Hopper);
        let mut cur = Regs::default();
        cur.en_set[15] = 1;
        cur.en_set[2] = 2;
        let (page, _) = s.step(&cur);
        assert_eq!(page.top, (1 << 7) | (1 << 1));
        let mut s8 = fresh(Family::Ampere);
        let mut cur8 = Regs::default();
        cur8.en_set[7] = 1;
        assert_eq!(s8.step(&cur8).0.top, 1 << 3);
    }

    #[test]
    fn a_family_never_owns_leaves_beyond_its_count() {
        let mut s = fresh(Family::Ampere);
        let mut cur = Regs::default();
        cur.en_set[8] = 0xFF; // Hopper-only leaf
        let (page, _) = s.step(&cur);
        assert_eq!(s.mask(8), 0);
        assert_eq!(page.leaf[8], 0);
        let mut h = fresh(Family::Hopper);
        let (page, _) = h.step(&cur);
        assert_eq!(h.mask(8), 0xFF);
        assert_eq!(page.leaf[8], 0xFF);
    }

    #[test]
    fn a_trigger_raises_once_and_the_word_is_cleared() {
        let mut s = fresh(Family::Ampere);
        let mut cur = Regs::default();
        cur.trigger = 129;
        let (page, raise) = s.step(&cur);
        assert!(raise);
        assert_eq!(page.trigger, 0, "the word is cleared");
        let (page2, raise2) = s.step(&page);
        assert!(!raise2, "one trigger, one raise");
        assert_eq!(page2.trigger, 0);
        // a second trigger raises again
        let mut cur3 = page2;
        cur3.trigger = 7;
        assert!(s.step(&cur3).1);
    }

    #[test]
    fn it_never_raises_without_a_trigger() {
        let mut s = fresh(Family::Hopper);
        let mut x: u32 = 0x1234_5678;
        let mut page = Regs::default();
        for _ in 0..2000 {
            // pseudo-random guest stores into every word EXCEPT the trigger
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let i = (x >> 8) as usize % MAX_LEAF;
            match (x >> 4) % 6 {
                0 => page.leaf[i] = x,
                1 => page.en_set[i] = x,
                2 => page.en_clear[i] = x,
                3 => page.top_en_set = x,
                4 => page.top_en_clear = x,
                _ => page.top = x,
            }
            let (new, raise) = s.step(&page);
            assert!(!raise, "a raise without a trigger");
            page = new;
        }
    }

    #[test]
    fn a_quiescent_page_is_a_fixpoint_and_stores_nothing() {
        let mut s = fresh(Family::Ampere);
        let mut cur = Regs::default();
        cur.en_set[4] = 0b10;
        cur.top_en_set = 0b100;
        let (p1, _) = s.step(&cur);
        let (p2, r2) = s.step(&p1);
        assert_eq!(p1, p2);
        assert!(!r2);
        assert_eq!(count_changes(&p1, &p2, 8), 0, "no store, no cache-line bounce");
        assert_eq!(p1.top_en_set, 0b100);
        assert_eq!(p1.top_en_clear, 0b100);
    }

    #[test]
    fn the_driver_loopback_self_test_sees_its_vector() {
        // _osVerifyInterrupts: clear 129 (W1C), enable it, enable its subtree, trigger it.
        let (leaf, bit, sub) = (129 / 32, 129 % 32, (129 / 32) / 2);
        let mut s = fresh(Family::Ampere);
        let mut page = Regs::default();
        page.leaf[leaf] = 1 << bit; // the clear
        let (p, raise) = s.step(&page);
        assert!(!raise);
        page = p;
        page.en_set[leaf] = 1 << bit;
        page.top_en_set = 1 << sub;
        page.trigger = 129;
        let (p, raise) = s.step(&page);
        assert!(raise, "the irqfd is written once");
        assert_eq!(p.trigger, 0);
        assert_eq!(p.leaf[leaf] & (1 << bit), 1 << bit, "the ISR finds it pending");
        assert_eq!(p.top, 1 << sub);
        // the ISR acks it; the page restores (forward everything)
        let mut acked = p;
        acked.leaf[leaf] = 1 << bit;
        assert_eq!(s.step(&acked).0.leaf[leaf], 1 << bit);
    }

    #[test]
    fn latch_forward_raises_without_enables_and_the_normal_latch_is_unchanged() {
        use crate::cpuintr::{CpuIntr, Raise};
        let t = CpuIntr::new(Family::Ampere, 0xBB_0000).unwrap();
        assert_eq!(t.latch(7), Raise::None, "masked: the model holds it");
        assert_eq!(t.latch_forward(7), Raise::Message, "forward: no enables needed");
        assert_eq!(t.latch_forward(16 * 32), Raise::OutOfRange);
        assert_eq!(t.latch_forward(8 * 32 - 1), Raise::Message, "last Ampere vector");
        assert_eq!(t.latch_forward(8 * 32), Raise::OutOfRange);
        let h = CpuIntr::new(Family::Hopper, 0xBB_0000).unwrap();
        assert_eq!(h.latch_forward(12 * 32), Raise::Message, "Hopper has 16 leaves");
    }

    #[test]
    fn latch_forward_does_not_touch_the_model() {
        use crate::cpuintr::CpuIntr;
        let t = CpuIntr::new(Family::Ampere, 0xBB_0000).unwrap();
        let before = t.snapshot();
        let _ = t.latch_forward(70);
        assert_eq!(t.snapshot(), before, "the page, not the model, holds pending");
    }

    #[test]
    fn leaf_regs_match_the_family_table() {
        for f in [Family::Turing, Family::Ampere, Family::Ada, Family::Hopper, Family::Blackwell] {
            assert_eq!(fresh(f).n_leaf(), leaf_regs_for(f));
        }
    }
}
