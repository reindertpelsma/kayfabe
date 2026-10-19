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
    match v.map(str::trim).and_then(|t| t.parse::<u64>().ok()) {
        Some(n) => u32::try_from(n.min(u64::from(MAX_DELAY_US))).unwrap_or(MAX_DELAY_US),
        None => {
            // An all-digit value too large for u64 is still "a lot": clamp, do not turn it off.
            match v.map(str::trim) {
                Some(t) if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) => MAX_DELAY_US,
                _ => 0,
            }
        }
    }
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
    if (INTR_PAGE..INTR_PAGE_END).contains(&off) {
        d.intr_us
    } else if off == doorbell_off {
        d.db_us
    } else {
        0
    }
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
        let mut r = Regs::default();
        for i in 0..n_leaf.min(MAX_LEAF) {
            r.leaf[i] = rd(Self::off(LEAF_OFF, i));
            r.en_set[i] = rd(Self::off(LEAF_EN_SET_OFF, i));
            r.en_clear[i] = rd(Self::off(LEAF_EN_CLEAR_OFF, i));
        }
        r.top = rd(Self::off(TOP_OFF, 0));
        r.top_en_set = rd(Self::off(TOP_EN_SET_OFF, 0));
        r.top_en_clear = rd(Self::off(TOP_EN_CLEAR_OFF, 0));
        r.trigger = rd(Self::off(LEAF_TRIGGER_OFF, 0));
        r
    }

    fn off(base: u64, i: usize) -> usize {
        (base - REL) as usize + 4 * i
    }

    /// Call `f(page_offset, old, new)` for every owned word whose value differs.
    pub fn changes(&self, new: &Regs, n_leaf: usize, mut f: impl FnMut(usize, u32, u32)) {
        let mut diff = |o: usize, a: u32, b: u32| {
            if a != b {
                f(o, a, b);
            }
        };
        for i in 0..n_leaf.min(MAX_LEAF) {
            diff(Self::off(LEAF_OFF, i), self.leaf[i], new.leaf[i]);
            diff(Self::off(LEAF_EN_SET_OFF, i), self.en_set[i], new.en_set[i]);
            diff(Self::off(LEAF_EN_CLEAR_OFF, i), self.en_clear[i], new.en_clear[i]);
        }
        diff(Self::off(TOP_OFF, 0), self.top, new.top);
        diff(Self::off(TOP_EN_SET_OFF, 0), self.top_en_set, new.top_en_set);
        diff(Self::off(TOP_EN_CLEAR_OFF, 0), self.top_en_clear, new.top_en_clear);
        diff(Self::off(LEAF_TRIGGER_OFF, 0), self.trigger, new.trigger);
    }
}

impl Regs {
    /// The word at page offset `off` (4-aligned, one of the owned registers of an `n_leaf` family).
    #[must_use]
    pub fn get(&self, off: usize, n_leaf: usize) -> Option<u32> {
        let n = n_leaf.min(MAX_LEAF);
        let idx = |base: u64| -> Option<usize> {
            let r = off.checked_sub((base - REL) as usize)?;
            (r % 4 == 0 && r / 4 < n).then_some(r / 4)
        };
        if let Some(i) = idx(LEAF_OFF) {
            return Some(self.leaf[i]);
        }
        if let Some(i) = idx(LEAF_EN_SET_OFF) {
            return Some(self.en_set[i]);
        }
        if let Some(i) = idx(LEAF_EN_CLEAR_OFF) {
            return Some(self.en_clear[i]);
        }
        match off as u64 + REL {
            TOP_OFF => Some(self.top),
            TOP_EN_SET_OFF => Some(self.top_en_set),
            TOP_EN_CLEAR_OFF => Some(self.top_en_clear),
            LEAF_TRIGGER_OFF => Some(self.trigger),
            _ => None,
        }
    }

    /// Set the word at page offset `off`; `false` when `off` is not an owned register.
    pub fn set(&mut self, off: usize, n_leaf: usize, v: u32) -> bool {
        if self.get(off, n_leaf).is_none() {
            return false;
        }
        let n = n_leaf.min(MAX_LEAF);
        let idx = |base: u64| -> Option<usize> {
            let r = off.checked_sub((base - REL) as usize)?;
            (r % 4 == 0 && r / 4 < n).then_some(r / 4)
        };
        if let Some(i) = idx(LEAF_OFF) {
            self.leaf[i] = v;
        } else if let Some(i) = idx(LEAF_EN_SET_OFF) {
            self.en_set[i] = v;
        } else if let Some(i) = idx(LEAF_EN_CLEAR_OFF) {
            self.en_clear[i] = v;
        } else {
            match off as u64 + REL {
                TOP_OFF => self.top = v,
                TOP_EN_SET_OFF => self.top_en_set = v,
                TOP_EN_CLEAR_OFF => self.top_en_clear = v,
                _ => self.trigger = v,
            }
        }
        true
    }
}

