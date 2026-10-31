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

use crate::guestmem::{
    GuestMemory, LAYOUT_CHANGED, LayoutEpoch, LayoutGuard, MAX_LAYOUT_BLOCKS, RamLayout,
};

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

/// ★ The table the host map reads, shareable and replaceable: a [`RamObjects`] behind a lock, so a
/// layout change swaps the table under every space that maps from it (`HostVas` holds a `&` to the
/// cell, not a copy of the table). Reads take the read lock for one lookup; a poisoned lock reads
/// as an EMPTY table (every guest-RAM row is then refused by name, never guessed).
#[derive(Debug, Default)]
pub struct RamObjectCell {
    table: std::sync::RwLock<RamObjects>,
}

impl RamObjectCell {
    /// An empty cell.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the table.
    pub fn set(&self, t: RamObjects) {
        if let Ok(mut w) = self.table.write() {
            *w = t;
        }
    }

    /// A copy of the table now.
    #[must_use]
    pub fn snapshot(&self) -> RamObjects {
        self.table.read().map(|t| t.clone()).unwrap_or_default()
    }

    /// [`RamObjects::resolve`] on the current table.
    ///
    /// # Errors
    /// As [`RamObjects::resolve`]; a poisoned lock is [`RamObjError::Uncovered`].
    pub fn resolve(&self, off: u64, len: u64) -> Result<(u32, u64), RamObjError> {
        self.table
            .read()
            .map_err(|_| RamObjError::Uncovered { at: off })?
            .resolve(off, len)
    }

    /// [`RamObjects::cuts`] on the current table.
    #[must_use]
    pub fn cuts(&self, off: u64, len: u64) -> Vec<u64> {
        self.table
            .read()
            .map_or_else(|_| Vec::new(), |t| t.cuts(off, len))
    }
}

/// One block of a [`RamPlan`]: guest-physical `[gpa, gpa + len)` is numbered from `ram_off` and
/// carried by ONE host object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanSegment {
    /// Index of the block in the layout.
    pub block: usize,
    /// Guest-physical base.
    pub gpa: u64,
    /// First RAM offset (the NUMBER of the block's first byte).
    pub ram_off: u64,
    /// Bytes.
    pub len: u64,
}

/// Why a layout cannot be planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    /// Block `index`: gpa, number base or length is not [`RAM_OBJ_ALIGN`]-aligned.
    Unaligned {
        /// Index in the layout.
        index: usize,
    },
    /// The layout has no block.
    Empty,
    /// The total size overflows `u64`.
    Overflow,
    /// Block `index`: its number base overlaps or precedes the previous block's range, or the
    /// numbering gave fewer bases than there are blocks.
    Numbering {
        /// Index in the layout.
        index: usize,
    },
}

impl core::fmt::Display for PlanError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unaligned { index } => {
                write!(
                    f,
                    "RAM block {index}: not {RAM_OBJ_ALIGN:#x}-aligned (gpa, number base or length)"
                )
            }
            Self::Empty => write!(f, "no guest RAM registered"),
            Self::Overflow => write!(f, "guest RAM size overflows"),
            Self::Numbering { index } => write!(
                f,
                "RAM block {index}: its row numbers overlap or precede the previous block's"
            ),
        }
    }
}

impl std::error::Error for PlanError {}

/// ★ **How a layout is numbered and carried by objects**, before any object exists: the numbering
/// ([`RamPlan::ram_offset`]) and the block each object must describe ([`RamPlan::segments`]).
///
/// - The **number** of a guest-RAM byte is its block's base number plus its offset in the block.
///   The base is chosen by the VMM adapter and MUST NOT depend on the other blocks, so adding or
///   removing a block never renumbers the rest: [`RamPlan::by_gpa`] numbers a byte by its own
///   guest-physical address (the default; a hole costs nothing); a VMM whose RAM is a file may use
///   file offsets instead, so that numbers also name file bytes.
/// - A segment is ONE layout block: one object describes one VMM mapping (a VMM registers each
///   mapping separately, and only the VMM can vouch that two mappings stay put together). Two blocks
///   that happen to be adjacent in the VMM's address space are still two objects; a row across them
///   is cut there, which costs one more host map and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RamPlan {
    layout: RamLayout,
    segs: Vec<PlanSegment>,
    /// Maximal runs of guest-physical-adjacent AND number-adjacent blocks: `(gpa, ram_off, len)`.
    runs: Vec<(u64, u64, u64)>,
}

