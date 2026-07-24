//! GMMU walk algorithm — regime-independent core logic (SKELETON this milestone).
//!
//! The walk *algorithm* is core (one loop, all regimes); the entry *format* is
//! Axis-B ([`nvkvm_arch::GmmuFmt`]). The full walker ports in the `nvkvm-mmu`
//! migration step (arch doc §4.5 step 3), property-tested against ogkm format
//! definitions and replayed against #13's banked traces. Requirements it must meet:
//!
//! - decode **every** leaf size [`nvkvm_arch::GmmuFmt::page_sizes`] enumerates
//!   (the #13 GA10x PD1 512M-leaf lesson: an un-enumerated size is a loud fault,
//!   never a silent drop);
//! - used **only** at forward-populate commit points (decode dirtied PT pages
//!   directly at the CE release semaphore — never a root walk racing a
//!   leaf-filled-then-linked build order, and never as a resolve fallback);
//! - read PT bytes only through an abstract `FbRead` source, so it is unit-testable
//!   against synthetic FB images.

use nvkvm_arch::ids::Pdb;

/// Abstract page-table byte source (a synthetic FB image in tests; the FB shadow
/// in production). Keeps the walker pure.
pub trait FbRead {
    /// Read `buf.len()` bytes at physical address `phys`; `false` if unbacked.
    fn read(&self, phys: u64, buf: &mut [u8]) -> bool;
}

/// Result of a (future) walk. Placeholder shape for the milestone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkResult {
    /// VA resolves to a leaf.
    Mapped {
        /// Physical address of the page.
        phys: u64,
    },
    /// The walk hit an invalid entry — a loud fault at the commit point.
    Fault {
        /// Directory level (0 = root) where the walk faulted.
        level: u8,
        /// The PDB being walked.
        pdb: Pdb,
    },
}