/// Variant B ("model-driven spinner"): the guest writes it made since the page was last published, as
/// `(page offset, value)`, in the order a trap would have applied them: enables, then leaf acks, then
/// the trigger. TOP is read-only and never a write.
#[must_use]
pub fn observed_writes(n_leaf: usize, last: &Regs, cur: &Regs) -> Vec<(usize, u32)> {
    let mut v: Vec<(u8, usize, u32)> = Vec::new();
    last.changes(cur, n_leaf, |off, _old, new| {
        let rel = off as u64 + REL;
        let class = if (LEAF_EN_SET_OFF..LEAF_EN_SET_OFF + 0x200).contains(&rel)
            || (LEAF_EN_CLEAR_OFF..LEAF_EN_CLEAR_OFF + 0x200).contains(&rel)
            || rel == TOP_EN_SET_OFF
            || rel == TOP_EN_CLEAR_OFF
        {
            0
        } else if (LEAF_OFF..LEAF_OFF + 0x200).contains(&rel) {
            1
        } else if rel == LEAF_TRIGGER_OFF {
            2
        } else {
            return; // TOP: read-only
        };
        v.push((class, off, new));
    });
    v.sort_by_key(|&(c, o, _)| (c, o));
    v.into_iter().map(|(_, o, n)| (o, n)).collect()
}

