//! ★★★ **Guest RAM as a VMM-agnostic address map: `translate(gpa, len)`** (`docs/design/V3_GUEST_MEMORY.md`).
//!
//! Kayfabe does not assume the hypervisor backs guest RAM with a file. All it needs from a VMM is
//! the answer to one question: *which host (VMM-address-space) blocks back the guest-physical range
//! `[gpa, gpa+len)`, in order?* That is [`GuestMemory::translate`].
//!
//! - A range inside one RAM block is ONE [`HostBlock`].
//! - A range that spans RAM blocks, or whose blocks are not contiguous in the VMM's address space,
//!   returns several, in guest-physical order. Fragmented output is the edge case and exists for
//!   correctness: contiguity in the VMM address space is NEVER guaranteed, access is possible block
//!   by block.
//! - Adjacent guest-physical blocks whose host ranges are also adjacent are MERGED into one block.
//! - A byte no RAM block backs (a PCI hole, MMIO, beyond the end) is an error that NAMES the first
//!   such guest-physical address ([`TranslateError::Unbacked`]). Nothing is ever clamped.
//!
//! ⊘ **Pure integers.** [`HostBlock::hva`] is a `u64`, never a pointer: turning an address into a
//! pointer happens only in `*_unsafe.rs` files of `kf-linux-raw`, `kf-qemu` and `kf-cuda`.
//!
//! ⊘ **Hostile guest.** `gpa` and `len` are guest-influenced (page-table leaves, descriptors).
//! Every sum is checked, the output is bounded by [`MAX_FRAGMENTS`] (a guest cannot make this
//! allocate more than `MAX_FRAGMENTS * 16` bytes, whatever it asks for), and no input can panic.
//!
//! ⊘ **The optional fd.** A VMM whose RAM is a file (QEMU's `memory-backend-memfd`) may ALSO
//! offer [`GuestMemory::export_fd`]: the fd and file offset of a range. Only the GPU-side stitch
//! (`kf_linux_raw::MappedRegion::stitch`) uses it; with the default `None`, that stitch is not
//! available and the per-run path is used instead. Nothing else may require an fd.

use std::os::fd::BorrowedFd;

/// The most fragments one [`GuestMemory::translate`] call may return. A request needing more is
/// refused ([`TranslateError::TooFragmented`]), never truncated. 8192 blocks cover 8192 discontiguous
/// VMM mappings; a real VMM registers a handful (q35: two RAM blocks around the PCI hole).
pub const MAX_FRAGMENTS: usize = 8192;

/// The most blocks a [`RamLayout`] accepts. A bound on the (VMM-authored, trusted) input so the
/// layout's own size is bounded too; far above any real machine.
pub const MAX_LAYOUT_BLOCKS: usize = 65_536;

/// One run of VMM address space that backs part of a guest-physical range: `[hva, hva + len)`.
///
/// Plain integers: the address of a byte in the VMM process, not a pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostBlock {
    /// VMM address of the first byte.
    pub hva: u64,
    /// Bytes (never 0 in a [`GuestMemory::translate`] result).
    pub len: u64,
}

/// One RAM block as a VMM registers it: guest-physical `[gpa, gpa + len)` lives at VMM address
/// `hva`. Input of [`RamLayout::new`], validated there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RamBlockSpec {
    /// Guest-physical base.
    pub gpa: u64,
    /// VMM address of `gpa`.
    pub hva: u64,
    /// Bytes.
    pub len: u64,
}

/// Why a block list is not a [`RamLayout`]. Every variant names the offending index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    /// Block `index` has `len == 0`.
    ZeroLength {
        /// Index in the input.
        index: usize,
    },
    /// Block `index`: `gpa + len` overflows `u64`.
    GpaOverflow {
        /// Index in the input.
        index: usize,
    },
    /// Block `index`: `hva + len` overflows `u64`.
    HvaOverflow {
        /// Index in the input.
        index: usize,
    },
    /// Block `index` starts below the previous block's start (the input must be sorted by `gpa`).
    Unsorted {
        /// Index in the input.
        index: usize,
    },
    /// Block `index` starts before the previous block ends.
    Overlap {
        /// Index in the input.
        index: usize,
    },
    /// More than [`MAX_LAYOUT_BLOCKS`] blocks.
    TooManyBlocks,
}