impl RamPlan {
    /// Plan `layout`, block `i` numbered from `bases[i]`.
    ///
    /// # Errors
    /// [`PlanError`]: an empty layout, an unaligned block, an overflowing total, numbers that
    /// overlap or run backwards.
    pub fn new(layout: &RamLayout, bases: &[u64]) -> Result<Self, PlanError> {
        if layout.blocks().is_empty() {
            return Err(PlanError::Empty);
        }
        if bases.len() != layout.blocks().len() {
            return Err(PlanError::Numbering {
                index: bases.len().min(layout.blocks().len()),
            });
        }
        let mut segs: Vec<PlanSegment> = Vec::new();
        let mut runs: Vec<(u64, u64, u64)> = Vec::new();
        let mut prev_end: Option<u64> = None;
        for (index, (b, &base)) in layout.blocks().iter().zip(bases).enumerate() {
            if ![b.gpa, base, b.len]
                .iter()
                .all(|v| v.is_multiple_of(RAM_OBJ_ALIGN))
            {
                return Err(PlanError::Unaligned { index });
            }
            let end = base.checked_add(b.len).ok_or(PlanError::Overflow)?;
            if prev_end.is_some_and(|e| base < e) {
                return Err(PlanError::Numbering { index });
            }
            segs.push(PlanSegment {
                block: index,
                gpa: b.gpa,
                ram_off: base,
                len: b.len,
            });
            match runs.last_mut() {
                Some((g, o, l))
                    if g.checked_add(*l) == Some(b.gpa) && o.checked_add(*l) == Some(base) =>
                {
                    *l = l.checked_add(b.len).ok_or(PlanError::Overflow)?;
                }
                _ => runs.push((b.gpa, base, b.len)),
            }
            prev_end = Some(end);
        }
        Ok(Self {
            layout: layout.clone(),
            segs,
            runs,
        })
    }

    /// Plan `layout` with every byte numbered by its own guest-physical address (stable under any
    /// add or remove of another block).
    ///
    /// # Errors
    /// As [`RamPlan::new`].
    pub fn by_gpa(layout: &RamLayout) -> Result<Self, PlanError> {
        let bases: Vec<u64> = layout.blocks().iter().map(|b| b.gpa).collect();
        Self::new(layout, &bases)
    }

    /// Plan `layout` with the blocks numbered one after another (a dense numbering; renumbers the
    /// blocks after one that is removed — for tests and fixed layouts only).
    ///
    /// # Errors
    /// As [`RamPlan::new`].
    pub fn cumulative(layout: &RamLayout) -> Result<Self, PlanError> {
        let mut next = 0u64;
        let mut bases = Vec::new();
        for b in layout.blocks() {
            bases.push(next);
            next = next.checked_add(b.len).ok_or(PlanError::Overflow)?;
        }
        Self::new(layout, &bases)
    }

    /// The layout this was planned from (to detect a changed topology).
    #[must_use]
    pub fn layout(&self) -> &RamLayout {
        &self.layout
    }

    /// The blocks, one object each, in guest-physical order.
    #[must_use]
    pub fn segments(&self) -> &[PlanSegment] {
        &self.segs
    }

    /// Bytes of guest RAM.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.layout.total_bytes()
    }

    /// The number of guest-physical `[gpa, gpa + len)`, when all of it is RAM: wholly inside one
    /// run of adjacent blocks whose numbers are also adjacent. `None` for a range with a hole,
    /// MMIO or the end of RAM in it, for `len == 0` and for an overflowing range — the leaf is
    /// then refused by name.
    #[must_use]
    pub fn ram_offset(&self, gpa: u64, len: u64) -> Option<u64> {
        let end = gpa.checked_add(len).filter(|_| len != 0)?;
        let i = self
            .runs
            .partition_point(|(g, _, l)| g.saturating_add(*l) <= gpa);
        let (g, off, l) = self.runs.get(i).copied()?;
        (g <= gpa && end <= g.saturating_add(l)).then(|| off.saturating_add(gpa.saturating_sub(g)))
    }

    /// The guest-physical address of RAM number `off`, and the bytes left in its block: the inverse
    /// of [`RamPlan::ram_offset`] for one byte. `None` when no block carries `off`.
    #[must_use]
    pub fn gpa_of(&self, off: u64) -> Option<(u64, u64)> {
        let i = self
            .segs
            .partition_point(|s| s.ram_off.saturating_add(s.len) <= off);
        let s = self.segs.get(i).filter(|s| s.ram_off <= off)?;
        let into = off.saturating_sub(s.ram_off);
        Some((s.gpa.saturating_add(into), s.len.saturating_sub(into)))
    }
}

