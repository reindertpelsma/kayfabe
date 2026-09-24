//! ★★★★★ **THE C↔RUST SEAM — the walk kernel's report, parsed.**
//!
//! `cuda/walk/kf_walk.h` declares three structs the CUDA kernel writes into a buffer that
//! crosses to the host each refresh. **Until this module existed there was no Rust reader for
//! them at all** — the differential oracle
//! (`crates/kayfabe-mmu/tests/walk_kernel_differential.rs`) compares through a *corpus file*
//! written by a separate host program, so the report ABI itself was carried by nothing but two
//! struct declarations in two languages that never met.
//!
//! # ⊘⊘⊘ WHY A ROUND TRIP AGAINST ITSELF IS NOT ENOUGH
//!
//! `[w725]` The class this module exists to catch is the one that bit `NVOS34`
//! (`kayfabe-abi/src/submit.rs`): a field at **+12 instead of +16** still encodes, still
//! decodes, and still round-trips **against itself** — every test green, and the bytes wrong.
//! Only **pinned byte offsets** catch it, and only a comparison against the **C compiler's own
//! `offsetof`** proves the pins describe the struct the kernel actually writes.
//!
//! ⇒ Two things are therefore tested, and they are different tests:
//!
//! - [`tests`] here pins every field's byte range and asserts the reserved words stay zero;
//! - `crates/kayfabe-mmu/tests/walk_report_seam.rs` compiles `cuda/walk/kf_report_emit.c` with
//!   the **real** `kf_walk.h`, runs it, and compares this module's pins against that compiler's
//!   `offsetof`/`sizeof` — then parses the bytes it emitted and checks every field against the
//!   emitter's own independently-printed account of what it wrote.
//!
//! # What this module deliberately does NOT do
//!
//! It has **no format-version knowledge** — no VER2 bit positions, no page-table geometry. That
//! is the property `the_walk_kernel_report_format.md` asks for (*"the format-version knowledge
//! stays in the kernel and does not leak into the host's parser"*), and the report is
//! self-describing about page sizes via [`ReportHeader::ps_log2`] precisely so that it can hold.
//!
//! ⊘ It also does not *trust* the report. Every count is checked against its capacity and
//! against the actual buffer length before a single element is read, and
//! [`Report::validate`] re-states property 3 of the format doc on this side of the link.

use crate::walkdiff::{PageClass, Run};

// ── kf_walk.h: the magic and the version ────────────────────────────────────────────────────

/// `'K','F','W','R'` as little-endian bytes — `KFWR_MAGIC`.
pub const MAGIC: u32 = 0x5257_464B;
/// `KFWR_VERSION`. A report carrying anything else is refused rather than guessed at.
pub const VERSION: u16 = 1;

// ── kf_walk.h: ReportHeader::flags ──────────────────────────────────────────────────────────

/// `KFWR_HF_TRUNCATED` — run/pdb capacity or walk budget hit.
pub const HF_TRUNCATED: u16 = 1 << 0;
/// `KFWR_HF_RESYNC` — every run is a `MAP`; this is not a delta.
pub const HF_RESYNC: u16 = 1 << 1;
/// `KFWR_HF_REFUSED` — at least one refusal; see [`ReportHeader::refuse_mask`].
pub const HF_REFUSED: u16 = 1 << 2;
/// `KFWR_HF_BUDGET` — the entry budget stopped a walk.
pub const HF_BUDGET: u16 = 1 << 3;
/// `KFWR_HF_SCOPED` — a scope hint restricted the walk.
pub const HF_SCOPED: u16 = 1 << 4;
/// `KFWR_HF_PDB_TRUNCATED` — more address spaces than `pdb_capacity`.
pub const HF_PDB_TRUNCATED: u16 = 1 << 5;
/// `KFWR_HF_SCOPE_DEGRADED` — a hint was unusable, so the walk was full.
pub const HF_SCOPE_DEGRADED: u16 = 1 << 6;

// ── kf_walk.h: ReportHeader::refuse_mask ────────────────────────────────────────────────────

/// `KFWR_R_OOB` — a table would leave the GPGA buffer.
pub const R_OOB: u32 = 1 << 0;
/// `KFWR_R_UNALIGNED` — a table pointer is not naturally aligned.
pub const R_UNALIGNED: u32 = 1 << 1;
/// `KFWR_R_FOREIGN_AP` — a table page is not in vidmem.
pub const R_FOREIGN_AP: u32 = 1 << 2;
/// `KFWR_R_TOO_DEEP` — ⊘ structurally unreachable; the kernel's suite asserts it is never set.
pub const R_TOO_DEEP: u32 = 1 << 3;
/// `KFWR_R_RUN_CAP` — out of run slots.
pub const R_RUN_CAP: u32 = 1 << 4;
/// `KFWR_R_BUDGET` — out of entry budget.
pub const R_BUDGET: u32 = 1 << 5;
/// `KFWR_R_PDB_CAP` — out of `PdbEntry` slots.
pub const R_PDB_CAP: u32 = 1 << 6;
/// `KFWR_R_BAD_SCOPE` — a hint we could not use.
pub const R_BAD_SCOPE: u32 = 1 << 7;
/// `KFWR_R_PDB_UNSORTED` — the caller's pdb list was not ascending.
pub const R_PDB_UNSORTED: u32 = 1 << 8;
/// `KFWR_R_DELTA_CAP` — the delta itself overflowed its slice.
pub const R_DELTA_CAP: u32 = 1 << 9;
/// `KFWR_R_MISALIGNED_LEAF` — a leaf whose target is not aligned to its own page size.
pub const R_MISALIGNED_LEAF: u32 = 1 << 10;
/// `KFWR_R_BAD_FORMAT` — the host's format descriptor was refused at launch.
pub const R_BAD_FORMAT: u32 = 1 << 11;

// ── kf_walk.h: PdbEntry::vas_flags ──────────────────────────────────────────────────────────

/// `KFWR_V_NEW` — this address space was not in the previous report.
pub const V_NEW: u32 = 1 << 0;
/// `KFWR_V_GONE` — this address space is gone. ⊘ Carries no runs, by design.
pub const V_GONE: u32 = 1 << 1;
/// `KFWR_V_RESYNC` — this address space's runs are its full state, not a delta.
pub const V_RESYNC: u32 = 1 << 2;

// ── kf_walk.h: MapRun::flags ────────────────────────────────────────────────────────────────

/// Shift of the aperture field within [`MapRun::flags`].
pub const RF_AP_SHIFT: u32 = 0;
/// Mask of the aperture field, once shifted down.
pub const RF_AP_MASK: u32 = 0x7;
/// `KFWR_RF_READ_ONLY`.
pub const RF_READ_ONLY: u32 = 1 << 3;
/// `KFWR_RF_ATOMIC_DISABLE`.
pub const RF_ATOMIC_DISABLE: u32 = 1 << 4;
/// `KFWR_RF_VOLATILE`.
pub const RF_VOLATILE: u32 = 1 << 5;
/// `KFWR_RF_PRIVILEGE`.
pub const RF_PRIVILEGE: u32 = 1 << 6;
/// Shift of the page-size code within [`MapRun::flags`].
pub const RF_PS_SHIFT: u32 = 8;
/// Mask of the page-size code, once shifted down.
pub const RF_PS_MASK: u32 = 0xF;
/// Shift of the PTE `KIND` within [`MapRun::flags`].
///
/// ★ `KIND` is part of **run identity**: a run is one published mapping and one
/// mapping carries one kind, so the walk never coalesces across a change of it
/// (`the_walk_kernel_report_format.md` §w725b). It rides in spare bits of the
/// existing flags word, so the report's byte layout is unchanged.
pub const RF_KIND_SHIFT: u32 = 16;
/// Mask of the `KIND` field, once shifted down.
pub const RF_KIND_MASK: u32 = 0xFF;