impl core::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroLength { index } => write!(f, "RAM block {index}: length 0"),
            Self::GpaOverflow { index } => write!(f, "RAM block {index}: gpa + len overflows"),
            Self::HvaOverflow { index } => write!(f, "RAM block {index}: hva + len overflows"),
            Self::Unsorted { index } => write!(f, "RAM block {index}: not sorted by gpa"),
            Self::Overlap { index } => {
                write!(
                    f,
                    "RAM block {index}: overlaps the previous block in guest-physical space"
                )
            }
            Self::TooManyBlocks => write!(f, "more than {MAX_LAYOUT_BLOCKS} RAM blocks"),
        }
    }
}

impl std::error::Error for LayoutError {}

/// Why [`GuestMemory::translate`] gave no blocks. Refused by name, never clamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslateError {
    /// `len == 0`: a zero-length access names no byte, so it is a caller defect, not an empty answer.
    ZeroLength,
    /// `gpa + len` overflows `u64`.
    Overflow {
        /// The requested start.
        gpa: u64,
        /// The requested length.
        len: u64,
    },
    /// The first guest-physical address in the range that no RAM block backs (a hole, MMIO, or past
    /// the end of RAM).
    Unbacked {
        /// That address.
        gpa: u64,
    },
    /// The range needs more than [`MAX_FRAGMENTS`] separate host blocks.
    TooFragmented,
}

impl core::fmt::Display for TranslateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroLength => write!(f, "guest-physical range of length 0"),
            Self::Overflow { gpa, len } => {
                write!(f, "guest-physical range {gpa:#x}+{len:#x} overflows")
            }
            Self::Unbacked { gpa } => {
                write!(f, "guest-physical address {gpa:#x} is not backed by RAM")
            }
            Self::TooFragmented => {
                write!(
                    f,
                    "guest-physical range needs more than {MAX_FRAGMENTS} host blocks"
                )
            }
        }
    }
}

impl std::error::Error for TranslateError {}

/// A file range backing guest-physical memory — the optional capability of [`GuestMemory::export_fd`].
#[derive(Debug, Clone, Copy)]
pub struct FdRange<'a> {
    /// The backing file.
    pub fd: BorrowedFd<'a>,
    /// File offset of the first byte of the range.
    pub offset: u64,
}

/// ★ **What kayfabe needs to know about a VMM's guest RAM.**
pub trait GuestMemory {
    /// The host blocks backing guest-physical `[gpa, gpa + len)`, in guest-physical order; each is
    /// non-empty; their lengths sum to `len`; adjacent blocks are never mergeable (a mergeable pair
    /// is one block). At most [`MAX_FRAGMENTS`] of them.
    ///
    /// # Errors
    /// [`TranslateError`], by name. On any error nothing is returned (no partial answer).
    fn translate(&self, gpa: u64, len: u64) -> Result<Vec<HostBlock>, TranslateError>;

    /// ★ OPTIONAL capability: the file and file offset backing `[gpa, gpa + len)`, when the VMM's
    /// RAM is a file AND the whole range is one contiguous range of ONE file. `None` is the normal
    /// answer for a VMM that does not back RAM with a file, for a range that is not contiguous in
    /// the file, and for an unbacked range; it never means an error. Only the GPU-side stitch uses
    /// this; it then falls back, by name, to the per-run path.
    fn export_fd(&self, gpa: u64, len: u64) -> Option<FdRange<'_>> {
        let _ = (gpa, len);
        None
    }
}

