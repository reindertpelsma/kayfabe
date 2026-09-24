//! ★★★★★ **The CPU interrupt tree** — `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_*`, the register block a
//! Turing-or-later driver drives to raise, find and clear an interrupt (`V3_P5_PORT_MAP.md` §2.7).
//!
//! ## Why it is on `RmInitAdapter`'s path
//!
//! `[measured p5d at 34a91296]` once the kernel CE channels were born, `RmInitAdapter` failed
//! `(0x11:0x45:2134)` after `osVerifySystemEnvironment failed, bailing!` — `NV_ERR_IRQ_NOT_FIRING`
//! from `_osVerifyInterrupts` (`ogkm-580: kernel/os/os_sanity.c:119-291`), a LOOPBACK: the driver
//! clears and enables the doorbell vector (`NV_CTRL_CPU_DOORBELL_VECTORID_VALUE_CONSTANT` = 129,
//! `turing/tu102/dev_ctrl.h:35`), writes it to `CPU_INTR_LEAF_TRIGGER`
//! (`intr_swintr_tu102.c:40-51`) and spins ~4 s for its own ISR, which tests the LEAF pending bit
//! (`intrIsVectorPending`, `intr_tu102.c:729-744`) before counting the interrupt. Three things must
//! all hold: the trigger is seen, a message reaches the guest, and the leaf reads back pending.
//!
//! ## The shape (copied from the old tree's `kayfabe-device/src/cpuintr.rs`, made lock-free)
//!
//! The decode, the vector arithmetic (`dev_ctrl_defines.h:46-47,70-78`), write-1-to-clear on LEAF
//! and the SET/CLEAR aliases reading back one enable mask are the old file's (pure; it passed this
//! very self-test on hardware). What changed: the state is ATOMIC words so the vCPU applies a write
//! synchronously and lock-free (§5.5 — the ISR writes a mask then reads pending; a deferred apply
//! would read stale), and TOP is derived from the leaves on every publication, never tracked.
//!
//! ## One layout per family, from the headers
//!
//! Offsets are identical in `turing/tu102`, `ampere/ga100|ga102`, `hopper/gh100` and
//! `blackwell/gb100` `dev_vm.h` (LEAF `0x1000`, LEAF_EN_SET `0x1200`, LEAF_EN_CLEAR `0x1400`, TOP
//! `0x1600`, TOP_EN_SET `0x1608`, TOP_EN_CLEAR `0x1610`, LEAF_TRIGGER `0x1640`, PRIV-relative);
//! `LEAF__SIZE_1` is **8** on Turing/Ampere/Ada and **16** on Hopper/Blackwell; `TOP__SIZE_1` is 1
//! everywhere. The PRIV block sits `0x30000` below the usermode window, as for the invalidate
//! registers ([`crate::mmuinval::InvalidateRegs::from_usermode_base`]).
//!
//! ⊘ **Delivery follows the enables** — a pending bit whose leaf or top enable is clear sends
//! nothing; enabling an already-pending leaf sends it (the GIN's level → message). Unlike the old
//! file, which raised unconditionally: with the state now applied synchronously there is no
//! bookkeeping lag to protect against, and a spurious vector is still harmless to the ISR.

use core::sync::atomic::{AtomicU32, Ordering};

/// PRIV-relative offsets (`dev_vm.h`, every family above).
pub const LEAF_OFF: u64 = 0x1000;
/// `CPU_INTR_LEAF_EN_SET(i)`.
pub const LEAF_EN_SET_OFF: u64 = 0x1200;
/// `CPU_INTR_LEAF_EN_CLEAR(i)`.
pub const LEAF_EN_CLEAR_OFF: u64 = 0x1400;
/// `CPU_INTR_TOP(i)` — read-only.
pub const TOP_OFF: u64 = 0x1600;
/// `CPU_INTR_TOP_EN_SET(i)`.
pub const TOP_EN_SET_OFF: u64 = 0x1608;
/// `CPU_INTR_TOP_EN_CLEAR(i)`.
pub const TOP_EN_CLEAR_OFF: u64 = 0x1610;
/// `CPU_INTR_LEAF_TRIGGER` — write-only, `VECTOR 11:0`.
pub const LEAF_TRIGGER_OFF: u64 = 0x1640;
/// The widest `LEAF__SIZE_1` of any family (Hopper, Blackwell).
pub const MAX_LEAF: usize = 16;
/// `NV_CTRL_CPU_DOORBELL_VECTORID_VALUE_CONSTANT` (`turing/tu102/dev_ctrl.h:35`) — the vector the
/// loopback triggers.
pub const DOORBELL_VECTOR: u32 = 129;