/// Which memory the run's target lives in — `KFWR_AP_*`, decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunAperture {
    /// `KFWR_AP_VIDMEM`.
    Vidmem,
    /// `KFWR_AP_PEER`.
    Peer,
    /// `KFWR_AP_SYSCOH`.
    SysmemCoherent,
    /// `KFWR_AP_SYSNONCOH`.
    SysmemNonCoherent,
}

/// What the host should do with a run — `KFWR_OP_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOp {
    /// `KFWR_OP_MAP` — a range the guest now has and did not before.
    Map,
    /// `KFWR_OP_UNMAP` — had, and no longer. ⊘ `gpga` is meaningless.
    Unmap,
    /// `KFWR_OP_REMAP` — same VA range, different target or flags.
    Remap,
}

impl RunOp {
    /// The wire value, so a test can pin it rather than trust the discriminant order.
    #[must_use]
    pub fn code(self) -> u16 {
        match self {
            RunOp::Map => 1,
            RunOp::Unmap => 2,
            RunOp::Remap => 3,
        }
    }
}

/// Why a report was refused. ⊘ Every variant names **which** check failed: a parser that says
/// only *"malformed"* has told the caller nothing it can act on, and the whole point of
/// [`ReportHeader::refuse_mask`] on the kernel's side is the same argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// The buffer is shorter than the struct being read at `offset`, which wanted `want` bytes.
    Short {
        /// Byte offset the read started at.
        offset: usize,
        /// Bytes the read needed.
        want: usize,
        /// Bytes the buffer actually had from `offset`.
        have: usize,
    },
    /// `magic` is not [`MAGIC`].
    BadMagic(u32),
    /// `version` is not [`VERSION`].
    BadVersion(u16),
    /// A count exceeds its own declared capacity.
    CountExceedsCapacity {
        /// `"pdb"` or `"run"`.
        which: &'static str,
        /// The declared count.
        count: u32,
        /// The declared capacity.
        capacity: u32,
    },
    /// A `PdbEntry`'s run slice leaves the run array — the format doc's property 3.
    PdbSliceOutOfRange {
        /// Index of the offending `PdbEntry`.
        index: usize,
        /// Its `first_run`.
        first_run: u32,
        /// Its `run_count`.
        run_count: u32,
        /// The header's `run_count`.
        total: u32,
    },
    /// A `MapRun::op` is not one of `KFWR_OP_*`.
    BadOp {
        /// Index of the offending run.
        index: usize,
        /// The value found.
        op: u16,
    },
    /// A `MapRun::pdb_index` names no `PdbEntry`.
    BadPdbIndex {
        /// Index of the offending run.
        index: usize,
        /// The value found.
        pdb_index: u16,
        /// The header's `pdb_count`.
        pdb_count: u32,
    },
    /// A run's `len` is zero, or is not a multiple of its own page size.
    BadRunLength {
        /// Index of the offending run.
        index: usize,
        /// Its `len`.
        len: u64,
        /// The page size its flags declare.
        page_size: u64,
    },
    /// ★★★★★ §39(c): a run names memory OUTSIDE the guest's own GPGA span. Not a
    /// malformation — the report is perfectly well formed — but acting on it would map memory
    /// that is not the guest's, which is the escalation the walker exists to refuse.
    RunOutsideGpga {
        /// Index of the offending run.
        index: usize,
        /// The GPGA it named.
        gpga: u64,
        /// Its length.
        len: u64,
        /// The span it had to lie inside.
        span: u64,
    },
    /// A run's `va` is not aligned to its own page size.
    UnalignedRunVa {
        /// Index of the offending run.
        index: usize,
        /// Its `va`.
        va: u64,
        /// The page size its flags declare.
        page_size: u64,
    },
    /// The page-size code in a run's flags has no entry in `ps_log2`, or that entry is absurd.
    BadPageSize {
        /// Index of the offending run.
        index: usize,
        /// The 4-bit code from the run's flags.
        code: u32,
        /// `ps_log2[code]`, if the code was in range.
        log2: Option<u8>,
    },
}

/// The report's fixed 64-byte head. Field order and every byte offset are pinned by
/// [`tests::the_header_fields_sit_where_the_kernel_puts_them`] and cross-checked against the C
/// compiler's `offsetof` by the seam test.
///
/// ⚠ **`kf_walk.h`'s own comment is stale here** and this doc is the accurate one: the header
/// reaches 64 bytes with `sparse_slots` and `ps_log2[4]`, **not** with the `pad` the comment
/// still names. There is no padding anywhere in this struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReportHeader {
    /// `KFWR_MAGIC`.
    pub magic: u32,
    /// Format version; an unknown one is refused, never guessed at.
    pub version: u16,
    /// `KFWR_HF_*`.
    pub flags: u16,
    /// +1 per completed walk.
    pub generation: u64,
    /// What the kernel believes the host last consumed.
    pub acked_generation: u64,
    /// `PdbEntry`s present.
    pub pdb_count: u32,
    /// `PdbEntry` slots available.
    pub pdb_capacity: u32,
    /// `MapRun`s present.
    pub run_count: u32,
    /// `MapRun` slots available. ⊘ Truncation is detectable from the buffer alone.
    pub run_capacity: u32,
    /// Walk census — cost, not correctness.
    pub entries_visited: u64,
    /// How many refusals fired.
    pub refusals: u32,
    /// `KFWR_R_*` — **which** refusals.
    pub refuse_mask: u32,
    /// Slots the guest declared empty, as opposed to never having written.
    pub sparse_slots: u32,
    /// Page-size code → `log2(bytes)`. ★ This is what lets this parser hold no format knowledge.
    pub ps_log2: [u8; 4],
}

impl ReportHeader {
    /// Bytes on the wire. An ABI fact: the kernel writes exactly this many.
    pub const SIZE: usize = 64;

