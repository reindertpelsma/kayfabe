// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ **The device's DMA regime: may a guest device address be read as a guest-physical one?**
//! (`docs/design/V3_VIOMMU.md` §4, 2026-10-04.)
//!
//! `docs/OWNER_RULINGS.md` §Q names a **fourth address kind** beside a guest VRAM offset, a
//! guest-physical address and a BAR offset: *"A guest DMA address (an IOVA under a guest vIOMMU)
//! is a fourth kind. It must be translated to a GPA at one validated boundary before any of the
//! rules above apply."* Every system-memory address the guest driver hands the device (sysmem PTE
//! leaves, USERD, error notifiers, the GSP boot arguments and message queues, copy-engine physical
//! operands, display memory) is the driver's DMA address for this device. With no guest IOMMU in
//! front of the device, or one that does not translate it, that value IS the guest-physical
//! address. Under a translating guest IOMMU it is an IOVA, and reading it as a GPA lands on an
//! unrelated guest page.
//!
//! The translator is not built (`V3_VIOMMU.md` §5). Until it is, the VMM adapter publishes the
//! regime into a [`DmaRegimeCell`], and every lookup of a device address in guest RAM goes through
//! [`DmaRegimeCell::admitted`] first: only [`DmaRegime::Direct`] and [`DmaRegime::Identity`]
//! admit; every other regime refuses at use, and the refusal is counted (§4.3).
//!
//! The regime is the VMM's **model** of the guest's IOMMU, not the guest's own statement.
//! [`DmaRegime::Untracked`] exists because a model can fail to follow the guest: a guest IOMMU
//! emulated without DMA remapping passes addresses through while the guest still programs
//! translations, so whether this device's addresses are IOVAs cannot be read from the VMM, and
//! they are refused (§4.2).
//!
//! Fail-closed throughout: a cell starts [`DmaRegime::Unset`] (refuses until the first publish), a
//! device address space that shows neither guest RAM nor a translator is [`DmaRegime::Blocked`]
//! (refuses), and an unknown wire value reads as [`DmaRegime::Translating`] (refuses).

use core::fmt;
use core::sync::atomic::{AtomicU8, AtomicU64, Ordering};

/// What a device address the guest programs means right now (`docs/design/V3_VIOMMU.md` §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DmaRegime {
    /// Nothing published yet. Refuses: the adapter publishes once the machine is assembled, and
    /// nothing before that may read a device address as a GPA.
    Unset,
    /// No guest IOMMU in front of the device: a device address IS a guest-physical address, for
    /// the device's lifetime.
    Direct,
    /// Behind a guest IOMMU that does not translate this device (pass-through, a bypass domain,
    /// translation switched off): a device address is a guest-physical address while this holds.
    Identity,
    /// The guest translates this device's DMA, so device addresses are IOVAs. Refused until the
    /// translator is built.
    Translating,
    /// Behind a guest IOMMU whose translation the VMM does not model, so whether device addresses
    /// are IOVAs is unknowable. Refused.
    Untracked,
    /// The device's address space shows neither guest RAM nor a translator: the intermediate state
    /// while a guest IOMMU switches this device between modes, or a state the adapter cannot
    /// classify. Refused.
    Blocked,
}

impl DmaRegime {
    /// Every regime (the test universe; nothing branches on the order).
    pub const ALL: [DmaRegime; 6] = [
        Self::Unset,
        Self::Direct,
        Self::Identity,
        Self::Translating,
        Self::Untracked,
        Self::Blocked,
    ];

    /// The VMM adapter's wire value (`qemu/hw/misc/kf3/kf3.h`, `KF3_DMA_*`): 0 direct, 1 identity,
    /// 2 translating, 3 untracked, 4 blocked. ⊘ Any other value reads as [`Self::Translating`]: an
    /// adapter newer than this archive is refused, never guessed admitting.
    #[must_use]
    pub const fn from_wire(wire: u32) -> Self {
        match wire {
            0 => Self::Direct,
            1 => Self::Identity,
            2 => Self::Translating,
            3 => Self::Untracked,
            4 => Self::Blocked,
            _ => Self::Translating,
        }
    }

    /// Whether a device address may be read as a guest-physical address in this regime.
    #[must_use]
    pub const fn admits(self) -> bool {
        matches!(self, Self::Direct | Self::Identity)
    }

    /// The cell's private encoding (not the wire's: `Unset` has no wire value).
    const fn code(self) -> u8 {
        match self {
            Self::Unset => 0,
            Self::Direct => 1,
            Self::Identity => 2,
            Self::Translating => 3,
            Self::Untracked => 4,
            Self::Blocked => 5,
        }
    }

    /// The inverse of [`Self::code`]. Only [`Self::code`] ever writes the cell, so the fallback
    /// arm is unreachable; it refuses all the same.
    const fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Unset,
            1 => Self::Direct,
            2 => Self::Identity,
            3 => Self::Translating,
            4 => Self::Untracked,
            5 => Self::Blocked,
            _ => Self::Translating,
        }
    }
}