/// `CPU_INTR_LEAF__SIZE_1` for a family (`dev_vm.h`, per family — never a die constant).
#[must_use]
pub const fn leaf_regs_for(family: kf_chip::Family) -> usize {
    match family {
        kf_chip::Family::Turing | kf_chip::Family::Ampere | kf_chip::Family::Ada => 8,
        kf_chip::Family::Hopper | kf_chip::Family::Blackwell => 16,
    }
}

/// One decoded register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reg {
    /// `LEAF(i)` — pending, write-1-to-clear.
    Leaf(usize),
    /// `LEAF_EN_SET(i)`.
    LeafEnSet(usize),
    /// `LEAF_EN_CLEAR(i)`.
    LeafEnClear(usize),
    /// `TOP(0)` — read-only.
    Top,
    /// `TOP_EN_SET(0)`.
    TopEnSet,
    /// `TOP_EN_CLEAR(0)`.
    TopEnClear,
    /// `LEAF_TRIGGER` — write-only.
    Trigger,
}

/// What a write asks of the outside world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Raise {
    /// Nothing.
    None,
    /// Send the message: a pending vector is (now) enabled.
    Message,
    /// A vector outside this family's leaves — latched nowhere, counted.
    OutOfRange,
}

/// ★ The tree: pending + enables, as atomic words.
#[derive(Debug)]
pub struct CpuIntr {
    priv_base: u64,
    n_leaf: usize,
    leaf: [AtomicU32; MAX_LEAF],
    leaf_en: [AtomicU32; MAX_LEAF],
    top_en: AtomicU32,
}

/// `NV_CTRL_INTR_GPU_VECTOR_TO_SUBTREE` — the leaf register index halved (`dev_ctrl_defines.h:77`).
const fn subtree_of_leaf(leaf: usize) -> u32 {
    (leaf / 2) as u32
}

impl CpuIntr {
    /// The tree for a family whose usermode window is at `usermode` (BAR0). `None` when the base
    /// is below the PRIV delta.
    #[must_use]
    pub fn new(family: kf_chip::Family, usermode: u64) -> Option<CpuIntr> {
        Some(CpuIntr {
            priv_base: usermode.checked_sub(crate::mmuinval::USERMODE_ABOVE_PRIV)?,
            n_leaf: leaf_regs_for(family),
            leaf: core::array::from_fn(|_| AtomicU32::new(0)),
            leaf_en: core::array::from_fn(|_| AtomicU32::new(0)),
            top_en: AtomicU32::new(0),
        })
    }

    /// Which register, if any, BAR0 offset `off` is. ⊘ An index beyond this family's
    /// `LEAF__SIZE_1` is not ours (it falls through to the ordinary privileged arm).
    #[must_use]
    pub fn decode(&self, off: u64) -> Option<Reg> {
        let rel = off.checked_sub(self.priv_base)?;
        if rel == LEAF_TRIGGER_OFF {
            return Some(Reg::Trigger);
        }
        let row = |base: u64| -> Option<usize> {
            let r = rel.checked_sub(base)?;
            (r % 4 == 0 && ((r / 4) as usize) < self.n_leaf).then_some((r / 4) as usize)
        };
        match rel {
            TOP_OFF => Some(Reg::Top),
            TOP_EN_SET_OFF => Some(Reg::TopEnSet),
            TOP_EN_CLEAR_OFF => Some(Reg::TopEnClear),
            r if (LEAF_OFF..LEAF_OFF + 0x200).contains(&r) => row(LEAF_OFF).map(Reg::Leaf),
            r if (LEAF_EN_SET_OFF..LEAF_EN_SET_OFF + 0x200).contains(&r) => row(LEAF_EN_SET_OFF).map(Reg::LeafEnSet),
            r if (LEAF_EN_CLEAR_OFF..LEAF_EN_CLEAR_OFF + 0x200).contains(&r) => row(LEAF_EN_CLEAR_OFF).map(Reg::LeafEnClear),
            _ => None,
        }
    }