// ---------------------------------------------------------------------------------------------
// The book of host objects, kept in step with the VMM's layout
// ---------------------------------------------------------------------------------------------

/// The host verbs the [`RamObjectBook`] needs: one OS descriptor per RAM block, and its release.
pub trait RamObjectHost {
    /// Make the host object over ALL of `segment` (the pages are pinned until [`RamObjectHost::free`]).
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn alloc(&mut self, segment: &kf_linux_raw::RamSegment<'_>) -> Result<u32, String>;
    /// Release an object (and with it the pin).
    fn free(&mut self, obj: u32);
}

/// One block that has a host object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookBlock {
    /// Guest-physical base.
    pub gpa: u64,
    /// Bytes.
    pub len: u64,
    /// First RAM number.
    pub ram_off: u64,
    /// The host object.
    pub obj: u32,
}

/// A block whose object is about to be freed: every GPU-VA slice backed by the numbers
/// `[ram_off, ram_off + len)` must be unmapped BEFORE the object is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retiring {
    /// Guest-physical base of the block that went.
    pub gpa: u64,
    /// First RAM number of the block.
    pub ram_off: u64,
    /// Bytes.
    pub len: u64,
    /// Its host object.
    pub obj: u32,
}

/// Why [`RamObjectBook::sync`] did not bring the objects in line with the layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    /// The VMM has no usable layout, or its numbering is not a plan.
    Layout(String),
    /// The host refused an object (the objects made by this call were freed).
    Host(String),
    /// Unmapping the GPU-VA slices of a removed block failed; its object is still held.
    Unmap(String),
    /// The layout changed under the call ([`LAYOUT_CHANGED`]): the objects made by this call were
    /// freed, nothing was committed.
    LayoutChanged,
}

impl core::fmt::Display for SyncError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Layout(e) => write!(f, "guest RAM layout: {e}"),
            Self::Host(e) => write!(f, "guest-RAM object: {e}"),
            Self::Unmap(e) => write!(f, "unmapping a removed RAM block's GPU slices: {e}"),
            Self::LayoutChanged => f.write_str(LAYOUT_CHANGED),
        }
    }
}

impl std::error::Error for SyncError {}

/// What one [`RamObjectBook::sync`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Blocks whose object was kept as it was.
    pub kept: usize,
    /// Blocks that got a new object.
    pub added: usize,
    /// Blocks whose object was unmapped and freed.
    pub removed: usize,
}

/// ★★ **The host objects of guest RAM, one per block, kept in step with the VMM's layout.**
///
/// [`RamObjectBook::sync`] takes a [`LayoutGuard`] (the VMM's blocks stay alive while it is held),
/// compares the layout with the blocks that have an object and
///
/// 1. for each block that is gone (or changed length — a resize), calls `unmap` so the caller takes
///    down every GPU-VA slice backed by its numbers, THEN frees its object;
/// 2. for each new block, translates the whole block under the same guard and makes ONE object over
///    it, re-checking the epoch after every host call: if the layout moved, the object just made
///    and every object made by this call are freed and the call is refused by name
///    ([`SyncError::LayoutChanged`]) — never retried silently;
/// 3. only then commits the new table and epoch.
///
/// Objects of blocks that did not change are untouched (their numbers are stable: see
/// [`RamPlan`]).
#[derive(Debug, Default)]
pub struct RamObjectBook {
    epoch: Option<LayoutEpoch>,
    blocks: Vec<BookBlock>,
}

impl RamObjectBook {
    /// An empty book (no object yet).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The epoch the book was last brought in line with.
    #[must_use]
    pub fn epoch(&self) -> Option<LayoutEpoch> {
        self.epoch
    }

    /// The blocks that have an object.
    #[must_use]
    pub fn blocks(&self) -> &[BookBlock] {
        &self.blocks
    }

    /// Whether the book describes the VMM's CURRENT layout.
    #[must_use]
    pub fn is_current<G: GuestMemory>(&self, vmm: &G) -> bool {
        self.epoch == Some(vmm.epoch())
    }

    /// The objects as a table for the host map ([`RamObjects`]).
    ///
    /// # Errors
    /// The table refuses overlapping or unaligned segments, by name.
    pub fn objects(&self) -> Result<RamObjects, RamObjError> {
        let segs: Vec<RamSegment> = self
            .blocks
            .iter()
            .map(|b| RamSegment {
                ram_off: b.ram_off,
                len: b.len,
                obj: b.obj,
                obj_off: 0,
            })
            .collect();
        RamObjects::new(&segs)
    }