/// One device's published [`DmaRegime`] and the device-address lookups it refused.
///
/// ⊘ [`Self::set`] is ONE atomic store and nothing else: the adapter calls it from a memory-change
/// callback that runs on whichever thread commits a memory change in the VM, a vCPU included, so
/// it may not block, allocate, log or take a lock (`docs/OWNER_RULINGS.md` §A.4).
pub struct DmaRegimeCell {
    regime: AtomicU8,
    refused: AtomicU64,
}

impl Default for DmaRegimeCell {
    /// [`DmaRegime::Unset`]: refuses until the adapter's first publish.
    fn default() -> Self {
        Self {
            regime: AtomicU8::new(DmaRegime::Unset.code()),
            refused: AtomicU64::new(0),
        }
    }
}

impl fmt::Debug for DmaRegimeCell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DmaRegimeCell")
            .field("regime", &self.get())
            .field("refused", &self.refused())
            .finish()
    }
}

impl DmaRegimeCell {
    /// Publish `regime` (any thread; one atomic store).
    pub fn set(&self, regime: DmaRegime) {
        self.regime.store(regime.code(), Ordering::Release);
    }

    /// The regime last published ([`DmaRegime::Unset`] before the first).
    #[must_use]
    pub fn get(&self) -> DmaRegime {
        DmaRegime::from_code(self.regime.load(Ordering::Acquire))
    }

    /// Whether a device-address lookup may proceed now. A refusal is counted.
    #[must_use]
    pub fn admit(&self) -> bool {
        let ok = self.get().admits();
        if !ok {
            self.refused.fetch_add(1, Ordering::Relaxed);
        }
        ok
    }

    /// ★ The interim gate (`docs/design/V3_VIOMMU.md` §4.3): run `lookup`, which resolves a
    /// device address as a guest-physical one, only when the regime admits; otherwise `None`,
    /// counted, and `lookup` never runs.
    pub fn admitted<T>(&self, lookup: impl FnOnce() -> Option<T>) -> Option<T> {
        if self.admit() { lookup() } else { None }
    }

    /// Device-address lookups refused so far.
    #[must_use]
    pub fn refused(&self) -> u64 {
        self.refused.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The adapter's five wire values, and the fail-closed reading of every other one.
    #[test]
    fn the_wire_values_map_and_an_unknown_one_refuses() {
        let known = [
            (0, DmaRegime::Direct),
            (1, DmaRegime::Identity),
            (2, DmaRegime::Translating),
            (3, DmaRegime::Untracked),
            (4, DmaRegime::Blocked),
        ];
        for (wire, regime) in known {
            assert_eq!(DmaRegime::from_wire(wire), regime, "wire {wire}");
        }
        for wire in [5, 6, 0x100, u32::MAX] {
            let r = DmaRegime::from_wire(wire);
            assert_eq!(r, DmaRegime::Translating, "wire {wire}");
            assert!(!r.admits(), "an unknown wire value {wire} must refuse");
        }
    }

    /// Only a direct or identity regime reads a device address as a GPA.
    #[test]
    fn only_direct_and_identity_admit() {
        let admitting: Vec<DmaRegime> = DmaRegime::ALL.into_iter().filter(|r| r.admits()).collect();
        assert_eq!(admitting, [DmaRegime::Direct, DmaRegime::Identity]);
        for r in DmaRegime::ALL {
            let cell = DmaRegimeCell::default();
            cell.set(r);
            assert_eq!(cell.get(), r);
            assert_eq!(cell.admit(), r.admits(), "{r:?}");
            assert_eq!(cell.refused(), u64::from(!r.admits()), "{r:?}");
        }
    }

    /// A fresh cell is `Unset` and refuses: nothing reads a device address before the first
    /// publish.
    #[test]
    fn a_fresh_cell_refuses_until_the_first_publish() {
        let cell = DmaRegimeCell::default();
        assert_eq!(cell.get(), DmaRegime::Unset);
        assert!(!cell.admit());
        assert_eq!(cell.admitted(|| Some(7)), None);
        assert_eq!(cell.refused(), 2);
        cell.set(DmaRegime::Direct);
        assert_eq!(cell.admitted(|| Some(7)), Some(7));
        assert_eq!(cell.refused(), 2, "an admitted lookup is not counted");
    }

    /// The gate runs the lookup only when the regime admits, and follows a published change in
    /// both directions (a guest IOMMU can switch a device into and out of translation).
    #[test]
    fn the_gate_follows_the_regime_in_both_directions() {
        let cell = DmaRegimeCell::default();
        let mut ran = 0u32;
        let mut probe = |cell: &DmaRegimeCell| {
            cell.admitted(|| {
                ran += 1;
                Some(())
            })
        };
        for (r, admitted) in [
            (DmaRegime::Identity, true),
            (DmaRegime::Blocked, false),
            (DmaRegime::Translating, false),
            (DmaRegime::Blocked, false),
            (DmaRegime::Identity, true),
            (DmaRegime::Untracked, false),
            (DmaRegime::Direct, true),
        ] {
            cell.set(r);
            assert_eq!(probe(&cell).is_some(), admitted, "{r:?}");
        }
        assert_eq!(ran, 3, "a refused lookup must never run");
        assert_eq!(cell.refused(), 4);
        assert_eq!(
            format!("{cell:?}"),
            "DmaRegimeCell { regime: Direct, refused: 4 }"
        );
    }
}
