//! # kayfabe-mmu — the address plane
//!
//! Owns THE ADDRESS TABLE (`mode2_address_table.md`, the one-table-of-truth
//! directive): **one authoritative, forward-populated VA→phys table per VAS, keyed by
//! PDB** — never by client handle (lesson L7: many clients share one VAS; a
//! GSP-managed `hVASpace=0` channel has no client-keyed VAS at all).
//!
//! The rules, as API shape (they are enforced *by construction*, testing strategy
//! `taddr_forward_only`):
//!
//! - **Forward-populate only.** [`AddressTable::bind`] is the only way in — called at
//!   bind time (RPC populate source) or at a CE-PT-write commit point (#13's populate
//!   source). There is no exec-time reverse-resolve entry point in this API.
//! - **MISS = FAULT.** [`AddressTable::resolve`] returns [`AddressFault::Miss`] on a
//!   miss. There is no fallback walk (a torn multi-level walk = wrong phys =
//!   cross-context leak — MISS=FAULT is a *security* property, arch doc §4.3.5).
//! - **Unmap eager**, map lazy, reclaim deferred ([`AddressTable::unbind`]).
//!
//! Also here (skeleton this milestone): the GMMU **walk algorithm** — regime-independent
//! core logic written against [`kayfabe_arch::GmmuFmt`], used only at forward-populate
//! commit points (decode-dirtied-PT-pages, #13), never as a resolve fallback.
//!
//! Concurrency (decision #17): plain owned data, no interior mutability;
//! [`AddressTable::resolve`]/[`AddressTable::iter`] are `&self` (concurrent reads
//! safe), bind/unbind are `&mut self` (caller-exclusive). `Send + Sync`
//! compile-time-asserted below; full contract in `kayfabe-core`'s crate docs.

pub mod walker;

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuVa, Pdb};
use kayfabe_util::IntervalMap;

/// Where a bound VA range points, in core terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    /// Physical/backing address (interpretation depends on `aperture`; for sysmem
    /// this is a guest-physical address).
    pub phys: u64,
    /// Aperture of the backing.
    pub aperture: Aperture,
    /// Host GPU VA this range is published at in the owning Vas's host address
    /// space, once materialized (`None` until the fwd plane maps it). Per-Vas
    /// host placement is #14's proven fix — see `kayfabe-fwd`.
    pub host_va: Option<u64>,
}

/// A resolution failure. Every variant is LOUD: callers propagate, they never guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressFault {
    /// No binding covers the VA — the guest's own TLB would have faulted too.
    Miss {
        /// The PDB whose table missed.
        pdb: Pdb,
        /// The faulting VA.
        va: GpuVa,
    },
    /// A bind attempted to overlap a live binding without an eager unbind first
    /// (the `ALREADY-MAPPED` collision class made loud).
    Overlap {
        /// The PDB whose table refused the bind.
        pdb: Pdb,
        /// Start of the attempted range.
        va: GpuVa,
    },
    /// A bind named a malformed range — zero length, or `va + len` wraps `u64`. The
    /// `va`/`len` are guest-controlled (a hostile `MapMemoryDma` or a hostile CE
    /// PT-write dst), so this is a clean loud fault, never an arithmetic panic
    /// (boundary-1 posture; regression-banked).
    Malformed {
        /// The PDB whose table refused the bind.
        pdb: Pdb,
        /// Start of the attempted (malformed) range.
        va: GpuVa,
    },
}

/// The forward-populated VA→backing table of ONE VAS (identified by its PDB).
///
/// This *is* the guest's GMMU TLB from the emulator's point of view: populated at
/// the guest's own publication points, invalidated by the guest's own invalidate
/// discipline, faulting where real hardware would fault.
#[derive(Debug, Default)]
pub struct AddressTable {
    map: IntervalMap<Binding>,
}