    /// Read a header from the front of `b`.
    ///
    /// # Errors
    /// [`ParseError::Short`] if `b` is under [`Self::SIZE`]; [`ParseError::BadMagic`] or
    /// [`ParseError::BadVersion`] if the head does not identify a report this code can read.
    pub fn decode(b: &[u8]) -> Result<Self, ParseError> {
        let h = Self {
            magic: u32_at(b, 0)?,
            version: u16_at(b, 4)?,
            flags: u16_at(b, 6)?,
            generation: u64_at(b, 8)?,
            acked_generation: u64_at(b, 16)?,
            pdb_count: u32_at(b, 24)?,
            pdb_capacity: u32_at(b, 28)?,
            run_count: u32_at(b, 32)?,
            run_capacity: u32_at(b, 36)?,
            entries_visited: u64_at(b, 40)?,
            refusals: u32_at(b, 48)?,
            refuse_mask: u32_at(b, 52)?,
            sparse_slots: u32_at(b, 56)?,
            ps_log2: [
                byte_at(b, 60)?,
                byte_at(b, 61)?,
                byte_at(b, 62)?,
                byte_at(b, 63)?,
            ],
        };
        if h.magic != MAGIC {
            return Err(ParseError::BadMagic(h.magic));
        }
        if h.version != VERSION {
            return Err(ParseError::BadVersion(h.version));
        }
        Ok(h)
    }

    /// Write this header into the front of `b`, for the layout test and for fixtures.
    ///
    /// # Errors
    /// [`ParseError::Short`] if `b` is under [`Self::SIZE`].
    pub fn encode_into(&self, b: &mut [u8]) -> Result<(), ParseError> {
        need(b.len(), 0, Self::SIZE)?;
        b[0..4].copy_from_slice(&self.magic.to_le_bytes());
        b[4..6].copy_from_slice(&self.version.to_le_bytes());
        b[6..8].copy_from_slice(&self.flags.to_le_bytes());
        b[8..16].copy_from_slice(&self.generation.to_le_bytes());
        b[16..24].copy_from_slice(&self.acked_generation.to_le_bytes());
        b[24..28].copy_from_slice(&self.pdb_count.to_le_bytes());
        b[28..32].copy_from_slice(&self.pdb_capacity.to_le_bytes());
        b[32..36].copy_from_slice(&self.run_count.to_le_bytes());
        b[36..40].copy_from_slice(&self.run_capacity.to_le_bytes());
        b[40..48].copy_from_slice(&self.entries_visited.to_le_bytes());
        b[48..52].copy_from_slice(&self.refusals.to_le_bytes());
        b[52..56].copy_from_slice(&self.refuse_mask.to_le_bytes());
        b[56..60].copy_from_slice(&self.sparse_slots.to_le_bytes());
        b[60..64].copy_from_slice(&self.ps_log2);
        Ok(())
    }

    /// `true` when this report must not be applied as a delta — the format doc's safety
    /// property 1. ⚠ A truncated report is a **full resync trigger**, never a partial apply.
    #[must_use]
    pub fn is_truncated(&self) -> bool {
        self.flags & (HF_TRUNCATED | HF_PDB_TRUNCATED) != 0
    }

    /// `true` when every run is the current state rather than a change.
    #[must_use]
    pub fn is_resync(&self) -> bool {
        self.flags & HF_RESYNC != 0
    }
}

/// One address space's slice of the run array.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PdbEntry {
    /// The page-directory base — the VAS identity.
    pub pdb: u64,
    /// Index into the run array.
    pub first_run: u32,
    /// Runs belonging to this address space.
    pub run_count: u32,
    /// `KFWR_V_*`.
    pub vas_flags: u32,
    /// Must be zero. Pinned so a future field cannot be added without a test noticing.
    pub reserved: u32,
    /// Must be zero.
    pub reserved2: u64,
}

impl PdbEntry {
    /// Bytes on the wire.
    pub const SIZE: usize = 32;

    /// Read the `i`-th `PdbEntry` from an array starting at `base`.
    ///
    /// # Errors
    /// [`ParseError::Short`] if the entry does not fit in `b`.
    pub fn decode_at(b: &[u8], at: usize) -> Result<Self, ParseError> {
        Ok(Self {
            pdb: u64_at(b, at)?,
            first_run: u32_at(b, at + 8)?,
            run_count: u32_at(b, at + 12)?,
            vas_flags: u32_at(b, at + 16)?,
            reserved: u32_at(b, at + 20)?,
            reserved2: u64_at(b, at + 24)?,
        })
    }

    /// Write this entry at `at`.
    ///
    /// # Errors
    /// [`ParseError::Short`] if it does not fit.
    pub fn encode_at(&self, b: &mut [u8], at: usize) -> Result<(), ParseError> {
        need(b.len(), at, Self::SIZE)?;
        b[at..at + 8].copy_from_slice(&self.pdb.to_le_bytes());
        b[at + 8..at + 12].copy_from_slice(&self.first_run.to_le_bytes());
        b[at + 12..at + 16].copy_from_slice(&self.run_count.to_le_bytes());
        b[at + 16..at + 20].copy_from_slice(&self.vas_flags.to_le_bytes());
        b[at + 20..at + 24].copy_from_slice(&self.reserved.to_le_bytes());
        b[at + 24..at + 32].copy_from_slice(&self.reserved2.to_le_bytes());
        Ok(())
    }

    /// `true` when this address space is gone. ⊘ It carries no runs.
    #[must_use]
    pub fn is_gone(&self) -> bool {
        self.vas_flags & V_GONE != 0
    }
}

/// One contiguous mapping change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MapRun {
    /// Start virtual address, page-aligned.
    pub va: u64,
    /// Start guest physical address. ⊘ Meaningless when `op == UNMAP`.
    pub gpga: u64,
    /// Bytes; a multiple of the page size the flags declare.
    pub len: u64,
    /// `KFWR_RF_*` — the **decoded** fields, never the raw entry.
    pub flags: u32,
    /// `KFWR_OP_*`.
    pub op: u16,
    /// Back-reference into the `PdbEntry` array, so a run is self-describing.
    pub pdb_index: u16,
}

impl MapRun {
    /// Bytes on the wire.
    pub const SIZE: usize = 32;

    /// Read a `MapRun` at `at`.
    ///
    /// # Errors
    /// [`ParseError::Short`] if it does not fit in `b`.
    pub fn decode_at(b: &[u8], at: usize) -> Result<Self, ParseError> {
        Ok(Self {
            va: u64_at(b, at)?,
            gpga: u64_at(b, at + 8)?,
            len: u64_at(b, at + 16)?,
            flags: u32_at(b, at + 24)?,
            op: u16_at(b, at + 28)?,
            pdb_index: u16_at(b, at + 30)?,
        })
    }

    /// Write this run at `at`.
    ///
    /// # Errors
    /// [`ParseError::Short`] if it does not fit.
    pub fn encode_at(&self, b: &mut [u8], at: usize) -> Result<(), ParseError> {
        need(b.len(), at, Self::SIZE)?;
        b[at..at + 8].copy_from_slice(&self.va.to_le_bytes());
        b[at + 8..at + 16].copy_from_slice(&self.gpga.to_le_bytes());
        b[at + 16..at + 24].copy_from_slice(&self.len.to_le_bytes());
        b[at + 24..at + 28].copy_from_slice(&self.flags.to_le_bytes());
        b[at + 28..at + 30].copy_from_slice(&self.op.to_le_bytes());
        b[at + 30..at + 32].copy_from_slice(&self.pdb_index.to_le_bytes());
        Ok(())
    }

