//! ★★★ **Guest RAM as a VMM-agnostic address map: `translate(gpa, len)`, under a layout guard**
//! (`docs/design/V3_GUEST_MEMORY.md`; owner direction 2026-10-31).
//!
//! Kayfabe does not assume the hypervisor backs guest RAM with a file. All it needs from a VMM is
//! the answer to one question: *which parts of the VMM's own mapping back the guest-physical range
//! `[gpa, gpa+len)`, in order?* That is [`LayoutGuard::translate`]:
//!
//! - the answer is [`Translated`]: a bounded list of [`RamSegment`]s plus the [`LayoutEpoch`] the
//!   answer is valid for;
//! - a [`RamSegment`] is an OBJECT (defined in `kf_linux_raw`'s `mapping_unsafe.rs`, the crate's
//!   one place for host addresses): its address is private, it borrows the guard that keeps the
//!   VMM's block alive, and the only way to narrow or touch it is through checked [`HostOffset`]
//!   arithmetic. **No VMM address is ever an integer in this crate.**
//! - a range inside one RAM block is ONE segment; a range across blocks is one segment per block
//!   touched, in guest-physical order. Segments of different blocks are never merged (two blocks
//!   adjacent in the VMM's address space are still two mappings, and only the VMM can vouch that
//!   they stay together).
//! - a byte no RAM block backs (a PCI hole, MMIO, beyond the end) is an error that NAMES the first
//!   such guest-physical address ([`TranslateError::Unbacked`]). Nothing is ever clamped.
//!
//! ⊘ **The guard.** A VMM may free or move a RAM block (hot-unplug, `virtio-mem`, a resize) between
//! `translate` and the host RM's pin. A [`GuestMemory`] therefore hands out a [`LayoutGuard`] that
//! (a) keeps the blocks alive while it lives (QEMU: a `MemoryRegion` reference per block, taken
//! under the registry lock, `kf-qemu`'s `ramguard_unsafe.rs`) and (b) carries the [`LayoutEpoch`]
//! of the layout it saw. A consumer re-checks the epoch after the RM call
//! ([`LayoutGuard::still_current`]): if it moved, the new object is freed and the map is refused
//! by name ([`LAYOUT_CHANGED`]) — never retried silently, never guessed.
//!
//! ⊘ **Pure geometry here.** [`RamLayout`] knows only `(gpa, len)` of each block. It computes which
//! `(block, offset, len)` runs a guest-physical range needs ([`RamLayout::locate`]) and
//! [`translate_blocks`] turns runs into segments by [`RamSegment::slice`] of the per-block segment
//! the VMM adapter supplied. A hypervisor implementation (QEMU now, Cloud Hypervisor later) supplies
//! its own guard and block segments and reuses both.
//!
//! ⊘ **Hostile guest.** `gpa` and `len` are guest-influenced (page-table leaves, descriptors).
//! Every sum is checked, the output is bounded by [`MAX_FRAGMENTS`], and no input can panic.

use kf_linux_raw::{HostOffset, RamSegment};

/// The most segments one [`LayoutGuard::translate`] call may return. A request needing more is
/// refused ([`TranslateError::TooFragmented`]), never truncated.
pub const MAX_FRAGMENTS: usize = 8192;

/// The most blocks a [`RamLayout`] accepts. A bound on the (VMM-authored, trusted) input so the
/// layout's own size is bounded too; far above any real machine.
pub const MAX_LAYOUT_BLOCKS: usize = 65_536;

/// ★ What a map is refused with when the guest-RAM layout changed between `translate` and the
/// host RM call that used it (the epoch moved): the new object is freed, nothing is placed.
pub const LAYOUT_CHANGED: &str = "LAYOUT_CHANGED: the guest-RAM layout changed (a RAM block was added, removed or resized) while a mapping was being prepared; nothing was placed";

/// ★ A monotonic counter of guest-RAM layout changes. The VMM adapter bumps it whenever a RAM
/// block is added, removed or resized (QEMU: from the memory listener's `region_add`/`region_del`).
/// A translate answer is valid for exactly one epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct LayoutEpoch(u64);

impl LayoutEpoch {
    /// An epoch with the given counter value.
    #[must_use]
    pub const fn new(n: u64) -> Self {
        LayoutEpoch(n)
    }

    /// The counter value (informational: logs and tests).
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl core::fmt::Display for LayoutEpoch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "layout epoch {}", self.0)
    }
}

/// One RAM block as a VMM registers it: guest-physical `[gpa, gpa + len)`. Input of
/// [`RamLayout::new`], validated there. (The block's host mapping is the adapter's business and
/// never appears here.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RamBlockSpec {
    /// Guest-physical base.
    pub gpa: u64,
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

