// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ **The memory plane's device-address seam** (`docs/design/V3_VIOMMU.md` §3, 2026-10-04).
//!
//! A sysmem leaf the walker reports, like every other system-memory address the guest driver
//! programs, is a [`DevAddr`]: the guest's DMA address for the device, a guest-physical address only
//! while no guest IOMMU translates it (`docs/OWNER_RULINGS.md` §Q, the fourth address kind). The
//! plane turns one into guest RAM through ONE resolver, [`DmaResolve`], which answers in **pieces**
//! of the guest-RAM object: under a translating guest IOMMU one IOVA-contiguous run is in general
//! several guest-physical ranges (`V3_VIOMMU.md` §2, row 1).
//!
//! Today the production resolver is the identity behind the regime gate (`kf-qemu`'s `DmaSpace`), so
//! a run is always one piece and behaviour is unchanged. What the seam fixes now, while it is cheap
//! (§6):
//! - the plane names the address kind ([`DevAddr`]) instead of a bare `u64` called `gpa`;
//! - a refusal carries its reason ([`DmaRefusal`]) instead of `None`;
//! - a multi-piece answer the plane cannot place yet is refused by name
//!   ([`DmaRefusal::Fragmented`]), never mapped as if it were one run.
//!
//! ⊘ Test and harness resolvers wrap a raw layout closure in [`IdentityFn`] — a NAMED wrapper, never
//! a blanket `impl` for closures — so a raw-`u64` closure cannot quietly become a production
//! resolver.

use core::fmt;
use kf_arch::dma::{DevAddr, DmaRegime};
use kf_host::MapPerm;

/// One piece of the guest-RAM object a device range resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RamPiece {
    /// Offset in the guest-RAM object (the memfd offset a sysmem row is placed at).
    pub off: u64,
    /// Bytes.
    pub len: u64,
    /// The permission the piece may be placed with: the one asked for, narrowed by the guest
    /// IOMMU's when one translates (the identity leaves it unchanged).
    pub perm: MapPerm,
}

/// Why a device range did not resolve. Refused by name, never clamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaRefusal {
    /// The device's DMA regime does not admit device addresses as guest-physical ones
    /// (`docs/design/V3_VIOMMU.md` §4.3: the interim, until the translator is built).
    Regime {
        /// The device address.
        at: DevAddr,
        /// Bytes.
        len: u64,
        /// The regime that refused.
        regime: DmaRegime,
    },
    /// The range is not guest RAM the VMM backs, in one block (§Q: a device address may reach
    /// *"only guest ram or its bar or an error"*).
    NotGuestRam {
        /// The device address.
        at: DevAddr,
        /// Bytes.
        len: u64,
    },
    /// The range resolved to `pieces` pieces where the consumer can use exactly one.
    Fragmented {
        /// The device address.
        at: DevAddr,
        /// Bytes.
        len: u64,
        /// How many pieces it resolved to.
        pieces: usize,
    },
}

impl fmt::Display for DmaRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Regime { at, len, regime } => write!(
                f,
                "device range {at:#x}+{len:#x}: the DMA regime {regime:?} does not admit it as guest-physical"
            ),
            Self::NotGuestRam { at, len } => {
                write!(
                    f,
                    "device range {at:#x}+{len:#x}: not guest RAM in one block"
                )
            }
            Self::Fragmented { at, len, pieces } => write!(
                f,
                "fragmented device range {at:#x}+{len:#x} ({pieces} pieces): multi-piece placement not built"
            ),
        }
    }
}

/// ★ The ONE way the memory plane turns a device range into guest RAM.
pub trait DmaResolve {
    /// The pieces of the guest-RAM object that `[at, at+len)` names, in device-address order,
    /// each with `want` narrowed to what the guest allows there. Their lengths sum to `len`.
    ///
    /// # Errors
    /// [`DmaRefusal`], by name.
    fn resolve(&self, at: DevAddr, len: u64, want: MapPerm) -> Result<Vec<RamPiece>, DmaRefusal>;