    /// The op, decoded.
    ///
    /// # Errors
    /// [`ParseError::BadOp`] for a value outside `KFWR_OP_*`.
    pub fn op_decoded(&self, index: usize) -> Result<RunOp, ParseError> {
        match self.op {
            1 => Ok(RunOp::Map),
            2 => Ok(RunOp::Unmap),
            3 => Ok(RunOp::Remap),
            other => Err(ParseError::BadOp { index, op: other }),
        }
    }

    /// The aperture, decoded. ⊘ Total: the field is three bits and all eight values are named
    /// by folding the four unused ones onto the closest thing the ABI defines, so this cannot
    /// fail and callers need no error path for it.
    #[must_use]
    pub fn aperture(&self) -> RunAperture {
        match (self.flags >> RF_AP_SHIFT) & RF_AP_MASK {
            0 => RunAperture::Vidmem,
            1 => RunAperture::Peer,
            2 => RunAperture::SysmemCoherent,
            _ => RunAperture::SysmemNonCoherent,
        }
    }

    /// The raw 4-bit page-size code. Meaningless without the header's `ps_log2`.
    #[must_use]
    pub fn page_size_code(&self) -> u32 {
        (self.flags >> RF_PS_SHIFT) & RF_PS_MASK
    }

    /// This run's page size in bytes, resolved through the header's self-describing table.
    ///
    /// # Errors
    /// [`ParseError::BadPageSize`] if the code is out of range or names an absurd shift.
    pub fn page_size(&self, h: &ReportHeader, index: usize) -> Result<u64, ParseError> {
        let code = self.page_size_code();
        let log2 = usize::try_from(code)
            .ok()
            .and_then(|c| h.ps_log2.get(c).copied())
            .ok_or(ParseError::BadPageSize {
                index,
                code,
                log2: None,
            })?;
        // 12 is 4 KiB, the smallest GPU page; 40 is 1 TiB, far past any real large page. A shift
        // outside that is a corrupt or uninitialised table, not a page size.
        if !(12..=40).contains(&log2) {
            return Err(ParseError::BadPageSize {
                index,
                code,
                log2: Some(log2),
            });
        }
        Ok(1u64 << log2)
    }

    /// `true` if the read-only bit is set.
    #[must_use]
    pub fn read_only(&self) -> bool {
        self.flags & RF_READ_ONLY != 0
    }
    /// `true` if the atomic-disable bit is set.
    #[must_use]
    pub fn atomic_disable(&self) -> bool {
        self.flags & RF_ATOMIC_DISABLE != 0
    }
    /// `true` if the volatile bit is set.
    #[must_use]
    pub fn is_volatile(&self) -> bool {
        self.flags & RF_VOLATILE != 0
    }
    /// `true` if the privilege bit is set.
    #[must_use]
    pub fn privileged(&self) -> bool {
        self.flags & RF_PRIVILEGE != 0
    }
}

/// A parsed report: the header and its two arrays, each already checked against the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The fixed head.
    pub header: ReportHeader,
    /// Exactly `header.pdb_count` entries.
    pub pdbs: Vec<PdbEntry>,
    /// Exactly `header.run_count` runs.
    pub runs: Vec<MapRun>,
}

impl Report {
    /// Byte offset of the `PdbEntry` array in the packed image.
    #[must_use]
    pub fn pdb_array_offset() -> usize {
        ReportHeader::SIZE
    }

    /// Byte offset of the `MapRun` array in the packed image, given the header's `pdb_count`.
    ///
    /// ⊘⊘⊘ **BY `pdb_count`, NOT `pdb_capacity` — and that is a finding, not a detail.**
    /// `kf_refresh` does not hand the host one buffer at all: it does **three** `cudaMemcpy`s,
    /// of `sizeof(header)`, `pdb_count * sizeof(PdbEntry)` and `run_count * sizeof(MapRun)`
    /// (`cuda/walk/kf_walk.cu:1286-1290`), into three separate host arrays. So the "small output
    /// buffer" the format doc describes as one thing **does not exist yet**; what crosses is
    /// three arrays sized by COUNT. This packed image is their concatenation, i.e. exactly the
    /// bytes that cross, and nothing is invented.
    #[must_use]
    pub fn run_array_offset(pdb_count: u32) -> usize {
        ReportHeader::SIZE + (pdb_count as usize) * PdbEntry::SIZE
    }

    /// Total bytes of the packed image a report with these counts occupies.
    #[must_use]
    pub fn packed_len(pdb_count: u32, run_count: u32) -> usize {
        Self::run_array_offset(pdb_count) + (run_count as usize) * MapRun::SIZE
    }

    /// Parse the **three arrays `kf_refresh` actually produces**, each as its own slice.
    ///
    /// This is the shape of the real C API: `kf_refresh(..., KfReportHeader *hdr_out,
    /// KfPdbEntry *pdb_out, KfMapRun *run_out)`. [`Report::parse`] is the packed form of the
    /// same bytes.
    ///
    /// # Errors
    /// Any [`ParseError`]; see [`Report::validate`].
    pub fn parse_parts(hdr: &[u8], pdb: &[u8], run: &[u8]) -> Result<Self, ParseError> {
        let header = ReportHeader::decode(hdr)?;
        Self::from_parts(header, pdb, run)
    }