/// Why a translate gave no segments. Refused by name, never clamped.
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
    /// The range needs more than [`MAX_FRAGMENTS`] separate segments.
    TooFragmented,
    /// The layout changed under the caller ([`LAYOUT_CHANGED`]).
    LayoutChanged,
    /// The VMM offers no layout to hold (nothing registered yet, or no way to hold its blocks).
    NoLayout,
    /// The adapter's per-block segment `index` does not match the layout's block (a defect in the
    /// adapter; refused rather than trusted).
    Inconsistent {
        /// Index of the block.
        index: usize,
    },
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
                    "guest-physical range needs more than {MAX_FRAGMENTS} segments"
                )
            }
            Self::LayoutChanged => f.write_str(LAYOUT_CHANGED),
            Self::NoLayout => f.write_str("the VMM offers no guest-RAM layout to hold"),
            Self::Inconsistent { index } => write!(
                f,
                "RAM block {index}: the VMM adapter's segment does not match the layout"
            ),
        }
    }
}

impl std::error::Error for TranslateError {}

/// One run of a guest-physical range inside one block: bytes `[off, off + len)` of block `index`.
/// Only [`RamLayout::locate`] makes one, so `off + len <= block.len` and `len > 0` hold by
/// construction; the fields are private.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockRun {
    index: usize,
    off: u64,
    len: u64,
}

impl BlockRun {
    /// Index of the block in the layout.
    #[must_use]
    pub fn index(&self) -> usize {
        self.index
    }

    /// Offset of the first byte inside the block.
    #[must_use]
    pub fn off(&self) -> u64 {
        self.off
    }

    /// Bytes (never 0).
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Never true: a run has at least one byte.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// A validated RAM layout: blocks sorted by `gpa`, non-empty, non-overlapping in guest-physical
/// space, with no `gpa + len` overflow. Pure geometry.
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