    /// Bring the objects in line with the VMM's current layout (see the type docs).
    ///
    /// `number` gives the base number of a block (default for a VMM with no file: its `gpa`).
    /// `unmap` takes down every GPU-VA slice backed by a block that is going away.
    ///
    /// # Errors
    /// [`SyncError`]. On `Err` the objects the call made are freed and the epoch is NOT advanced,
    /// so the next call retries; blocks already retired stay retired.
    pub fn sync<G: GuestMemory>(
        &mut self,
        vmm: &G,
        number: &dyn Fn(&crate::guestmem::RamBlockSpec) -> Option<u64>,
        host: &mut dyn RamObjectHost,
        unmap: &mut dyn FnMut(&Retiring) -> Result<(), String>,
    ) -> Result<SyncReport, SyncError> {
        let guard = vmm
            .lock_layout()
            .map_err(|e| SyncError::Layout(e.to_string()))?;
        let epoch = guard.epoch();
        if self.epoch == Some(epoch) {
            return Ok(SyncReport {
                kept: self.blocks.len(),
                ..SyncReport::default()
            });
        }
        let layout = guard.layout().clone();
        let mut bases = Vec::with_capacity(layout.blocks().len());
        for (index, b) in layout.blocks().iter().enumerate() {
            bases.push(number(b).ok_or_else(|| {
                SyncError::Layout(format!(
                    "RAM block {index} ({:#x}+{:#x}): no number",
                    b.gpa, b.len
                ))
            })?);
        }
        let plan = RamPlan::new(&layout, &bases).map_err(|e| SyncError::Layout(e.to_string()))?;
        let want: Vec<PlanSegment> = plan.segments().to_vec();
        let same = |b: &BookBlock, w: &PlanSegment| {
            b.gpa == w.gpa && b.len == w.len && b.ram_off == w.ram_off
        };
        let mut report = SyncReport::default();

        // 1. Retire the blocks that went: unmap their GPU slices first, then free the object.
        let mut keep: Vec<BookBlock> = Vec::new();
        let old = std::mem::take(&mut self.blocks);
        let mut it = old.into_iter();
        while let Some(b) = it.next() {
            if want.iter().any(|w| same(&b, w)) {
                keep.push(b);
                continue;
            }
            let r = Retiring {
                gpa: b.gpa,
                ram_off: b.ram_off,
                len: b.len,
                obj: b.obj,
            };
            if let Err(e) = unmap(&r) {
                // Still held, still listed: the next sync retires it again.
                keep.push(b);
                keep.extend(it);
                self.blocks = keep;
                return Err(SyncError::Unmap(e));
            }
            host.free(b.obj);
            report.removed = report.removed.saturating_add(1);
        }

        // 2. New blocks: one object each, over the whole block, epoch re-checked after each call.
        let mut made: Vec<BookBlock> = Vec::new();
        for w in &want {
            if keep.iter().any(|b| same(b, w)) {
                report.kept = report.kept.saturating_add(1);
                continue;
            }
            let outcome = guard
                .translate(w.gpa, w.len)
                .map_err(|e| SyncError::Layout(e.to_string()))
                .and_then(|t| match t.segments() {
                    [seg] => host.alloc(seg).map_err(SyncError::Host),
                    _ => Err(SyncError::Layout(format!(
                        "RAM block at {:#x}+{:#x} is not one segment",
                        w.gpa, w.len
                    ))),
                });
            let result = match outcome {
                Ok(obj) if guard.still_current() => Ok(obj),
                Ok(obj) => {
                    host.free(obj);
                    Err(SyncError::LayoutChanged)
                }
                Err(e) => Err(e),
            };
            match result {
                Ok(obj) => made.push(BookBlock {
                    gpa: w.gpa,
                    len: w.len,
                    ram_off: w.ram_off,
                    obj,
                }),
                Err(e) => {
                    for m in &made {
                        host.free(m.obj);
                    }
                    self.blocks = keep;
                    return Err(e);
                }
            }
            report.added = report.added.saturating_add(1);
        }
        if !guard.still_current() {
            for m in &made {
                host.free(m.obj);
            }
            self.blocks = keep;
            return Err(SyncError::LayoutChanged);
        }

        // 3. Commit, in layout order.
        let mut all: Vec<BookBlock> = keep;
        all.extend(made);
        all.sort_by_key(|b| b.ram_off);
        self.blocks = all;
        self.epoch = Some(epoch);
        Ok(report)
    }
}

#[cfg(test)]
mod tests;