    /// Parse a report out of one contiguous image laid out as
    /// `header | PdbEntry[pdb_count] | MapRun[run_count]`.
    ///
    /// ⊘ Every count is checked against its capacity **and** against `b.len()` before an element
    /// is read, so a header claiming a million runs over a 4 KiB buffer is a named refusal and
    /// not a panic.
    ///
    /// # Errors
    /// Any [`ParseError`]; see [`Report::validate`] for the structural rules.
    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        let header = ReportHeader::decode(b)?;
        let pdb_at = Self::pdb_array_offset();
        let run_at = Self::run_array_offset(header.pdb_count);
        let pdb = b.get(pdb_at..).unwrap_or(&[]);
        let run = b.get(run_at..).unwrap_or(&[]);
        Self::from_parts(header, pdb, run)
    }

    fn from_parts(header: ReportHeader, pdb: &[u8], run: &[u8]) -> Result<Self, ParseError> {
        if header.pdb_count > header.pdb_capacity {
            return Err(ParseError::CountExceedsCapacity {
                which: "pdb",
                count: header.pdb_count,
                capacity: header.pdb_capacity,
            });
        }
        if header.run_count > header.run_capacity {
            return Err(ParseError::CountExceedsCapacity {
                which: "run",
                count: header.run_count,
                capacity: header.run_capacity,
            });
        }
        let mut pdbs = Vec::with_capacity(header.pdb_count as usize);
        for i in 0..header.pdb_count as usize {
            pdbs.push(PdbEntry::decode_at(pdb, i * PdbEntry::SIZE)?);
        }
        let mut runs = Vec::with_capacity(header.run_count as usize);
        for i in 0..header.run_count as usize {
            runs.push(MapRun::decode_at(run, i * MapRun::SIZE)?);
        }
        let r = Self { header, pdbs, runs };
        r.validate()?;
        Ok(r)
    }

    /// Property 3 of the format doc, restated on this side of the link.
    ///
    /// ⚠ The doc's rule reads *"every `first_run + run_count <= run_count`"*, which is a typo for
    /// `<= header.run_count`; `kf_validate_report` implements the latter and so does this.
    ///
    /// # Errors
    /// The specific [`ParseError`] naming which rule failed and where.
    pub fn validate(&self) -> Result<(), ParseError> {
        for (i, p) in self.pdbs.iter().enumerate() {
            let end = u64::from(p.first_run) + u64::from(p.run_count);
            if end > u64::from(self.header.run_count) {
                return Err(ParseError::PdbSliceOutOfRange {
                    index: i,
                    first_run: p.first_run,
                    run_count: p.run_count,
                    total: self.header.run_count,
                });
            }
        }
        for (i, r) in self.runs.iter().enumerate() {
            r.op_decoded(i)?;
            if u32::from(r.pdb_index) >= self.header.pdb_count {
                return Err(ParseError::BadPdbIndex {
                    index: i,
                    pdb_index: r.pdb_index,
                    pdb_count: self.header.pdb_count,
                });
            }
            let ps = r.page_size(&self.header, i)?;
            if r.len == 0 || r.len % ps != 0 {
                return Err(ParseError::BadRunLength {
                    index: i,
                    len: r.len,
                    page_size: ps,
                });
            }
            if r.va % ps != 0 {
                return Err(ParseError::UnalignedRunVa {
                    index: i,
                    va: r.va,
                    page_size: ps,
                });
            }
            // ⊘ NOTHING is asserted about `gpga` ALIGNMENT, and that is deliberate — see
            // `the_walk_kernel_report_format.md` §3: VER2 carries a 4 KiB-granular address field
            // at every leaf level, so a hostile guest can spell a 512 MiB page whose base is
            // 4 KiB-aligned. The kernel refuses such a leaf by name; the host must not
            // *assume* the alignment it cannot enforce.
        }
        Ok(())
    }

    /// ★★★★★ **§39(c) CONTAINMENT — every mapping run must lie inside the guest's own GPGA.**
    ///
    /// ⊘ Separate from [`Self::validate`] because it is a different KIND of property. Everything
    /// `validate` asserts describes a report that is internally inconsistent — a slice that runs
    /// past its array, a length that is not a multiple of its page size. This asserts that the
    /// report does not ask us to map memory **that is not the guest's**, which is the escalation
    /// the walker exists to refuse and cannot be checked without knowing how big the guest is.
    ///
    /// An `UNMAP` names a VA being retired and carries a meaningless `gpga`, so it is exempt —
    /// the same exemption [`Self::present_runs`] relies on.
    ///
    /// ⚠ This is the layer on the PRODUCTION path. The CUDA kernel refuses such a leaf at both
    /// emit chokepoints and `StoreMapPort::map` bounds again at map time; this is the middle
    /// one, and until w760m it did not exist — so a report that named memory outside the store
    /// was carried all the way into a built diff before anything objected.
    ///
    /// # Errors
    /// [`ParseError::RunOutsideGpga`] naming the first run that leaves `span`.
    pub fn validate_within(&self, span: u64) -> Result<(), ParseError> {
        for (i, r) in self.runs.iter().enumerate() {
            if r.op == RunOp::Unmap.code() {
                continue;
            }
            // ★ w825 — the span bounds VIDMEM runs only (GPGA offsets). A system-memory run
            // names a guest-PHYSICAL address and is bounded by the guest-RAM object and the
            // VMM's layout at map time; PEER is refused (single-GPU guest). Mirrors
            // `kf_emit`'s per-aperture check.
            match r.aperture() {
                RunAperture::SysmemCoherent | RunAperture::SysmemNonCoherent => continue,
                RunAperture::Peer => {
                    return Err(ParseError::RunOutsideGpga {
                        index: i,
                        gpga: r.gpga,
                        len: r.len,
                        span,
                    });
                }
                RunAperture::Vidmem => {}
            }
            if r.gpga > span || r.len > span - r.gpga {
                return Err(ParseError::RunOutsideGpga {
                    index: i,
                    gpga: r.gpga,
                    len: r.len,
                    span,
                });
            }
        }
        Ok(())
    }

    /// The report's runs as [`crate::walkdiff::Run`]s, so the parsed bytes can be fed straight
    /// into the host-side diff.
    ///
    /// Only `MAP` and `REMAP` runs describe present mappings; `UNMAP` runs carry a meaningless
    /// `gpga` and are skipped, which is what makes the result a *mapping set* rather than an
    /// instruction list.
    ///
    /// # Errors
    /// [`ParseError::BadPageSize`] for a page size with no [`PageClass`], or any error
    /// [`MapRun::page_size`] raises.
    pub fn present_runs(&self) -> Result<Vec<Run>, ParseError> {
        let mut out = Vec::new();
        for (i, r) in self.runs.iter().enumerate() {
            if r.op_decoded(i)? == RunOp::Unmap {
                continue;
            }
            let ps = r.page_size(&self.header, i)?;
            let class = match ps {
                4096 => PageClass::P4K,
                65_536 => PageClass::P64K,
                0x0020_0000 => PageClass::P2M,
                0x2000_0000 => PageClass::P512M,
                _ => {
                    return Err(ParseError::BadPageSize {
                        index: i,
                        code: r.page_size_code(),
                        log2: Some(ps.trailing_zeros().try_into().unwrap_or(0)),
                    });
                }
            };
            out.push(Run {
                va: r.va,
                gpga: r.gpga,
                len: r.len,
                flags: r.flags,
                class,
            });
        }
        Ok(out)
    }

    /// The runs belonging to one address space, by its index in [`Report::pdbs`].
    #[must_use]
    pub fn runs_of(&self, pdb_index: usize) -> &[MapRun] {
        let Some(p) = self.pdbs.get(pdb_index) else {
            return &[];
        };
        let lo = p.first_run as usize;
        let hi = lo + p.run_count as usize;
        self.runs.get(lo..hi).unwrap_or(&[])
    }
}

// ── the only three readers, each bounds-checked before it reads ─────────────────────────────

fn need(have_len: usize, offset: usize, want: usize) -> Result<(), ParseError> {
    let have = have_len.saturating_sub(offset);
    if have < want {
        return Err(ParseError::Short { offset, want, have });
    }
    Ok(())
}

fn byte_at(b: &[u8], at: usize) -> Result<u8, ParseError> {
    b.get(at).copied().ok_or(ParseError::Short {
        offset: at,
        want: 1,
        have: 0,
    })
}

fn u16_at(b: &[u8], at: usize) -> Result<u16, ParseError> {
    need(b.len(), at, 2)?;
    let s: [u8; 2] = b[at..at + 2].try_into().unwrap_or([0; 2]);
    Ok(u16::from_le_bytes(s))
}

fn u32_at(b: &[u8], at: usize) -> Result<u32, ParseError> {
    need(b.len(), at, 4)?;
    let s: [u8; 4] = b[at..at + 4].try_into().unwrap_or([0; 4]);
    Ok(u32::from_le_bytes(s))
}

