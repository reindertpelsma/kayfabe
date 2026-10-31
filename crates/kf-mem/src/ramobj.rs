//! ★★★ **The GPU-side view of guest RAM when it is NOT one file: RAM objects, one per host block**
//! (`docs/design/V3_GUEST_MEMORY.md` §5).
//!
//! With a memfd the whole of guest RAM is ONE host object (an OS descriptor over the file) and a
//! sysmem row's `Desired::off` is a file offset (`RamObjects` is not involved, [`crate::ledger::HostVas::ram_obj`]).
//! A VMM that does not back RAM with a file offers only `translate(gpa, len)`
//! ([`crate::guestmem::GuestMemory`]), whose answer can be several host blocks that are not
//! contiguous in the VMM's address space. One OS descriptor describes ONE contiguous address range,
//! so the host holds ONE OBJECT PER HOST SEGMENT, and a sysmem row names a position in the
//! concatenation of the segments:
//!
//! - the **RAM offset** of a guest-physical byte is its position among the bytes of guest RAM taken
//!   in guest-physical order ([`RamPlan::ram_offset`]); adjacent blocks are adjacent in it;
//! - a [`RamSegment`] is one object and the RAM-offset range it carries ([`RamObjects::resolve`]);
//! - a row whose range crosses a segment edge is cut there ([`RamObjects::cuts`]): **one host map per
//!   segment, never one map across the edge** (two segments may be anywhere in the VMM's address
//!   space, so the bytes after the edge are not the bytes of the next object).
//!
//! ⊘ Pure: no unsafe, no host call. The objects themselves are made by kf-qemu from a [`RamPlan`].
//!
//! ⊘ **Hostile guest.** A row's range comes from guest page tables. Every sum is checked; a range
//! not wholly covered is refused with the first uncovered RAM offset ([`RamObjError::Uncovered`]);
//! the number of cuts is bounded by the number of segments ([`MAX_RAM_SEGMENTS`]).

use crate::guestmem::{MAX_LAYOUT_BLOCKS, RamLayout};

/// The grain every segment edge must sit on: the host's map granule for a guest-RAM row.
pub const RAM_OBJ_ALIGN: u64 = 0x1000;

/// The most segments a [`RamObjects`] or [`RamPlan`] holds.
pub const MAX_RAM_SEGMENTS: usize = MAX_LAYOUT_BLOCKS;

/// One host object and the RAM-offset range it carries: RAM offset `ram_off + i` is byte
/// `obj_off + i` of object `obj`, for `i < len`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RamSegment {
    /// First RAM offset.
    pub ram_off: u64,
    /// Bytes.
    pub len: u64,
    /// The host object (an OS descriptor over one host block).
    pub obj: u32,
    /// Offset of `ram_off` inside `obj`.
    pub obj_off: u64,
}

/// One piece of a row after the cut: wholly inside one object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RamSlice {
    /// First RAM offset of the piece.
    pub ram_off: u64,
    /// Bytes.
    pub len: u64,
    /// The host object.
    pub obj: u32,
    /// Offset inside `obj`.
    pub obj_off: u64,
}

/// Why a segment list is not a [`RamObjects`], or why a range is not served. Refused by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RamObjError {
    /// Segment `index` has `len == 0`.
    ZeroLength {
        /// Index in the input.
        index: usize,
    },
    /// Segment `index`: `ram_off`, `len` or `obj_off` is not a multiple of [`RAM_OBJ_ALIGN`].
    Unaligned {
        /// Index in the input.
        index: usize,
    },
    /// Segment `index`: `ram_off + len` or `obj_off + len` overflows.
    Overflow {
        /// Index in the input.
        index: usize,
    },
    /// Segment `index` starts before the previous one ends (the input must be sorted by `ram_off`).
    Overlap {
        /// Index in the input.
        index: usize,
    },
    /// More than [`MAX_RAM_SEGMENTS`] segments.
    TooMany,
    /// A range of length 0.
    ZeroRange,
    /// `off + len` overflows.
    RangeOverflow,
    /// The first RAM offset in the range no segment carries.
    Uncovered {
        /// That offset.
        at: u64,
    },
    /// [`RamObjects::resolve`]: the range reaches past the end of the segment it starts in (use
    /// [`RamObjects::split`] to cut it). `at` is the first offset of the next segment's side.
    Crosses {
        /// The offset where the first segment ends.
        at: u64,
    },
}

impl core::fmt::Display for RamObjError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroLength { index } => write!(f, "RAM segment {index}: length 0"),
            Self::Unaligned { index } => {
                write!(f, "RAM segment {index}: not {RAM_OBJ_ALIGN:#x}-aligned")
            }
            Self::Overflow { index } => write!(f, "RAM segment {index}: offset + length overflows"),
            Self::Overlap { index } => write!(
                f,
                "RAM segment {index}: overlaps or precedes the previous one"
            ),
            Self::TooMany => write!(f, "more than {MAX_RAM_SEGMENTS} RAM segments"),
            Self::ZeroRange => write!(f, "RAM range of length 0"),
            Self::RangeOverflow => write!(f, "RAM range overflows"),
            Self::Uncovered { at } => write!(f, "RAM offset {at:#x} is carried by no host object"),
            Self::Crosses { at } => {
                write!(
                    f,
                    "RAM range crosses a host-object edge at RAM offset {at:#x}"
                )
            }
        }
    }
}