    /// Exactly one piece — for a consumer that can use no other (one placement, a copy-engine
    /// physical operand, a USERD slot, an error notifier). An implementation that can answer
    /// without allocating should override this.
    ///
    /// # Errors
    /// [`DmaRefusal::Fragmented`] for more than one piece; otherwise as [`Self::resolve`].
    fn resolve_one(&self, at: DevAddr, len: u64, want: MapPerm) -> Result<RamPiece, DmaRefusal> {
        let pieces = self.resolve(at, len, want)?;
        match pieces.as_slice() {
            [one] => Ok(*one),
            [] => Err(DmaRefusal::NotGuestRam { at, len }),
            _ => Err(DmaRefusal::Fragmented {
                at,
                len,
                pieces: pieces.len(),
            }),
        }
    }
}

/// A test or harness resolver: the identity regime over a raw layout closure `f(gpa, len)` that
/// answers the guest-RAM object offset of `[gpa, gpa+len)`, or `None` where it backs nothing.
///
/// ⊘ Named on purpose (`docs/design/V3_VIOMMU.md` §3.3): production resolves through `kf-qemu`'s
/// `DmaSpace`, behind the device's DMA regime; a closure becomes a resolver only by being wrapped
/// here, in plain sight.
pub struct IdentityFn<F: Fn(u64, u64) -> Option<u64>>(pub F);

impl<F: Fn(u64, u64) -> Option<u64>> DmaResolve for IdentityFn<F> {
    fn resolve(&self, at: DevAddr, len: u64, want: MapPerm) -> Result<Vec<RamPiece>, DmaRefusal> {
        self.resolve_one(at, len, want).map(|p| vec![p])
    }

    fn resolve_one(&self, at: DevAddr, len: u64, want: MapPerm) -> Result<RamPiece, DmaRefusal> {
        (self.0)(at.translator_raw(), len)
            .map(|off| RamPiece {
                off,
                len,
                perm: want,
            })
            .ok_or(DmaRefusal::NotGuestRam { at, len })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RW: MapPerm = MapPerm::READ_WRITE;

    /// A resolver that answers a fixed piece list (or refusal) whatever it is asked.
    struct Fixed(Result<Vec<RamPiece>, DmaRefusal>);
    impl DmaResolve for Fixed {
        fn resolve(&self, _: DevAddr, _: u64, _: MapPerm) -> Result<Vec<RamPiece>, DmaRefusal> {
            self.0.clone()
        }
    }

    fn piece(off: u64, len: u64) -> RamPiece {
        RamPiece { off, len, perm: RW }
    }

    /// The identity answers one piece at the closure's offset, and names a hole.
    #[test]
    fn the_identity_is_one_piece_or_a_named_hole() {
        let at = DevAddr::from_guest(0x10_0000);
        let id = IdentityFn(|gpa, _| (gpa < 0x8000_0000).then_some(gpa + 0x40));
        assert_eq!(
            id.resolve(at, 0x2000, RW),
            Ok(vec![piece(0x10_0040, 0x2000)])
        );
        assert_eq!(id.resolve_one(at, 0x2000, RW), Ok(piece(0x10_0040, 0x2000)));
        let hole = DevAddr::from_guest(0x9000_0000);
        assert_eq!(
            id.resolve_one(hole, 0x1000, RW),
            Err(DmaRefusal::NotGuestRam {
                at: hole,
                len: 0x1000
            })
        );
    }

    /// `resolve_one` takes exactly one piece: two are `Fragmented`, none is no guest RAM, and a
    /// refusal passes through unchanged.
    #[test]
    fn exactly_one_piece_or_a_named_refusal() {
        let at = DevAddr::from_guest(0x4000);
        assert_eq!(
            Fixed(Ok(vec![piece(0, 0x1000)])).resolve_one(at, 0x1000, RW),
            Ok(piece(0, 0x1000))
        );
        assert_eq!(
            Fixed(Ok(vec![piece(0, 0x1000), piece(0x9000, 0x1000)])).resolve_one(at, 0x2000, RW),
            Err(DmaRefusal::Fragmented {
                at,
                len: 0x2000,
                pieces: 2
            })
        );
        assert_eq!(
            Fixed(Ok(vec![])).resolve_one(at, 0x1000, RW),
            Err(DmaRefusal::NotGuestRam { at, len: 0x1000 })
        );
        let regime = DmaRefusal::Regime {
            at,
            len: 0x1000,
            regime: DmaRegime::Translating,
        };
        assert_eq!(Fixed(Err(regime)).resolve_one(at, 0x1000, RW), Err(regime));
        assert!(
            regime.to_string().contains("Translating"),
            "a refusal names its regime: {regime}"
        );
    }
}