    /// TOP(0), derived: subtree `s` is pending iff leaf `2s` or `2s+1` has a pending bit.
    #[must_use]
    pub fn top(&self) -> u32 {
        let mut t = 0u32;
        for i in 0..self.n_leaf {
            if self.leaf[i].load(Ordering::Acquire) != 0 {
                t |= 1 << subtree_of_leaf(i);
            }
        }
        t
    }

    /// Is any pending leaf bit enabled at leaf AND top level?
    fn asserted_leaf(&self, i: usize) -> bool {
        let p = self.leaf[i].load(Ordering::Acquire) & self.leaf_en[i].load(Ordering::Acquire);
        p != 0 && self.top_en.load(Ordering::Acquire) & (1 << subtree_of_leaf(i)) != 0
    }

    /// What the guest reads at `r`.
    #[must_use]
    pub fn read(&self, r: Reg) -> u32 {
        match r {
            Reg::Leaf(i) => self.leaf[i].load(Ordering::Acquire),
            Reg::LeafEnSet(i) | Reg::LeafEnClear(i) => self.leaf_en[i].load(Ordering::Acquire),
            Reg::Top => self.top(),
            Reg::TopEnSet | Reg::TopEnClear => self.top_en.load(Ordering::Acquire),
            Reg::Trigger => 0,
        }
    }

    /// ★ **vCPU**: apply one guest write. Lock-free; the caller publishes [`Self::shadow`].
    pub fn write(&self, r: Reg, v: u32) -> Raise {
        match r {
            // Write-1-to-clear (`intrClearLeafVector_TU102`, `intr_tu102.c:648-663`).
            Reg::Leaf(i) => {
                self.leaf[i].fetch_and(!v, Ordering::AcqRel);
                Raise::None
            }
            Reg::LeafEnSet(i) => {
                let was = self.leaf_en[i].fetch_or(v, Ordering::AcqRel);
                // Enabling an already-pending bit sends it.
                if (v & !was) & self.leaf[i].load(Ordering::Acquire) != 0 && self.asserted_leaf(i) {
                    Raise::Message
                } else {
                    Raise::None
                }
            }
            Reg::LeafEnClear(i) => {
                self.leaf_en[i].fetch_and(!v, Ordering::AcqRel);
                Raise::None
            }
            // Read-only (`R--4A`): TOP is a function of the leaves.
            Reg::Top => Raise::None,
            Reg::TopEnSet => {
                let was = self.top_en.fetch_or(v, Ordering::AcqRel);
                let newly = v & !was;
                let any = (0..self.n_leaf).any(|i| newly & (1 << subtree_of_leaf(i)) != 0 && self.asserted_leaf(i));
                if any { Raise::Message } else { Raise::None }
            }
            Reg::TopEnClear => {
                self.top_en.fetch_and(!v, Ordering::AcqRel);
                Raise::None
            }
            Reg::Trigger => self.latch(v & 0xFFF),
        }
    }

    /// ★ Latch `vector` pending — a guest `LEAF_TRIGGER`, or a completion THIS device announces
    /// (a worker, on a host non-stall event). One body for both: the ISR cannot tell them apart.
    pub fn latch(&self, vector: u32) -> Raise {
        let (i, bit) = ((vector / 32) as usize, vector % 32);
        if i >= self.n_leaf {
            return Raise::OutOfRange;
        }
        self.leaf[i].fetch_or(1 << bit, Ordering::AcqRel);
        if self.asserted_leaf(i) { Raise::Message } else { Raise::None }
    }