impl std::error::Error for RamObjError {}

/// The host objects that carry guest RAM, as a table: sorted by RAM offset, disjoint, aligned.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RamObjects {
    segs: Vec<RamSegment>,
}

impl RamObjects {
    /// Validate `segs` (sorted by `ram_off`).
    ///
    /// # Errors
    /// The first violation, by name and index.
    pub fn new(segs: &[RamSegment]) -> Result<Self, RamObjError> {
        if segs.len() > MAX_RAM_SEGMENTS {
            return Err(RamObjError::TooMany);
        }
        let mut prev_end: Option<u64> = None;
        for (index, s) in segs.iter().enumerate() {
            if s.len == 0 {
                return Err(RamObjError::ZeroLength { index });
            }
            if ![s.ram_off, s.len, s.obj_off]
                .iter()
                .all(|v| v.is_multiple_of(RAM_OBJ_ALIGN))
            {
                return Err(RamObjError::Unaligned { index });
            }
            let end = s
                .ram_off
                .checked_add(s.len)
                .ok_or(RamObjError::Overflow { index })?;
            s.obj_off
                .checked_add(s.len)
                .ok_or(RamObjError::Overflow { index })?;
            if prev_end.is_some_and(|e| s.ram_off < e) {
                return Err(RamObjError::Overlap { index });
            }
            prev_end = Some(end);
        }
        Ok(Self {
            segs: segs.to_vec(),
        })
    }

    /// ONE object carrying `[0, len)` at the same offsets — the memfd shape, for tests that want
    /// the single-object behaviour through this table.
    ///
    /// # Errors
    /// As [`RamObjects::new`].
    pub fn whole(obj: u32, len: u64) -> Result<Self, RamObjError> {
        Self::new(&[RamSegment {
            ram_off: 0,
            len,
            obj,
            obj_off: 0,
        }])
    }

    /// The segments.
    #[must_use]
    pub fn segments(&self) -> &[RamSegment] {
        &self.segs
    }

    /// Whether no object is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    /// Index of the segment holding `off`, if any.
    fn find(&self, off: u64) -> Option<usize> {
        let i = self
            .segs
            .partition_point(|s| s.ram_off.saturating_add(s.len) <= off);
        self.segs.get(i).filter(|s| s.ram_off <= off).map(|_| i)
    }

    /// The object and object offset of RAM `[off, off + len)` when ONE segment carries all of it.
    ///
    /// # Errors
    /// [`RamObjError::Uncovered`] (the range starts in no segment), [`RamObjError::Crosses`] (it
    /// reaches past the end of the segment it starts in), [`RamObjError::ZeroRange`],
    /// [`RamObjError::RangeOverflow`].
    pub fn resolve(&self, off: u64, len: u64) -> Result<(u32, u64), RamObjError> {
        if len == 0 {
            return Err(RamObjError::ZeroRange);
        }
        let end = off.checked_add(len).ok_or(RamObjError::RangeOverflow)?;
        let i = self.find(off).ok_or(RamObjError::Uncovered { at: off })?;
        let s = self.segs.get(i).ok_or(RamObjError::Uncovered { at: off })?;
        let s_end = s.ram_off.saturating_add(s.len);
        if end > s_end {
            return Err(RamObjError::Crosses { at: s_end });
        }
        Ok((
            s.obj,
            s.obj_off.saturating_add(off.saturating_sub(s.ram_off)),
        ))
    }

    /// The RAM range `[off, off + len)` cut at every segment edge inside it: one [`RamSlice`] per
    /// segment it touches, in order. The slices are contiguous in RAM offsets and sum to `len`.
    ///
    /// # Errors
    /// [`RamObjError::Uncovered`] with the first RAM offset no segment carries (a gap between two
    /// segments, or the range reaches past the last one) — nothing is returned for a partly
    /// covered range; [`RamObjError::ZeroRange`]; [`RamObjError::RangeOverflow`].
    pub fn split(&self, off: u64, len: u64) -> Result<Vec<RamSlice>, RamObjError> {
        if len == 0 {
            return Err(RamObjError::ZeroRange);
        }
        let end = off.checked_add(len).ok_or(RamObjError::RangeOverflow)?;
        let mut at = off;
        let mut out: Vec<RamSlice> = Vec::new();
        while at < end {
            let i = self.find(at).ok_or(RamObjError::Uncovered { at })?;
            let s = self.segs.get(i).ok_or(RamObjError::Uncovered { at })?;
            let take_end = s.ram_off.saturating_add(s.len).min(end);
            out.push(RamSlice {
                ram_off: at,
                len: take_end.saturating_sub(at),
                obj: s.obj,
                obj_off: s.obj_off.saturating_add(at.saturating_sub(s.ram_off)),
            });
            at = take_end;
        }
        Ok(out)
    }