    /// ★ The `(block, offset, len)` runs that back guest-physical `[gpa, gpa + len)`, in order —
    /// one per block touched.
    ///
    /// # Errors
    /// [`TranslateError::ZeroLength`], [`TranslateError::Overflow`], [`TranslateError::Unbacked`]
    /// (the FIRST unbacked byte; no partial answer), [`TranslateError::TooFragmented`].
    pub fn locate(&self, gpa: u64, len: u64) -> Result<Vec<BlockRun>, TranslateError> {
        if len == 0 {
            return Err(TranslateError::ZeroLength);
        }
        let end = gpa
            .checked_add(len)
            .ok_or(TranslateError::Overflow { gpa, len })?;
        // The first block whose END is above `gpa` (blocks are sorted and disjoint, so their ends
        // are sorted too). Everything before it ends at or below `gpa`.
        let first = self.blocks.partition_point(|b| block_end(b) <= gpa);
        let mut out: Vec<BlockRun> = Vec::new();
        let mut at = gpa; // the next guest-physical byte still to place
        for (index, b) in self.blocks.iter().enumerate().skip(first) {
            if at >= end {
                break;
            }
            if b.gpa > at {
                // A hole before this block: `at` is the first byte nothing backs.
                return Err(TranslateError::Unbacked { gpa: at });
            }
            // `at` is inside `b` here: b.gpa <= at < block_end(b).
            let take_end = block_end(b).min(end);
            if out.len() >= MAX_FRAGMENTS {
                return Err(TranslateError::TooFragmented);
            }
            out.push(BlockRun {
                index,
                off: at.saturating_sub(b.gpa),
                len: take_end.saturating_sub(at),
            });
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

/// ★ A translate answer: the segments backing a guest-physical range, in guest-physical order,
/// and the [`LayoutEpoch`] they are valid for. Borrowed from the [`LayoutGuard`] that produced it.
#[derive(Debug)]
pub struct Translated<'g> {
    epoch: LayoutEpoch,
    segments: Vec<RamSegment<'g>>,
}

impl<'g> Translated<'g> {
    /// The epoch of the layout this answer was computed from.
    #[must_use]
    pub fn epoch(&self) -> LayoutEpoch {
        self.epoch
    }

    /// The segments: non-empty, lengths summing to the requested `len`, at most
    /// [`MAX_FRAGMENTS`], one per block touched.
    #[must_use]
    pub fn segments(&self) -> &[RamSegment<'g>] {
        &self.segments
    }

    /// Take the segments.
    #[must_use]
    pub fn into_segments(self) -> Vec<RamSegment<'g>> {
        self.segments
    }

    /// Total bytes (the requested length).
    #[must_use]
    pub fn total_len(&self) -> u64 {
        self.segments
            .iter()
            .fold(0u64, |a, s| a.saturating_add(s.len()))
    }

    /// Copy the whole range into `dst` (`dst.len()` must equal [`Translated::total_len`]).
    ///
    /// # Errors
    /// A length mismatch is refused by name; a segment copy error is passed up.
    pub fn read_into(&self, dst: &mut [u8]) -> Result<(), kf_linux_raw::RawError> {
        let dst_len = dst.len();
        self.copy(dst_len, |seg, at, n| {
            let end = at.saturating_add(n);
            let part = dst
                .get_mut(at..end)
                .ok_or(kf_linux_raw::RawError::OutOfRange {
                    offset: at as u64,
                    len: n as u64,
                    object_len: dst_len as u64,
                })?;
            seg.read_into(HostOffset::ZERO, part)
        })
    }

    /// Copy `src` over the whole range (`src.len()` must equal [`Translated::total_len`]).
    ///
    /// # Errors
    /// A length mismatch is refused by name; a segment copy error is passed up.
    pub fn write_from(&self, src: &[u8]) -> Result<(), kf_linux_raw::RawError> {
        self.copy(src.len(), |seg, at, n| {
            let end = at.saturating_add(n);
            let part = src.get(at..end).ok_or(kf_linux_raw::RawError::OutOfRange {
                offset: at as u64,
                len: n as u64,
                object_len: src.len() as u64,
            })?;
            seg.write_from(HostOffset::ZERO, part)
        })
    }

    fn copy(
        &self,
        buf_len: usize,
        mut each: impl FnMut(&RamSegment<'g>, usize, usize) -> Result<(), kf_linux_raw::RawError>,
    ) -> Result<(), kf_linux_raw::RawError> {
        let total = self.total_len();
        if total != buf_len as u64 {
            return Err(kf_linux_raw::RawError::OutOfRange {
                offset: 0,
                len: buf_len as u64,
                object_len: total,
            });
        }
        let mut at = 0usize;
        for seg in &self.segments {
            let n = usize::try_from(seg.len())
                .map_err(|_| kf_linux_raw::RawError::TooLargeForHost { value: seg.len() })?;
            each(seg, at, n)?;
            at = at.saturating_add(n);
        }
        Ok(())
    }
}

/// ★ **A hold on the VMM's guest-RAM blocks.** While it lives, every segment it produced is valid
/// (the VMM cannot free or unmap the blocks); it carries the epoch of the layout it saw.
pub trait LayoutGuard {
    /// The epoch of the layout this guard was taken at.
    fn epoch(&self) -> LayoutEpoch;

    /// Whether the VMM's layout is STILL the one this guard saw (the epoch has not moved). A
    /// consumer calls this AFTER the host RM call that used a segment: `false` ⇒ free what the
    /// call made and refuse by name ([`LAYOUT_CHANGED`]).
    fn still_current(&self) -> bool;

    /// The block geometry this guard saw (pure `(gpa, len)`; no host address).
    fn layout(&self) -> &RamLayout;

    /// The segments backing guest-physical `[gpa, gpa + len)`, in order, with the epoch.
    ///
    /// # Errors
    /// [`TranslateError`], by name. On any error nothing is returned (no partial answer).
    fn translate(&self, gpa: u64, len: u64) -> Result<Translated<'_>, TranslateError>;
}

/// ★ **What kayfabe needs to know about a VMM's guest RAM** — and nothing about how it is backed.
pub trait GuestMemory {
    /// The guard type, borrowed from the memory for `'g`.
    type Guard<'g>: LayoutGuard
    where
        Self: 'g;

    /// Take a hold on the current layout.
    ///
    /// # Errors
    /// The VMM has no usable layout (none registered, or the registry is not a layout).
    fn lock_layout(&self) -> Result<Self::Guard<'_>, TranslateError>;

    /// The CURRENT epoch (cheap, lock-free; for cached results).
    fn epoch(&self) -> LayoutEpoch;
}

/// ★ Turn runs into segments: the shared implementation of [`LayoutGuard::translate`] for any
/// VMM adapter. `blocks[i]` is the adapter's segment over the WHOLE of block `i` of `layout`; each
/// run narrows it by [`RamSegment::slice`], so every offset and length is established by checked
/// arithmetic against the segment itself, not trusted from the layout.
///
/// # Errors
/// As [`RamLayout::locate`]; [`TranslateError::Inconsistent`] if a needed block's segment is
/// missing or not the layout block's length.
pub fn translate_blocks<'g>(
    layout: &RamLayout,
    blocks: &[RamSegment<'g>],
    epoch: LayoutEpoch,
    gpa: u64,
    len: u64,
) -> Result<Translated<'g>, TranslateError> {
    let runs = layout.locate(gpa, len)?;
    let mut segments: Vec<RamSegment<'g>> = Vec::with_capacity(runs.len());
    for run in runs {
        let index = run.index();
        let (Some(whole), Some(spec)) = (blocks.get(index), layout.blocks().get(index)) else {
            return Err(TranslateError::Inconsistent { index });
        };
        if whole.len() != spec.len {
            return Err(TranslateError::Inconsistent { index });
        }
        let seg = whole
            .slice(HostOffset::new(run.off()), run.len())
            .map_err(|_| TranslateError::Inconsistent { index })?;
        segments.push(seg);
    }
    Ok(Translated { epoch, segments })
}

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) mod testvmm;