    /// `(BAR0 offset, value)` for every register whose read-back `r` may have changed — the vCPU
    /// stores these into the read shadow before returning (TOP is always included: derived).
    pub fn shadow(&self, r: Reg, mut put: impl FnMut(u64, u32)) {
        let b = self.priv_base;
        match r {
            Reg::Leaf(i) => put(b + LEAF_OFF + 4 * i as u64, self.read(r)),
            Reg::LeafEnSet(i) | Reg::LeafEnClear(i) => {
                let v = self.read(r);
                put(b + LEAF_EN_SET_OFF + 4 * i as u64, v);
                put(b + LEAF_EN_CLEAR_OFF + 4 * i as u64, v);
            }
            Reg::TopEnSet | Reg::TopEnClear => {
                let v = self.read(r);
                put(b + TOP_EN_SET_OFF, v);
                put(b + TOP_EN_CLEAR_OFF, v);
            }
            Reg::Trigger | Reg::Top => {
                for i in 0..self.n_leaf {
                    put(b + LEAF_OFF + 4 * i as u64, self.leaf[i].load(Ordering::Acquire));
                }
                put(b + LEAF_TRIGGER_OFF, 0);
            }
        }
        put(b + TOP_OFF, self.top());
    }

    /// The whole block's `(offset, value)` — every leaf and TOP (a latch from a worker publishes
    /// this).
    pub fn shadow_all(&self, put: impl FnMut(u64, u32)) {
        self.shadow(Reg::Trigger, put);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UM: u64 = 0x00BB_0000;

    /// `_osVerifyInterrupts` on GA10x, register for register (`os_sanity.c:182-249`).
    #[test]
    fn the_loopback_self_test_sees_its_vector() {
        let t = CpuIntr::new(kf_chip::Family::Ampere, UM).unwrap();
        let pb = UM - 0x30000;
        let (leaf, bit, top_bit) = (129 / 32, 129 % 32, (129 / 32) / 2);
        let w = |off: u64, v: u32| t.write(t.decode(off).unwrap(), v);
        assert_eq!(w(pb + LEAF_OFF + 4 * leaf, 1 << bit), Raise::None, "clear first");
        assert_eq!(w(pb + LEAF_EN_SET_OFF + 4 * leaf, 1 << bit), Raise::None);
        assert_eq!(w(pb + TOP_EN_SET_OFF, 1 << top_bit), Raise::None);
        assert_eq!(w(pb + LEAF_TRIGGER_OFF, 129), Raise::Message, "the trigger sends");
        assert_eq!(t.read(Reg::Leaf(leaf as usize)), 1 << bit, "the ISR finds it pending");
        assert_eq!(t.read(Reg::Top), 1 << top_bit);
        assert_eq!(w(pb + LEAF_OFF + 4 * leaf, 1 << bit), Raise::None, "W1C");
        assert_eq!(t.read(Reg::Leaf(leaf as usize)), 0);
        assert_eq!(t.read(Reg::Top), 0, "TOP is derived");
    }

    #[test]
    fn a_masked_vector_waits_for_its_enable() {
        let t = CpuIntr::new(kf_chip::Family::Ampere, UM).unwrap();
        assert_eq!(t.latch(7), Raise::None, "nothing enabled: pending only");
        assert_eq!(t.write(Reg::TopEnSet, 1), Raise::None, "leaf still masked");
        assert_eq!(t.write(Reg::LeafEnSet(0), 1 << 7), Raise::Message, "enabling a pending leaf sends it");
        assert_eq!(t.latch(16 * 32), Raise::OutOfRange, "Ampere has 8 leaves");
        let h = CpuIntr::new(kf_chip::Family::Hopper, UM).unwrap();
        assert_ne!(h.latch(12 * 32), Raise::OutOfRange, "Hopper has 16");
    }

    #[test]
    fn decode_is_bounded_by_the_family() {
        let t = CpuIntr::new(kf_chip::Family::Ampere, UM).unwrap();
        let pb = UM - 0x30000;
        assert_eq!(t.decode(pb + LEAF_OFF + 7 * 4), Some(Reg::Leaf(7)));
        assert_eq!(t.decode(pb + LEAF_OFF + 8 * 4), None);
        assert_eq!(t.decode(pb + LEAF_TRIGGER_OFF), Some(Reg::Trigger));
        assert_eq!(t.decode(pb + TOP_EN_CLEAR_OFF), Some(Reg::TopEnClear));
        assert_eq!(t.decode(pb + 0x3000), None);
    }
}