    /// The RAM offsets strictly inside `(off, off + len)` where one segment ends and the NEXT
    /// begins at the same offset: where a row must be cut so that no host map spans two objects.
    /// Ascending. Empty for an unaddressable range (overflow, zero length) — the row is refused
    /// elsewhere, by name.
    #[must_use]
    pub fn cuts(&self, off: u64, len: u64) -> Vec<u64> {
        let Some(end) = off.checked_add(len).filter(|_| len != 0) else {
            return Vec::new();
        };
        // The first segment whose START is above `off`.
        let first = self.segs.partition_point(|s| s.ram_off <= off);
        self.segs
            .iter()
            .skip(first)
            .take_while(|s| s.ram_off < end)
            .map(|s| s.ram_off)
            .collect()
    }
}

/// One host segment of a [`RamPlan`]: the VMM address range an object must describe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanSegment {
    /// First RAM offset.
    pub ram_off: u64,
    /// VMM address of that byte.
    pub hva: u64,
    /// Bytes.
    pub len: u64,
}

/// Why a layout cannot be planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    /// Block `index`: gpa, hva or length is not [`RAM_OBJ_ALIGN`]-aligned.
    Unaligned {
        /// Index in the layout.
        index: usize,
    },
    /// The layout has no block.
    Empty,
    /// The total size overflows `u64`.
    Overflow,
}

impl core::fmt::Display for PlanError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unaligned { index } => {
                write!(
                    f,
                    "RAM block {index}: not {RAM_OBJ_ALIGN:#x}-aligned (gpa, host address or length)"
                )
            }
            Self::Empty => write!(f, "no guest RAM registered"),
            Self::Overflow => write!(f, "guest RAM size overflows"),
        }
    }
}

impl std::error::Error for PlanError {}

/// ★ **How a layout is carried by objects**, before any object exists: the numbering ([`RamPlan::ram_offset`])
/// and the VMM address ranges one object each must describe ([`RamPlan::segments`]).
///
/// - RAM offsets number the bytes of guest RAM in guest-physical order, so blocks that are adjacent
///   in guest-physical space are adjacent in RAM offsets, and a hole costs nothing.
/// - A segment is ONE layout block: one object describes one VMM mapping (a VMM registers each
///   mapping separately, and only the VMM can vouch that two mappings stay put together). Two blocks
///   that happen to be adjacent in the VMM's address space are still two objects; a row across them
///   is cut there, which costs one more host map and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RamPlan {
    layout: RamLayout,
    segs: Vec<PlanSegment>,
    /// Maximal runs of guest-physical-adjacent blocks: `(gpa, ram_off, len)`.
    runs: Vec<(u64, u64, u64)>,
}

impl RamPlan {
    /// Plan `layout`.
    ///
    /// # Errors
    /// [`PlanError`]: an empty layout, an unaligned block, an overflowing total.
    pub fn new(layout: &RamLayout) -> Result<Self, PlanError> {
        if layout.blocks().is_empty() {
            return Err(PlanError::Empty);
        }
        let mut segs: Vec<PlanSegment> = Vec::new();
        let mut runs: Vec<(u64, u64, u64)> = Vec::new();
        let mut ram_off = 0u64;
        for (index, b) in layout.blocks().iter().enumerate() {
            if ![b.gpa, b.hva, b.len]
                .iter()
                .all(|v| v.is_multiple_of(RAM_OBJ_ALIGN))
            {
                return Err(PlanError::Unaligned { index });
            }
            segs.push(PlanSegment {
                ram_off,
                hva: b.hva,
                len: b.len,
            });
            match runs.last_mut() {
                Some((g, _, l)) if g.checked_add(*l) == Some(b.gpa) => {
                    *l = l.checked_add(b.len).ok_or(PlanError::Overflow)?;
                }
                _ => runs.push((b.gpa, ram_off, b.len)),
            }
            ram_off = ram_off.checked_add(b.len).ok_or(PlanError::Overflow)?;
        }
        Ok(Self {
            layout: layout.clone(),
            segs,
            runs,
        })
    }

    /// The layout this was planned from (to detect a changed topology).
    #[must_use]
    pub fn layout(&self) -> &RamLayout {
        &self.layout
    }

    /// The host segments, one object each, in RAM-offset order.
    #[must_use]
    pub fn segments(&self) -> &[PlanSegment] {
        &self.segs
    }

    /// Bytes of guest RAM.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.layout.total_bytes()
    }

    /// The RAM offset of guest-physical `[gpa, gpa + len)`, when all of it is RAM: wholly inside
    /// one run of adjacent blocks. `None` for a range with a hole, MMIO or the end of RAM in it,
    /// for `len == 0` and for an overflowing range — the leaf is then refused by name.
    #[must_use]
    pub fn ram_offset(&self, gpa: u64, len: u64) -> Option<u64> {
        let end = gpa.checked_add(len).filter(|_| len != 0)?;
        let i = self
            .runs
            .partition_point(|(g, _, l)| g.saturating_add(*l) <= gpa);
        let (g, off, l) = self.runs.get(i).copied()?;
        (g <= gpa && end <= g.saturating_add(l)).then(|| off.saturating_add(gpa.saturating_sub(g)))
    }
}

#[cfg(test)]
mod tests;