/// A validated RAM layout: blocks sorted by `gpa`, non-empty, non-overlapping in guest-physical
/// space, with no `gpa + len` or `hva + len` overflow. (Host ranges MAY overlap or alias — a VMM
/// can map one host block at two guest addresses; translate answers each query independently.)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RamLayout {
    blocks: Vec<RamBlockSpec>,
}

impl RamLayout {
    /// Validate `blocks` (sorted by `gpa`).
    ///
    /// # Errors
    /// The first violation, by name and index.
    pub fn new(blocks: &[RamBlockSpec]) -> Result<Self, LayoutError> {
        if blocks.len() > MAX_LAYOUT_BLOCKS {
            return Err(LayoutError::TooManyBlocks);
        }
        let mut prev_end: Option<u64> = None;
        let mut prev_gpa: Option<u64> = None;
        for (index, b) in blocks.iter().enumerate() {
            if b.len == 0 {
                return Err(LayoutError::ZeroLength { index });
            }
            let end = b
                .gpa
                .checked_add(b.len)
                .ok_or(LayoutError::GpaOverflow { index })?;
            b.hva
                .checked_add(b.len)
                .ok_or(LayoutError::HvaOverflow { index })?;
            if prev_gpa.is_some_and(|p| b.gpa < p) {
                return Err(LayoutError::Unsorted { index });
            }
            if prev_end.is_some_and(|e| b.gpa < e) {
                return Err(LayoutError::Overlap { index });
            }
            prev_gpa = Some(b.gpa);
            prev_end = Some(end);
        }
        Ok(Self {
            blocks: blocks.to_vec(),
        })
    }

    /// The validated blocks.
    #[must_use]
    pub fn blocks(&self) -> &[RamBlockSpec] {
        &self.blocks
    }

    /// Total bytes of RAM (saturating; informational).
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.blocks
            .iter()
            .fold(0u64, |a, b| a.saturating_add(b.len))
    }
}

impl GuestMemory for RamLayout {
    fn translate(&self, gpa: u64, len: u64) -> Result<Vec<HostBlock>, TranslateError> {
        if len == 0 {
            return Err(TranslateError::ZeroLength);
        }
        let end = gpa
            .checked_add(len)
            .ok_or(TranslateError::Overflow { gpa, len })?;
        // The first block whose END is above `gpa` (blocks are sorted and disjoint, so their ends
        // are sorted too). Everything before it ends at or below `gpa`.
        let first = self.blocks.partition_point(|b| block_end(b) <= gpa);
        let mut out: Vec<HostBlock> = Vec::new();
        let mut at = gpa; // the next guest-physical byte still to place
        for b in self.blocks.iter().skip(first) {
            if at >= end {
                break;
            }
            if b.gpa > at {
                // A hole before this block: `at` is the first byte nothing backs.
                return Err(TranslateError::Unbacked { gpa: at });
            }
            // `at` is inside `b` here: b.gpa <= at < block_end(b).
            let take_end = block_end(b).min(end);
            let take = take_end.saturating_sub(at);
            let hva = b.hva.saturating_add(at.saturating_sub(b.gpa));
            match out.last_mut() {
                Some(last) if last.hva.checked_add(last.len) == Some(hva) => {
                    last.len = last.len.saturating_add(take);
                }
                _ => {
                    if out.len() >= MAX_FRAGMENTS {
                        return Err(TranslateError::TooFragmented);
                    }
                    out.push(HostBlock { hva, len: take });
                }
            }
            at = take_end;
        }
        if at < end {
            // Ran out of blocks: the byte after the last placed one is unbacked.
            return Err(TranslateError::Unbacked { gpa: at });
        }
        Ok(out)
    }
}

/// `gpa + len` of a validated block (never overflows; saturates defensively).
fn block_end(b: &RamBlockSpec) -> u64 {
    b.gpa.saturating_add(b.len)
}

#[cfg(test)]
mod tests;