impl AddressTable {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Forward-populate `[va, va+len)` → `binding` (bind-time RPC or CE-PT-write
    /// commit point). Overlap with a live binding is a loud fault: the guest must
    /// have unbound first (unmap eager); silently replacing would hide the #14
    /// collision class.
    pub fn bind(
        &mut self,
        pdb: Pdb,
        va: GpuVa,
        len: u64,
        binding: Binding,
    ) -> Result<(), AddressFault> {
        self.map.insert(va.0, len, binding).map_err(|e| match e {
            kayfabe_util::IntervalError::Overlap { .. } => AddressFault::Overlap { pdb, va },
            // Zero-length / u64-wrapping range from hostile guest input: loud, not a panic.
            kayfabe_util::IntervalError::Empty | kayfabe_util::IntervalError::Wraps => {
                AddressFault::Malformed { pdb, va }
            }
        })
    }

    /// Eagerly drop the binding starting at `va`. Returns the dropped binding so the
    /// caller can retire its host backing (reclaim deferred).
    pub fn unbind(&mut self, va: GpuVa) -> Option<(u64, Binding)> {
        self.map.remove_at(va.0)
    }

    /// Resolve `va` to its binding + offset within it. **MISS = FAULT** — there is no
    /// fallback and never will be (see crate docs).
    pub fn resolve(&self, pdb: Pdb, va: GpuVa) -> Result<(Binding, u64), AddressFault> {
        match self.map.lookup(va.0) {
            Some((start, _len, b)) => Ok((*b, va.0 - start)),
            None => Err(AddressFault::Miss { pdb, va }),
        }
    }

    /// Iterate bindings as `(va, len, &binding)` in ascending VA order.
    pub fn iter(&self) -> impl Iterator<Item = (u64, u64, &Binding)> {
        self.map.iter()
    }
}

// The concurrency contract, compile-time-asserted (decision #17).
kayfabe_util::assert_send_sync!(AddressTable, Binding, AddressFault, walker::WalkResult);

#[cfg(test)]
mod tests {
    use super::*;

    const PDB: Pdb = Pdb(0x340_1000);

    /// testing strategy §2.4 `taddr_miss_is_fault`: a lookup miss is a loud fault,
    /// never an opportunistic walk.
    #[test]
    fn taddr_miss_is_fault() {
        let mut t = AddressTable::new();
        t.bind(
            PDB,
            GpuVa(0x2_0020_0000),
            0x10000,
            Binding {
                phys: 0x8000_0000,
                aperture: Aperture::SysmemCoherent,
                host_va: None,
            },
        )
        .unwrap();
        // In range: resolves with offset.
        let (b, off) = t.resolve(PDB, GpuVa(0x2_0020_4000)).unwrap();
        assert_eq!((b.phys, off), (0x8000_0000, 0x4000));
        // Out of range: FAULT, carrying the identity needed for a loud diagnostic.
        assert_eq!(
            t.resolve(PDB, GpuVa(0x2_0030_0000)),
            Err(AddressFault::Miss {
                pdb: PDB,
                va: GpuVa(0x2_0030_0000)
            })
        );
    }

    /// `taddr_unmap_eager`: a removed range faults immediately; rebinding after an
    /// eager unbind succeeds; rebinding over a live range is a loud overlap fault.
    #[test]
    fn taddr_unmap_eager_and_overlap_loud() {
        let mut t = AddressTable::new();
        let bind = Binding {
            phys: 0x1000,
            aperture: Aperture::Vidmem,
            host_va: None,
        };
        t.bind(PDB, GpuVa(0x1000), 0x1000, bind).unwrap();
        assert_eq!(
            t.bind(PDB, GpuVa(0x1800), 0x1000, bind),
            Err(AddressFault::Overlap {
                pdb: PDB,
                va: GpuVa(0x1800)
            }),
            "re-point without unbind must be loud"
        );
        assert!(t.unbind(GpuVa(0x1000)).is_some());
        assert!(matches!(
            t.resolve(PDB, GpuVa(0x1000)),
            Err(AddressFault::Miss { .. })
        ));
        t.bind(PDB, GpuVa(0x1800), 0x1000, bind).unwrap();
    }
}