/// `TOP(0)` derived from the leaves: subtree `s` pends iff leaf `2s` or `2s+1` is nonzero.
#[must_use]
pub fn derive_top(leaf: &[u32], n_leaf: usize) -> u32 {
    let mut t = 0u32;
    for (i, w) in leaf.iter().enumerate().take(n_leaf) {
        if *w != 0 {
            t |= 1 << (i / 2);
        }
    }
    t
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
        SpinState {
            n_leaf: leaf_regs_for(family),
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
        let n = self.n_leaf;
        let mut new = *cur;
        for i in 0..n {
            // (a) learn: the guest only enables vectors it handles. A value in EN_SET that is not
            // the mask means a store happened; OR it in. EN_CLEAR is never learned from.
            self.p[i] |= cur.en_set[i];
            new.en_set[i] = self.p[i];
            new.en_clear[i] = self.p[i];
            // (b) forward everything enabled-ever, restoring it right after any guest store (W1C).
            new.leaf[i] = self.p[i];
        }
        self.top_p |= cur.top_en_set;
        new.top_en_set = self.top_p;
        new.top_en_clear = self.top_p;
        new.top = derive_top(&new.leaf, n);
        // (c) a nonzero trigger word: one raise, word cleared.
        let raise = cur.trigger != 0;
        new.trigger = 0;
        (new, raise)
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
    #[test]
    fn delay_cap_holds_for_every_large_value() {
        for v in ["1000", "1001", "2000", "65536", "4294967295", "4294967296", "18446744073709551616"] {
            assert!(parse_delay_us(Some(v)) <= MAX_DELAY_US, "{v}");
            assert_eq!(parse_delay_us(Some(v)), MAX_DELAY_US, "{v}");
        }
        assert_eq!(parse_delay_us(Some("1")), 1);
    }

    #[test]
    fn delay_garbage_is_off_never_max() {
        for v in ["x", "10us", "0x10", "+", "--3", "1e3", " ", "\t"] {
            assert_eq!(parse_delay_us(Some(v)), 0, "{v:?}");
        }
    }

    #[test]
    fn the_interrupt_range_is_exactly_one_page() {
        let d = Delays { intr_us: 7, db_us: 0 };
        let hit = (0u64..0xBC_0000).step_by(4).filter(|&o| delay_for(&d, 0xBB_0090, o) != 0).count();
        assert_eq!(hit, 0x1000 / 4);
        assert_eq!(delay_for(&d, 0xBB_0090, 0xB8_1000), 7);
        assert_eq!(delay_for(&d, 0xBB_0090, 0xB8_2000), 0);
    }

    #[test]
    fn the_doorbell_delay_is_exactly_one_register() {
        let d = Delays { intr_us: 0, db_us: 9 };
        let hit: Vec<u64> = (0u64..0xBC_0000).step_by(4).filter(|&o| delay_for(&d, 0xBB_0090, o) != 0).collect();
        assert_eq!(hit, vec![0xBB_0090]);
    }

    #[test]
    fn learning_never_forgets_a_vector() {
        let mut s = fresh(Family::Ampere);
        let mut page = Regs::default();
        page.en_set[0] = 0b01;
        page = s.step(&page).0;
        page.en_set[0] = 0b10;
        page = s.step(&page).0;
        assert_eq!(s.mask(0), 0b11);
        page.en_set[0] = 0b01; // re-enabling a subset
        page = s.step(&page).0;
        assert_eq!(s.mask(0), 0b11);
        assert_eq!(page.leaf[0], 0b11);
        page.top_en_set = 0b01;
        page = s.step(&page).0;
        page.top_en_set = 0b10;
        page = s.step(&page).0;
        assert_eq!(page.top_en_set, 0b11);
    }

    #[test]
    fn an_en_clear_store_alone_never_changes_the_mask() {
        let mut s = fresh(Family::Ampere);
        let mut page = Regs::default();
        page.en_set[3] = 0b1111;
        page = s.step(&page).0;
        for v in [0b0001, 0b0110, 0xFFFF_FFFF, 0b1111, 0] {
            page.en_clear[3] = v;
            let (n, raise) = s.step(&page);
            assert!(!raise);
            assert_eq!(s.mask(3), 0b1111, "after EN_CLEAR {v:#x}");
            assert_eq!(n.leaf[3], 0b1111);
            assert_eq!(n.en_clear[3], 0b1111);
            page = n;
        }
    }

    #[test]
    fn the_restore_never_exceeds_the_learned_mask() {
        let mut s = fresh(Family::Hopper);
        let mut page = Regs::default();
        page.en_set[9] = 0x0000_00F0;
        page = s.step(&page).0;
        for v in [0xFFFF_FFFF, 0x0F, 0x100, 0xF0, 0] {
            page.leaf[9] = v;
            let (n, _) = s.step(&page);
            assert_eq!(n.leaf[9], 0xF0, "guest stored {v:#x}");
            assert_eq!(n.leaf[9] & !s.mask(9), 0);
            page = n;
        }
    }

    #[test]
    fn en_set_reads_back_the_mask_not_the_last_store() {
        let mut s = fresh(Family::Ampere);
        let mut page = Regs::default();
        page.en_set[1] = 0b0100;
        page = s.step(&page).0;
        page.en_set[1] = 0b0001;
        let (n, _) = s.step(&page);
        assert_eq!(n.en_set[1], 0b0101);
        assert_eq!(n.en_clear[1], 0b0101);
    }

    #[test]
    fn every_register_has_its_own_word_at_its_documented_offset() {
        let at = |f: &dyn Fn(&mut Regs)| {
            let mut r = Regs::default();
            f(&mut r);
            let mut v = Vec::new();
            r.changes(&Regs::default(), 16, |o, _, _| v.push(o));
            v
        };
        assert_eq!(at(&|r| r.leaf[0] = 1), vec![0x000]);
        assert_eq!(at(&|r| r.leaf[15] = 1), vec![0x03C]);
        assert_eq!(at(&|r| r.en_set[0] = 1), vec![0x200]);
        assert_eq!(at(&|r| r.en_set[15] = 1), vec![0x23C]);
        assert_eq!(at(&|r| r.en_clear[0] = 1), vec![0x400]);
        assert_eq!(at(&|r| r.en_clear[15] = 1), vec![0x43C]);
        assert_eq!(at(&|r| r.top = 1), vec![0x600]);
        assert_eq!(at(&|r| r.top_en_set = 1), vec![0x608]);
        assert_eq!(at(&|r| r.top_en_clear = 1), vec![0x610]);
        assert_eq!(at(&|r| r.trigger = 1), vec![0x640]);
    }
    #[test]
    fn latch_forward_range_is_the_family_range_for_every_vector() {
        use crate::cpuintr::{CpuIntr, Raise};
        for (f, n) in [(Family::Ampere, 8u32), (Family::Hopper, 16)] {
            let t = CpuIntr::new(f, 0xBB_0000).unwrap();
            let snap = t.snapshot();
            for v in 0..(20 * 32) {
                let want = if v / 32 < n { Raise::Message } else { Raise::OutOfRange };
                assert_eq!(t.latch_forward(v), want, "{f:?} vector {v}");
            }
            assert_eq!(t.snapshot(), snap, "{f:?}: the model is untouched");
        }
    }

    #[test]
    fn get_and_set_address_every_owned_word_and_nothing_else() {
        let mut r = Regs::default();
        for (off, n) in [(0x000usize, 8usize), (0x01C, 8), (0x200, 8), (0x21C, 8), (0x400, 8), (0x41C, 8), (0x600, 8), (0x608, 8), (0x610, 8), (0x640, 8), (0x03C, 16), (0x43C, 16)] {
            assert!(r.set(off, n, off as u32 + 1), "{off:#x}");
            assert_eq!(r.get(off, n), Some(off as u32 + 1), "{off:#x}");
        }
        assert_eq!(r.leaf[0], 1);
        assert_eq!(r.leaf[7], 0x1D);
        assert_eq!(r.en_set[0], 0x201);
        assert_eq!(r.en_clear[7], 0x41D);
        assert_eq!(r.top, 0x601);
        assert_eq!(r.trigger, 0x641);
        for off in [0x020usize, 0x03C, 0x220, 0x420, 0x604, 0x60C, 0x644, 0x800, 0xFFC, 0x002] {
            let n = if off == 0x03C { 8 } else { 8 };
            assert_eq!(r.get(off, n), None, "{off:#x} is not owned on an 8-leaf family");
            assert!(!r.set(off, n, 7), "{off:#x}");
        }
        assert_eq!(r.get(0x03C, 16), Some(0x3D), "owned on a 16-leaf family");
    }

    #[test]
    fn observed_writes_are_enables_then_acks_then_the_trigger_and_never_top() {
        let last = Regs::default();
        let mut cur = Regs::default();
        cur.trigger = 129;
        cur.leaf[4] = 2;
        cur.top = 0xFF; // a guest store to a read-only word is not a write
        cur.top_en_set = 4;
        cur.en_set[4] = 2;
        cur.en_clear[1] = 1;
        let w = observed_writes(8, &last, &cur);
        assert_eq!(
            w,
            vec![(0x210, 2), (0x404, 1), (0x608, 4), (0x010, 2), (0x640, 129)],
            "en_set, en_clear, top_en_set; leaf; trigger"
        );
        assert!(observed_writes(8, &cur, &cur).is_empty());
    }

    #[test]
    fn observed_writes_see_only_the_family_leaves() {
        let last = Regs::default();
        let mut cur = Regs::default();
        cur.leaf[9] = 1;
        assert!(observed_writes(8, &last, &cur).is_empty());
        assert_eq!(observed_writes(16, &last, &cur), vec![(0x024, 1)]);
    }
    #[test]
    fn observed_writes_put_enables_before_acks() {
        let mut cur = Regs::default();
        cur.leaf[0] = 1;
        cur.en_set[7] = 1;
        let w = observed_writes(8, &Regs::default(), &cur);
        assert_eq!(w, vec![(0x21C, 1), (0x000, 1)]);
    }

    #[test]
    fn observed_writes_put_acks_before_the_trigger() {
        let mut cur = Regs::default();
        cur.trigger = 3;
        cur.leaf[7] = 1;
        let w = observed_writes(8, &Regs::default(), &cur);
        assert_eq!(w, vec![(0x01C, 1), (0x640, 3)]);
    }

    #[test]
    fn a_store_to_the_read_only_top_word_is_not_a_write() {
        let mut cur = Regs::default();
        cur.top = 7;
        assert!(observed_writes(8, &Regs::default(), &cur).is_empty());
        assert_eq!(observed_writes(16, &Regs::default(), &cur).len(), 0);
    }

    #[test]
    fn get_reads_the_three_top_words_and_the_trigger() {
        let mut r = Regs::default();
        r.top = 1;
        r.top_en_set = 2;
        r.top_en_clear = 3;
        r.trigger = 4;
        assert_eq!(
            [r.get(0x600, 8), r.get(0x608, 8), r.get(0x610, 8), r.get(0x640, 8)],
            [Some(1), Some(2), Some(3), Some(4)]
        );
    }

    #[test]
    fn set_refuses_a_word_that_is_not_owned_and_changes_nothing() {
        let mut r = Regs::default();
        assert!(!r.set(0x800, 8, 5));
        assert!(!r.set(0x024, 8, 5), "leaf 9 on an 8-leaf family");
        assert_eq!(r, Regs::default());
    }

    #[test]
    fn get_is_bounded_by_the_family_leaf_count_for_every_array() {
        let mut r = Regs::default();
        r.leaf[8] = 1;
        r.en_set[8] = 2;
        r.en_clear[8] = 3;
        assert_eq!([r.get(0x020, 8), r.get(0x220, 8), r.get(0x420, 8)], [None, None, None]);
        assert_eq!([r.get(0x020, 16), r.get(0x220, 16), r.get(0x420, 16)], [Some(1), Some(2), Some(3)]);
    }
}