fn u64_at(b: &[u8], at: usize) -> Result<u64, ParseError> {
    need(b.len(), at, 8)?;
    let s: [u8; 8] = b[at..at + 8].try_into().unwrap_or([0; 8]);
    Ok(u64::from_le_bytes(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⊘⊘⊘ **THE BYTE OFFSETS ARE THE WHOLE TEST.**
    ///
    /// A field at the wrong offset still encodes, still decodes, and still round-trips against
    /// itself — the `NVOS34` lesson, in the one struct that crosses from CUDA to Rust. Every
    /// field gets a distinct value so a *swap* of two same-width neighbours is caught too.
    #[test]
    fn the_header_fields_sit_where_the_kernel_puts_them() {
        let h = ReportHeader {
            magic: MAGIC,
            version: 0x1111,
            flags: 0x2222,
            generation: 0x3333_3333_4444_4444,
            acked_generation: 0x5555_5555_6666_6666,
            pdb_count: 0x7777_7777,
            pdb_capacity: 0x0888_8888,
            run_count: 0x0999_9999,
            run_capacity: 0x0AAA_AAAA,
            entries_visited: 0x0BBB_BBBB_CCCC_CCCC,
            refusals: 0x0DDD_DDDD,
            refuse_mask: 0x0EEE_EEEE,
            sparse_slots: 0x0FFF_FFFF,
            ps_log2: [12, 16, 21, 29],
        };
        let mut b = [0u8; ReportHeader::SIZE];
        h.encode_into(&mut b).expect("encode");

        assert_eq!(&b[0..4], &MAGIC.to_le_bytes(), "magic @ +0");
        assert_eq!(&b[4..6], &0x1111u16.to_le_bytes(), "version @ +4");
        assert_eq!(&b[6..8], &0x2222u16.to_le_bytes(), "flags @ +6");
        assert_eq!(
            &b[8..16],
            &0x3333_3333_4444_4444u64.to_le_bytes(),
            "generation @ +8"
        );
        assert_eq!(
            &b[16..24],
            &0x5555_5555_6666_6666u64.to_le_bytes(),
            "acked_generation @ +16"
        );
        assert_eq!(&b[24..28], &0x7777_7777u32.to_le_bytes(), "pdb_count @ +24");
        assert_eq!(
            &b[28..32],
            &0x0888_8888u32.to_le_bytes(),
            "pdb_capacity @ +28"
        );
        assert_eq!(&b[32..36], &0x0999_9999u32.to_le_bytes(), "run_count @ +32");
        assert_eq!(
            &b[36..40],
            &0x0AAA_AAAAu32.to_le_bytes(),
            "run_capacity @ +36"
        );
        assert_eq!(
            &b[40..48],
            &0x0BBB_BBBB_CCCC_CCCCu64.to_le_bytes(),
            "entries_visited @ +40"
        );
        assert_eq!(&b[48..52], &0x0DDD_DDDDu32.to_le_bytes(), "refusals @ +48");
        assert_eq!(
            &b[52..56],
            &0x0EEE_EEEEu32.to_le_bytes(),
            "refuse_mask @ +52"
        );
        assert_eq!(
            &b[56..60],
            &0x0FFF_FFFFu32.to_le_bytes(),
            "sparse_slots @ +56"
        );
        // ★★★ `ps_log2` is the field that makes the host's parser format-agnostic. It is a
        // BYTE array at +60 — not a u32 — so a host that read it as one would silently invert
        // the page-size table's order on a big-endian build and, worse, decode the four codes
        // as one number here.
        assert_eq!(
            &b[60..64],
            &[12u8, 16, 21, 29],
            "ps_log2[4] @ +60, one BYTE per code"
        );
        assert_eq!(ReportHeader::SIZE, 64, "the doc and the header both say 64");

        // ⊘ The pins above use a DISTINCTIVE `version` so a mis-sized read of the field is
        // visible; `decode` refuses an unknown version, by design. So the round trip is taken
        // over the same header with the real version — the only field changed.
        let h = ReportHeader {
            version: VERSION,
            ..h
        };
        let mut b = [0u8; ReportHeader::SIZE];
        h.encode_into(&mut b).expect("encode");
        assert_eq!(&b[4..6], &VERSION.to_le_bytes(), "version @ +4");
        assert_eq!(ReportHeader::decode(&b).expect("decode"), h, "round trip");
    }

    /// ★ The two reserved words are pinned to zero. They are the only place a future field can
    /// appear, and a field added there without a matching change here would otherwise be
    /// invisible on this side.
    #[test]
    fn the_pdb_entry_fields_sit_where_the_kernel_puts_them() {
        let p = PdbEntry {
            pdb: 0x1111_1111_2222_2222,
            first_run: 0x3333_3333,
            run_count: 0x4444_4444,
            vas_flags: 0x5555_5555,
            reserved: 0,
            reserved2: 0,
        };
        let mut b = [0u8; PdbEntry::SIZE];
        p.encode_at(&mut b, 0).expect("encode");

        assert_eq!(
            &b[0..8],
            &0x1111_1111_2222_2222u64.to_le_bytes(),
            "pdb @ +0"
        );
        assert_eq!(&b[8..12], &0x3333_3333u32.to_le_bytes(), "first_run @ +8");
        assert_eq!(&b[12..16], &0x4444_4444u32.to_le_bytes(), "run_count @ +12");
        assert_eq!(&b[16..20], &0x5555_5555u32.to_le_bytes(), "vas_flags @ +16");
        assert_eq!(&b[20..24], &[0u8; 4], "★ reserved @ +20 must stay zero");
        assert_eq!(&b[24..32], &[0u8; 8], "★ reserved2 @ +24 must stay zero");
        assert_eq!(PdbEntry::SIZE, 32);

        assert_eq!(PdbEntry::decode_at(&b, 0).expect("decode"), p, "round trip");
    }

    /// ⊘ `op` and `pdb_index` are two `u16` sharing the last 4 bytes. Swapping them is exactly
    /// the mistake that survives a self-round-trip, so they carry distinguishable values here.
    #[test]
    fn the_map_run_fields_sit_where_the_kernel_puts_them() {
        let r = MapRun {
            va: 0x1111_1111_2222_2222,
            gpga: 0x3333_3333_4444_4444,
            len: 0x5555_5555_6666_6666,
            flags: 0x7777_7777,
            op: 0x00AA,
            pdb_index: 0x00BB,
        };
        let mut b = [0u8; MapRun::SIZE];
        r.encode_at(&mut b, 0).expect("encode");

        assert_eq!(&b[0..8], &0x1111_1111_2222_2222u64.to_le_bytes(), "va @ +0");
        assert_eq!(
            &b[8..16],
            &0x3333_3333_4444_4444u64.to_le_bytes(),
            "gpga @ +8"
        );
        assert_eq!(
            &b[16..24],
            &0x5555_5555_6666_6666u64.to_le_bytes(),
            "len @ +16"
        );
        assert_eq!(&b[24..28], &0x7777_7777u32.to_le_bytes(), "flags @ +24");
        assert_eq!(
            &b[28..30],
            &0x00AAu16.to_le_bytes(),
            "★ op @ +28, a u16 not a u32"
        );
        assert_eq!(&b[30..32], &0x00BBu16.to_le_bytes(), "★ pdb_index @ +30");
        assert_eq!(MapRun::SIZE, 32);

        assert_eq!(MapRun::decode_at(&b, 0).expect("decode"), r, "round trip");
    }

    /// The wire values of the three ops, pinned against `kf_walk.h`'s `KFWR_OP_*`.
    #[test]
    fn the_op_codes_are_the_headers_codes() {
        assert_eq!(RunOp::Map.code(), 1);
        assert_eq!(RunOp::Unmap.code(), 2);
        assert_eq!(RunOp::Remap.code(), 3);
        for (code, want) in [(1u16, RunOp::Map), (2, RunOp::Unmap), (3, RunOp::Remap)] {
            let r = MapRun {
                op: code,
                ..MapRun::default()
            };
            assert_eq!(r.op_decoded(0).expect("decode"), want);
        }
        assert!(
            MapRun {
                op: 0,
                ..MapRun::default()
            }
            .op_decoded(0)
            .is_err(),
            "0 is not an op"
        );
        assert!(
            MapRun {
                op: 4,
                ..MapRun::default()
            }
            .op_decoded(0)
            .is_err(),
            "4 is not an op"
        );
    }

    /// The bit positions of every flag, against `kf_walk.h`. ⊘ These are the values a hostile
    /// test names when it asserts *which* refusal fired; a drift here would silently rename
    /// every such assertion.
    #[test]
    fn the_flag_bits_are_the_headers_bits() {
        assert_eq!(
            (HF_TRUNCATED, HF_RESYNC, HF_REFUSED, HF_BUDGET, HF_SCOPED),
            (1, 2, 4, 8, 16)
        );
        assert_eq!((HF_PDB_TRUNCATED, HF_SCOPE_DEGRADED), (32, 64));
        assert_eq!((R_OOB, R_UNALIGNED, R_FOREIGN_AP, R_TOO_DEEP), (1, 2, 4, 8));
        assert_eq!(
            (R_RUN_CAP, R_BUDGET, R_PDB_CAP, R_BAD_SCOPE),
            (16, 32, 64, 128)
        );
        assert_eq!((R_PDB_UNSORTED, R_DELTA_CAP), (256, 512));
        assert_eq!((R_MISALIGNED_LEAF, R_BAD_FORMAT), (1024, 2048));
        assert_eq!((V_NEW, V_GONE, V_RESYNC), (1, 2, 4));
        assert_eq!(
            (RF_READ_ONLY, RF_ATOMIC_DISABLE, RF_VOLATILE, RF_PRIVILEGE),
            (8, 16, 32, 64)
        );
        assert_eq!(
            (RF_AP_SHIFT, RF_AP_MASK, RF_PS_SHIFT, RF_PS_MASK),
            (0, 7, 8, 15)
        );
        assert_eq!(
            MAGIC.to_le_bytes(),
            *b"KFWR",
            "the magic spells KFWR in memory order"
        );
    }

    /// Build a minimal well-formed report image, for the refusal tests below.
    fn image(h: &ReportHeader, pdbs: &[PdbEntry], runs: &[MapRun]) -> Vec<u8> {
        let mut b = vec![0u8; Report::packed_len(h.pdb_count, h.run_count)];
        h.encode_into(&mut b).expect("header");
        for (i, p) in pdbs.iter().enumerate() {
            p.encode_at(&mut b, Report::pdb_array_offset() + i * PdbEntry::SIZE)
                .expect("pdb");
        }
        for (i, r) in runs.iter().enumerate() {
            r.encode_at(
                &mut b,
                Report::run_array_offset(h.pdb_count) + i * MapRun::SIZE,
            )
            .expect("run");
        }
        b
    }

    fn a_header(pdb_count: u32, run_count: u32) -> ReportHeader {
        ReportHeader {
            magic: MAGIC,
            version: VERSION,
            pdb_count,
            pdb_capacity: pdb_count.max(4),
            run_count,
            run_capacity: run_count.max(8),
            ps_log2: [12, 16, 21, 29],
            ..ReportHeader::default()
        }
    }

    fn a_run(va: u64, gpga: u64, len: u64, ps_code: u32) -> MapRun {
        MapRun {
            va,
            gpga,
            len,
            flags: ps_code << RF_PS_SHIFT,
            op: RunOp::Map.code(),
            pdb_index: 0,
        }
    }

    #[test]
    fn a_well_formed_report_parses_and_its_runs_reach_the_diff() {
        let h = a_header(1, 3);
        let pdbs = [PdbEntry {
            pdb: 0xa000_0000,
            first_run: 0,
            run_count: 3,
            ..PdbEntry::default()
        }];
        let runs = [
            a_run(0x1_0000_0000, 0x30000, 4096 * 4, 0),
            a_run(0x1_0010_0000, 0x40000, 65_536 * 2, 1),
            {
                let mut r = a_run(0x1_0040_0000, 0x50000, 0x20_0000, 2);
                r.op = RunOp::Remap.code();
                r
            },
        ];
        let b = image(&h, &pdbs, &runs);
        let rep = Report::parse(&b).expect("parse");
        assert_eq!(rep.header.run_count, 3);
        assert_eq!(rep.runs_of(0).len(), 3, "the PdbEntry's slice resolves");
        assert_eq!(rep.runs[0].page_size(&rep.header, 0).expect("ps"), 4096);
        assert_eq!(rep.runs[1].page_size(&rep.header, 1).expect("ps"), 65_536);
        assert_eq!(
            rep.runs[2].page_size(&rep.header, 2).expect("ps"),
            0x20_0000
        );

        // ★ The whole path: parsed bytes → walkdiff's own type → a diff that closes.
        let cur = rep.present_runs().expect("present");
        assert_eq!(cur.len(), 3, "MAP and REMAP are present mappings");
        assert_eq!(cur[0].class, PageClass::P4K);
        assert_eq!(cur[1].class, PageClass::P64K);
        assert_eq!(cur[2].class, PageClass::P2M);
        let ops = crate::walkdiff::diff(&[], &cur);
        let applied = crate::walkdiff::apply(&[], &ops);
        let mut a = applied;
        let mut c = cur;
        a.sort_by_key(|r| (r.class, r.va));
        c.sort_by_key(|r| (r.class, r.va));
        assert_eq!(a, c, "apply(∅, diff(∅, parsed)) == parsed");
    }

    /// ⊘ An `UNMAP` run's `gpga` is meaningless, so it is not a present mapping. A parser that
    /// passed it through would hand the publisher a binding to garbage.
    #[test]
    fn an_unmap_run_is_not_a_present_mapping() {
        let h = a_header(1, 2);
        let pdbs = [PdbEntry {
            first_run: 0,
            run_count: 2,
            ..PdbEntry::default()
        }];
        let mut u = a_run(0x1_0000_0000, 0xdead_0000, 4096, 0);
        u.op = RunOp::Unmap.code();
        let runs = [u, a_run(0x2_0000_0000, 0x30000, 4096, 0)];
        let rep = Report::parse(&image(&h, &pdbs, &runs)).expect("parse");
        let cur = rep.present_runs().expect("present");
        assert_eq!(cur.len(), 1, "only the MAP survives");
        assert_eq!(cur[0].va, 0x2_0000_0000);
    }

    #[test]
    fn a_bad_magic_or_version_is_refused_by_name() {
        let mut h = a_header(0, 0);
        h.magic = 0xdead_beef;
        let b = image(&h, &[], &[]);
        assert!(matches!(
            Report::parse(&b),
            Err(ParseError::BadMagic(0xdead_beef))
        ));

        let mut h = a_header(0, 0);
        h.version = 99;
        let b = image(&h, &[], &[]);
        assert!(matches!(Report::parse(&b), Err(ParseError::BadVersion(99))));
    }

    /// ★★★ The dangerous shape: a header that claims more than the buffer holds. It must be a
    /// named refusal, never a panic and never a short read that looks whole.
    #[test]
    fn a_header_claiming_more_than_the_buffer_holds_is_refused() {
        let h = a_header(1, 4);
        let pdbs = [PdbEntry {
            first_run: 0,
            run_count: 4,
            ..PdbEntry::default()
        }];
        let runs: Vec<MapRun> = (0..4)
            .map(|i| a_run(0x1_0000_0000 + i * 4096, 0x30000 + i * 4096, 4096, 0))
            .collect();
        let full = image(&h, &pdbs, &runs);
        for cut in [
            0usize,
            1,
            32,
            63,
            ReportHeader::SIZE,
            ReportHeader::SIZE + 1,
            full.len() - 1,
        ] {
            let short = &full[..cut.min(full.len())];
            match Report::parse(short) {
                Ok(_) if cut == full.len() => {}
                Ok(r) => panic!("a {cut}-byte buffer parsed as a whole report: {r:?}"),
                Err(_) => {}
            }
        }
        // And the one that is NOT short, merely lying: a count past its own capacity.
        let mut lying = h;
        lying.run_count = 9;
        lying.run_capacity = 8;
        let mut b = full;
        lying.encode_into(&mut b).expect("header");
        assert!(matches!(
            Report::parse(&b),
            Err(ParseError::CountExceedsCapacity {
                which: "run",
                count: 9,
                capacity: 8
            })
        ));
    }

    /// Property 3: a `PdbEntry` whose slice leaves the run array.
    #[test]
    fn a_pdb_slice_past_the_run_array_is_refused() {
        let h = a_header(1, 2);
        let pdbs = [PdbEntry {
            first_run: 1,
            run_count: 5,
            ..PdbEntry::default()
        }];
        let runs = [a_run(0, 0x30000, 4096, 0), a_run(4096, 0x31000, 4096, 0)];
        assert!(matches!(
            Report::parse(&image(&h, &pdbs, &runs)),
            Err(ParseError::PdbSliceOutOfRange {
                index: 0,
                first_run: 1,
                run_count: 5,
                total: 2
            })
        ));
    }

    #[test]
    fn a_run_naming_no_pdb_is_refused() {
        let h = a_header(1, 1);
        let pdbs = [PdbEntry {
            first_run: 0,
            run_count: 1,
            ..PdbEntry::default()
        }];
        let mut r = a_run(0, 0x30000, 4096, 0);
        r.pdb_index = 7;
        assert!(matches!(
            Report::parse(&image(&h, &pdbs, &[r])),
            Err(ParseError::BadPdbIndex {
                index: 0,
                pdb_index: 7,
                pdb_count: 1
            })
        ));
    }

    /// `len` must be non-zero and a whole number of pages; `va` must be page-aligned.
    /// ⊘ `gpga` is checked by NEITHER, on purpose — see [`Report::validate`].
    #[test]
    fn a_partial_or_misaligned_run_is_refused_but_a_misaligned_gpga_is_not() {
        let h = a_header(1, 1);
        let pdbs = [PdbEntry {
            first_run: 0,
            run_count: 1,
            ..PdbEntry::default()
        }];

        let zero = a_run(0, 0x30000, 0, 0);
        assert!(matches!(
            Report::parse(&image(&h, &pdbs, &[zero])),
            Err(ParseError::BadRunLength { len: 0, .. })
        ));
        let partial = a_run(0, 0x30000, 4095, 0);
        assert!(matches!(
            Report::parse(&image(&h, &pdbs, &[partial])),
            Err(ParseError::BadRunLength { len: 4095, .. })
        ));
        let unaligned_va = a_run(0x800, 0x30000, 4096, 0);
        assert!(matches!(
            Report::parse(&image(&h, &pdbs, &[unaligned_va])),
            Err(ParseError::UnalignedRunVa { va: 0x800, .. })
        ));

        // ★★★ A 512 MiB run whose target is only 4 KiB-aligned. VER2 can spell it, so the host
        // must PARSE it — the refusal belongs in the kernel, by name, and a parser that
        // rejected this would be asserting an alignment the encoding cannot carry.
        let hostile = a_run(0, 0x3000, 0x2000_0000, 3);
        let rep = Report::parse(&image(&h, &pdbs, &[hostile]))
            .expect("a 4 KiB-aligned 512 MiB target must parse");
        assert_eq!(rep.runs[0].gpga, 0x3000);
    }

    /// The page-size code is resolved through the header's own table, so a report whose table
    /// says something absurd is refused rather than producing a nonsense length.
    #[test]
    fn an_absurd_page_size_table_is_refused() {
        let mut h = a_header(1, 1);
        h.ps_log2 = [12, 16, 21, 63];
        let pdbs = [PdbEntry {
            first_run: 0,
            run_count: 1,
            ..PdbEntry::default()
        }];
        let r = a_run(0, 0x30000, 4096, 3);
        assert!(matches!(
            Report::parse(&image(&h, &pdbs, &[r])),
            Err(ParseError::BadPageSize {
                code: 3,
                log2: Some(63),
                ..
            })
        ));
        // The code itself is 4 bits but the table has only 4 entries.
        let h = a_header(1, 1);
        let r = a_run(0, 0x30000, 4096, 9);
        assert!(matches!(
            Report::parse(&image(&h, &pdbs, &[r])),
            Err(ParseError::BadPageSize {
                code: 9,
                log2: None,
                ..
            })
        ));
    }

    /// ⚠ A truncated report must never be applied as a delta (format doc, safety property 1).
    /// The flag is the host's whole defence, so it is read here rather than inferred from
    /// `run_count == run_capacity`, which a hostile guest can also produce legitimately.
    #[test]
    fn truncation_is_visible_from_the_header_alone() {
        let mut h = a_header(0, 0);
        h.flags = HF_TRUNCATED;
        assert!(
            Report::parse(&image(&h, &[], &[]))
                .expect("parse")
                .header
                .is_truncated()
        );
        let mut h = a_header(0, 0);
        h.flags = HF_PDB_TRUNCATED;
        assert!(
            Report::parse(&image(&h, &[], &[]))
                .expect("parse")
                .header
                .is_truncated()
        );
        let mut h = a_header(0, 0);
        h.flags = HF_RESYNC;
        let rep = Report::parse(&image(&h, &[], &[])).expect("parse");
        assert!(!rep.header.is_truncated());
        assert!(rep.header.is_resync());
    }
}
