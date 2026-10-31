//! A **test-only reference decoder** for the Turing..Ada (and GA100) GMMU page-table format,
//! `GMMU_FMT_VERSION_2`, written down from the open kernel modules and from nothing else.
//!
//! # What it is for
//!
//! `cuda/walk/kf_walk.cu` is the only production decoder of guest page tables in v3 (the guest's
//! leaves are copied verbatim, `V3_BUILD.md`; there is no host-side Rust walker). Its inputs and
//! the shipped `kf_format_ver2()` descriptor therefore have no CPU-side judge, and the walk
//! kernel's frozen expected output (`cuda/walk/corpus/*_leaves.txt`) lost the Rust decoder that
//! produced it when `kayfabe-mmu` was archived. This module is that judge again, for tests only:
//! it lives under `tests/`, is compiled into no library, and has no caller outside this test
//! binary.
//!
//! # Provenance: ogkm, never a capture, never the archived code
//!
//! Every constant below is a `hi:lo` pair or a size copied from a published header or from the
//! driver's own level-table initialisers, with the line it came from. Nothing is derived from a
//! closed binary, and no value comes from a trace (the corpus is an INPUT to the cross-check, not
//! a source of constants). The structure is deliberately the driver's own: a table of
//! `MMU_FMT_LEVEL` rows (`virtAddrBitHi`, `virtAddrBitLo`, `entrySize`, `numSubLevels`,
//! `bPageTable`), and `gmmuFmtEntryIsPte` deciding leaf-or-directory per entry. The level
//! geometry (shift, entry count) and the set of leaf sizes are DERIVED from those rows, never
//! typed a second time.
//!
//! Revisions: `ogkm-580:` means 580.159.04, the driver the bench runs. Every `file:line` below
//! was re-read against that tree on 2026-10-31 (`NVIDIA_VERSION = 580.159.04`); the 595.84 tree
//! has the same lines in these files. Paths are relative to the tree root.
//!
//! * `H` = `src/common/inc/swref/published/pascal/gp100/dev_mmu.h`
//! * `LV` = `src/nvidia/src/kernel/gpu/mmu/arch/pascal/kern_gmmu_fmt_gp10x.c`
//! * `GA` = `src/nvidia/src/kernel/gpu/mmu/arch/ampere/kern_gmmu_fmt_ga10x.c`
//! * `AP` = `src/nvidia/src/kernel/gpu/mmu/arch/maxwell/kern_gmmu_fmt_gm10x.c`
//! * `SP` = `src/nvidia/src/kernel/gpu/mmu/arch/maxwell/kern_gmmu_gm200.c`
//! * `IP` = `src/nvidia/src/libraries/mmu/gmmu_fmt.c`
//!
//! The archived Rust decoder this replaces (`archive/crates/kayfabe-chips-modules/src/ga10x.rs`,
//! `Ga10xGmmu`) cited its `ga10x.c` statement at line 52; at 580.159.04 it is line 55
//! (`pLevels[2].bPageTable = NV_TRUE`), which is the number used here.

use std::collections::BTreeMap;

/// A `hi:lo` bit range exactly as `dev_mmu.h` spells it (both ends inclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bits {
    pub hi: u32,
    pub lo: u32,
}

impl Bits {
    /// Number of bits.
    pub const fn width(self) -> u32 {
        self.hi - self.lo + 1
    }

    /// The field's value in `raw` (not shifted).
    pub const fn get(self, raw: u128) -> u64 {
        ((raw >> self.lo) & ((1u128 << self.width()) - 1)) as u64
    }
}

const fn b(hi: u32, lo: u32) -> Bits {
    Bits { hi, lo }
}

/// `NV_MMU_VER2_*` / `NV_MMU_PTE_*`, `H` (gp100 `dev_mmu.h`).
pub mod hdr {
    use super::{Bits, b};

    /// `NV_MMU_VER2_PDE__SIZE`, `H:97`.
    pub const PDE_SIZE: u8 = 8;
    /// `NV_MMU_VER2_DUAL_PDE__SIZE`, `H:112`.
    pub const DUAL_PDE_SIZE: u8 = 16;
    /// `NV_MMU_VER2_PTE__SIZE`, `H:157`.
    pub const PTE_SIZE: u8 = 8;

    /// `NV_MMU_VER2_DUAL_PDE_APERTURE_BIG`, `H:98`.
    pub const DUAL_APERTURE_BIG: Bits = b(2, 1);
    /// `NV_MMU_VER2_DUAL_PDE_VOL_BIG`, `H:99`.
    pub const DUAL_VOL_BIG: Bits = b(3, 3);
    /// `NV_MMU_VER2_DUAL_PDE_ADDRESS_BIG_VID` = `(35-3):(8-4)`, `H:102`.
    pub const DUAL_ADDRESS_BIG_VID: Bits = b(35 - 3, 8 - 4);
    /// `NV_MMU_VER2_DUAL_PDE_ADDRESS_BIG_SYS` = `53:(8-4)`, `H:103`.
    pub const DUAL_ADDRESS_BIG_SYS: Bits = b(53, 8 - 4);
    /// `NV_MMU_VER2_DUAL_PDE_ADDRESS_BIG_SHIFT`, `H:104` (**8**, not 12).
    pub const DUAL_ADDRESS_BIG_SHIFT: u32 = 8;
    /// `NV_MMU_VER2_DUAL_PDE_APERTURE_SMALL`, `H:105` (a bit range of the 128-bit entry).
    pub const DUAL_APERTURE_SMALL: Bits = b(66, 65);
    /// `NV_MMU_VER2_DUAL_PDE_ADDRESS_SMALL_VID` = `(99-3):72`, `H:109`.
    pub const DUAL_ADDRESS_SMALL_VID: Bits = b(99 - 3, 72);
    /// `NV_MMU_VER2_DUAL_PDE_ADDRESS_SMALL_SYS`, `H:110`.
    pub const DUAL_ADDRESS_SMALL_SYS: Bits = b(117, 72);
    /// `NV_MMU_VER2_DUAL_PDE_ADDRESS_SHIFT`, `H:111`.
    pub const DUAL_ADDRESS_SHIFT: u32 = 12;

    /// `NV_MMU_VER2_PDE_ADDRESS_VID` = `(35-3):8`, `H:113`.
    pub const PDE_ADDRESS_VID: Bits = b(35 - 3, 8);
    /// `NV_MMU_VER2_PDE_ADDRESS_SHIFT`, `H:114`.
    pub const PDE_ADDRESS_SHIFT: u32 = 12;
    /// `NV_MMU_VER2_PDE_APERTURE`, `H:115`.
    pub const PDE_APERTURE: Bits = b(2, 1);
    /// `NV_MMU_VER2_PDE_VOL`, `H:116`.
    pub const PDE_VOL: Bits = b(3, 3);
    /// `NV_MMU_VER2_PDE_ADDRESS_SYS`, `H:119`.
    pub const PDE_ADDRESS_SYS: Bits = b(53, 8);

    /// `NV_MMU_VER2_PTE_VALID`, `H:120`.
    pub const PTE_VALID: Bits = b(0, 0);
    /// `NV_MMU_VER2_PTE_APERTURE`, `H:123`.
    pub const PTE_APERTURE: Bits = b(2, 1);
    /// `NV_MMU_VER2_PTE_VOL`, `H:124`.
    pub const PTE_VOL: Bits = b(3, 3);
    /// `NV_MMU_VER2_PTE_PRIVILEGE`, `H:130`.
    pub const PTE_PRIVILEGE: Bits = b(5, 5);
    /// `NV_MMU_VER2_PTE_READ_ONLY`, `H:133`.
    pub const PTE_READ_ONLY: Bits = b(6, 6);
    /// `NV_MMU_VER2_PTE_ATOMIC_DISABLE`, `H:136`.
    pub const PTE_ATOMIC_DISABLE: Bits = b(7, 7);
    /// `NV_MMU_VER2_PTE_ADDRESS_SYS`, `H:139`.
    pub const PTE_ADDRESS_SYS: Bits = b(53, 8);
    /// `NV_MMU_VER2_PTE_ADDRESS_VID` = `(35-3):8`, `H:140`.
    pub const PTE_ADDRESS_VID: Bits = b(35 - 3, 8);
    /// `NV_MMU_VER2_PTE_KIND`, `H:151`.
    pub const PTE_KIND: Bits = b(63, 56);
    /// `NV_MMU_VER2_PTE_ADDRESS_SHIFT`, `H:156`.
    pub const PTE_ADDRESS_SHIFT: u32 = 12;
}

/// Which memory a PTE / PDE points into. The numeric code is the PTE field's own value
/// (`NV_MMU_PTE_APERTURE_*`, `H:80-83`) and is what `kf_walk.h` calls `KFWR_AP_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Aperture {
    Vidmem,
    Peer,
    SysmemCoherent,
    SysmemNonCoherent,
}

impl Aperture {
    /// `KFWR_AP_*`.
    pub const fn code(self) -> u32 {
        match self {
            Aperture::Vidmem => 0,
            Aperture::Peer => 1,
            Aperture::SysmemCoherent => 2,
            Aperture::SysmemNonCoherent => 3,
        }
    }

    fn is_sysmem(self) -> bool {
        matches!(self, Aperture::SysmemCoherent | Aperture::SysmemNonCoherent)
    }
}

/// A PTE's aperture field: `kgmmuFmtInitPteApertures_GM10X` binds `GMMU_APERTURE_VIDEO`,
/// `_PEER`, `_SYS_COH`, `_SYS_NONCOH` to `NV_MMU_PTE_APERTURE_*` = 0, 1, 2, 3 (`AP:184-201`,
/// values `H:80-83`).
pub fn pte_aperture(field: u64) -> Aperture {
    match field {
        0 => Aperture::Vidmem,
        1 => Aperture::Peer,
        2 => Aperture::SysmemCoherent,
        _ => Aperture::SysmemNonCoherent,
    }
}

/// A PDE's aperture field: `kgmmuFmtInitPdeApertures_GM10X` binds `GMMU_APERTURE_INVALID`,
/// `_VIDEO`, `_SYS_COH`, `_SYS_NONCOH` to `NV_MMU_PDE_APERTURE_BIG_*` = 0, 1, 2, 3 (`AP:165-182`,
/// values `H:27-30`). **Not the PTE table**: here 0 is INVALID and video memory is 1.
pub fn pde_aperture(field: u64) -> Option<Aperture> {
    match field {
        0 => None,
        1 => Some(Aperture::Vidmem),
        2 => Some(Aperture::SysmemCoherent),
        _ => Some(Aperture::SysmemNonCoherent),
    }
}

/// One `MMU_FMT_LEVEL` row (`LV:48-105`): the virtual-address range this level indexes, its entry
/// width, its sub-levels, and whether an entry here can be a PTE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    /// `virtAddrBitHi`.
    pub va_hi: u32,
    /// `virtAddrBitLo`.
    pub va_lo: u32,
    /// `entrySize`.
    pub entry_bytes: u8,
    /// `numSubLevels`.
    pub num_sub: u8,
    /// Index of `subLevels[0]` in the level array (the driver writes `pLevels + n`).
    pub first_sub: usize,
    /// `bPageTable`.
    pub page_table: bool,
}

/// The six-row level array of `kgmmuFmtInitLevels_GP10X` with `bigPageShift = 16`
/// (`LV:48-105`; `bigPageShift` is 16 or 17 by `LV:56`, and 16 is the 64 KiB page).
pub const fn gp10x_levels() -> [Level; 6] {
    [
        // Page directory 3 (root), `LV:58-64`.
        Level {
            va_hi: 48,
            va_lo: 47,
            entry_bytes: hdr::PDE_SIZE,
            num_sub: 1,
            first_sub: 1,
            page_table: false,
        },
        // Page directory 2, `LV:66-72`.
        Level {
            va_hi: 46,
            va_lo: 38,
            entry_bytes: hdr::PDE_SIZE,
            num_sub: 1,
            first_sub: 2,
            page_table: false,
        },
        // Page directory 1, `LV:74-80`.
        Level {
            va_hi: 37,
            va_lo: 29,
            entry_bytes: hdr::PDE_SIZE,
            num_sub: 1,
            first_sub: 3,
            page_table: false,
        },
        // Page directory 0, `LV:82-89`: dual, two sub-levels, and `bPageTable` (`LV:87`).
        Level {
            va_hi: 28,
            va_lo: 21,
            entry_bytes: hdr::DUAL_PDE_SIZE,
            num_sub: 2,
            first_sub: 4,
            page_table: true,
        },
        // Big page table, `LV:92-97` (`virtAddrBitLo = bigPageShift`).
        Level {
            va_hi: 20,
            va_lo: 16,
            entry_bytes: hdr::PTE_SIZE,
            num_sub: 0,
            first_sub: 0,
            page_table: true,
        },
        // Small page table, `LV:99-104`.
        Level {
            va_hi: 20,
            va_lo: 12,
            entry_bytes: hdr::PTE_SIZE,
            num_sub: 0,
            first_sub: 0,
            page_table: true,
        },
    ]
}

/// `kgmmuFmtInitLevels_GA10X` (`GA:52-55`): the GP10x levels, then the one GA10x statement,
/// `pLevels[2].bPageTable = NV_TRUE` (`GA:55`, comment `GA:54`: "Page directory 1 can now hold a
/// PTE pointing to a 512MB Page").
pub const fn ga10x_levels() -> [Level; 6] {
    let mut l = gp10x_levels();
    l[2].page_table = true;
    l
}

/// Entry count and VA shift of one level, derived from its `virtAddrBitHi:Lo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelShift {
    pub shift: u8,
    pub entries: u32,
}

/// A leaf page size in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PageSize(pub u64);

/// One decoded slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decode {
    /// Nothing written here.
    Invalid,
    /// The guest declared the range backing-free (`SP:43-70`).
    Sparse,
    /// A pointer to one sub-table, or two (a dual entry).
    Pde { edge: Edge, also: Option<Edge> },
    /// A leaf mapping.
    Leaf {
        phys: u64,
        aperture: Aperture,
        size: PageSize,
        read_only: bool,
    },
}

/// One sub-table a directory entry names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub next: u64,
    pub aperture: Aperture,
    pub child_level: usize,
}

/// The `GMMU_FMT_VERSION_2` format over a level array.
#[derive(Debug, Clone, Copy)]
pub struct Ver2 {
    levels: [Level; 6],
}

impl Ver2 {
    /// Ampere GA10x / Ada (`kgmmuFmtInitLevels_GA10X`): PD1 is a leaf level too.
    pub const fn ga10x() -> Ver2 {
        Ver2 {
            levels: ga10x_levels(),
        }
    }

    /// Turing / Volta / Pascal (`kgmmuFmtInitLevels_GP10X`): only PD0 and the page tables are
    /// leaf levels.
    pub const fn gp10x() -> Ver2 {
        Ver2 {
            levels: gp10x_levels(),
        }
    }

    /// The level rows.
    pub const fn levels(&self) -> &[Level; 6] {
        &self.levels
    }

    /// Entries a table at `level` holds, and the VA bit its index starts at. `None` for a level
    /// the format does not have. The count comes from the bit range, so the big-page table is 32
    /// entries (`20:16`), never `4096 / 8`.
    pub fn level_shift(&self, level: usize) -> Option<LevelShift> {
        let l = self.levels.get(level)?;
        Some(LevelShift {
            shift: u8::try_from(l.va_lo).ok()?,
            entries: 1u32 << (l.va_hi - l.va_lo + 1),
        })
    }

    /// Bytes per entry at `level`; `0` for a level that does not exist.
    pub fn entry_size(&self, level: usize) -> u8 {
        self.levels.get(level).map_or(0, |l| l.entry_bytes)
    }

    /// Depth of a small-page walk (PD3, PD2, PD1, PD0, small PT): follow `subLevels[0]` from the
    /// root, taking the LAST sub-level of a dual level.
    pub fn small_page_walk_depth(&self) -> u8 {
        let mut depth = 1u8;
        let mut at = 0usize;
        while self.levels[at].num_sub > 0 {
            let l = self.levels[at];
            at = l.first_sub + usize::from(l.num_sub) - 1;
            depth += 1;
        }
        depth
    }

    /// Every leaf size some level can hold, ascending: the span of one entry
    /// (`1 << virtAddrBitLo`) of each `bPageTable` level.
    pub fn page_sizes(&self) -> Vec<PageSize> {
        let mut v: Vec<PageSize> = self
            .levels
            .iter()
            .filter(|l| l.page_table)
            .map(|l| PageSize(1u64 << l.va_lo))
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// `gmmuFmtEntryIsPte` (`IP:63-86`): at a level that is both a page table and a directory the
    /// PTE's valid bit decides; at a pure page table the entry is a PTE; a pure directory never.
    fn entry_is_pte(l: &Level, raw: u128) -> bool {
        let page_dir = l.num_sub > 0;
        if l.page_table && page_dir {
            hdr::PTE_VALID.get(raw) == 1
        } else {
            l.page_table
        }
    }

    /// Decode one raw entry read at `level`.
    pub fn decode_entry(&self, level: usize, raw: u128) -> Decode {
        let Some(l) = self.levels.get(level) else {
            // A level that does not exist carries no declaration, not even a sparse one.
            return Decode::Invalid;
        };
        if Self::entry_is_pte(l, raw) {
            return Self::decode_pte(l, raw);
        }
        if l.num_sub == 2 {
            return Self::decode_dual(l, raw);
        }
        Self::decode_pde(l, raw)
    }

    /// An entry that is a PTE slot: valid -> leaf of this level's span; otherwise the sparse or
    /// invalid encoding (`SP:43-47`: sparse = valid clear, volatile set).
    fn decode_pte(l: &Level, raw: u128) -> Decode {
        if hdr::PTE_VALID.get(raw) == 0 {
            return empty(hdr::PTE_VOL.get(raw));
        }
        let aperture = pte_aperture(hdr::PTE_APERTURE.get(raw));
        // `LV:172-176, 178-194`: video and peer use the VID address field; a discrete GPU (no
        // unified aperture) uses the SYS field for sysmem.
        let addr = if aperture.is_sysmem() {
            hdr::PTE_ADDRESS_SYS
        } else {
            hdr::PTE_ADDRESS_VID
        };
        Decode::Leaf {
            phys: addr.get(raw) << hdr::PTE_ADDRESS_SHIFT,
            aperture,
            size: PageSize(1u64 << l.va_lo),
            read_only: hdr::PTE_READ_ONLY.get(raw) == 1,
        }
    }

    /// A single-entry PDE (`LV:138-153`): the aperture is the validity; INVALID + volatile is the
    /// sparse PDE (`SP:49-54`).
    fn decode_pde(l: &Level, raw: u128) -> Decode {
        let Some(aperture) = pde_aperture(hdr::PDE_APERTURE.get(raw)) else {
            return empty(hdr::PDE_VOL.get(raw));
        };
        let addr = if aperture.is_sysmem() {
            hdr::PDE_ADDRESS_SYS
        } else {
            hdr::PDE_ADDRESS_VID
        };
        Decode::Pde {
            edge: Edge {
                next: addr.get(raw) << hdr::PDE_ADDRESS_SHIFT,
                aperture,
                child_level: l.first_sub,
            },
            also: None,
        }
    }

    /// The dual PDE (`kgmmuFmtInitPdeMulti_GP10X`, `LV:107-136`): `subLevels[0]` is the BIG half,
    /// `subLevels[1]` the SMALL half, at different bit ranges and different shifts. Sparse is
    /// spelled on sub-level 0 only (`SP:57-68`: `if (0 == i)` at `:65` sets the volatile bit).
    fn decode_dual(l: &Level, raw: u128) -> Decode {
        let small = pde_aperture(hdr::DUAL_APERTURE_SMALL.get(raw)).map(|aperture| {
            let addr = if aperture.is_sysmem() {
                hdr::DUAL_ADDRESS_SMALL_SYS
            } else {
                hdr::DUAL_ADDRESS_SMALL_VID
            };
            Edge {
                next: addr.get(raw) << hdr::DUAL_ADDRESS_SHIFT,
                aperture,
                child_level: l.first_sub + 1,
            }
        });
        let big = pde_aperture(hdr::DUAL_APERTURE_BIG.get(raw)).map(|aperture| {
            let addr = if aperture.is_sysmem() {
                hdr::DUAL_ADDRESS_BIG_SYS
            } else {
                hdr::DUAL_ADDRESS_BIG_VID
            };
            Edge {
                next: addr.get(raw) << hdr::DUAL_ADDRESS_BIG_SHIFT,
                aperture,
                child_level: l.first_sub,
            }
        });
        // The small-page table is what RM populates for ordinary mappings, and a point query
        // follows `edge` first; a half that is present is never dropped.
        match (small, big) {
            (None, None) => empty(hdr::DUAL_VOL_BIG.get(raw)),
            (Some(edge), also) => Decode::Pde { edge, also },
            (None, Some(edge)) => Decode::Pde { edge, also: None },
        }
    }
}

fn empty(volatile: u64) -> Decode {
    if volatile == 1 {
        Decode::Sparse
    } else {
        Decode::Invalid
    }
}

// =============================================================================================
// Walking an image
// =============================================================================================

/// A sparse byte image standing in for the fabricated vidmem aperture: 4 KiB pages by offset,
/// absent pages read as zero, reads past `len` fail.
#[derive(Debug, Default)]
pub struct Image {
    pub len: u64,
    pub pages: BTreeMap<u64, Box<[u8; 4096]>>,
}

impl Image {
    /// Fill `buf` from `phys`. `false` when the read would run past the image.
    pub fn read(&self, phys: u64, buf: &mut [u8]) -> bool {
        let Some(end) = phys.checked_add(buf.len() as u64) else {
            return false;
        };
        if end > self.len {
            return false;
        }
        for (i, out) in buf.iter_mut().enumerate() {
            let at = phys + i as u64;
            *out = self
                .pages
                .get(&(at & !0xFFF))
                .map_or(0, |p| p[(at & 0xFFF) as usize]);
        }
        true
    }
}

/// One table page to decode, with the metadata its parent's entry gave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub phys: u64,
    pub aperture: Aperture,
    pub level: usize,
    pub vabase: u64,
}

/// One leaf found by a walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Leaf {
    pub va: u64,
    pub phys: u64,
    pub aperture: Aperture,
    pub size: PageSize,
    pub read_only: bool,
}

/// A branch that could not be decoded. Kept and returned: a faulted branch is not an empty one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// The table page lies (partly) outside the image.
    Unbacked { phys: u64, level: usize },
    /// A table page that is not in video memory. The image serves vidmem only.
    ForeignAperture { phys: u64, level: usize },
    /// The depth bound was reached (a guest-built cycle).
    TooDeep { phys: u64 },
}

/// How a directory entry whose sub-table address is ZERO is treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZeroChild {
    /// ogkm-580: the aperture is the validity, so a zero address is a real table at offset 0
    /// (`_gmmuGetPdePa` yields `MMU_INVALID_ADDR` only for `GMMU_APERTURE_INVALID`,
    /// `src/nvidia/src/kernel/gpu/mmu/gmmu_trace.c:111-134`).
    IsATable,
    /// The archived decoder's rule: a null pointer is skipped.
    Skipped,
}

/// What a bounded descent produced.
#[derive(Debug, Default)]
pub struct Subtree {
    pub leaves: Vec<Leaf>,
    /// Virtual addresses of slots the guest declared sparse.
    pub sparse: Vec<u64>,
    pub faults: Vec<Fault>,
    pub pages_visited: usize,
}

/// The descent ran out of its entry budget; the result as a whole is untrustworthy.
#[derive(Debug, PartialEq, Eq)]
pub struct BudgetExhausted;

/// A cycle is a guest-writable shape, so depth is bounded independently of the format's levels.
pub const MAX_DEPTH: u8 = 16;

/// Decode every leaf reachable from `root`, depth first with an explicit stack, in ascending VA
/// order within a page. `budget` counts entries examined.
///
/// # Errors
/// [`BudgetExhausted`] only; every per-branch problem lands in [`Subtree::faults`].
pub fn decode_subtree(
    fmt: &Ver2,
    img: &Image,
    root: Page,
    budget: u32,
    zero_child: ZeroChild,
) -> Result<Subtree, BudgetExhausted> {
    let mut out = Subtree::default();
    let mut left = budget;
    let mut stack = vec![(root, 0u8)];
    while let Some((page, depth)) = stack.pop() {
        if depth >= MAX_DEPTH {
            out.faults.push(Fault::TooDeep { phys: page.phys });
            continue;
        }
        let Some(geo) = fmt.level_shift(page.level) else {
            // No such level: nothing to read.
            continue;
        };
        if left < geo.entries {
            return Err(BudgetExhausted);
        }
        left -= geo.entries;
        if page.aperture != Aperture::Vidmem {
            out.faults.push(Fault::ForeignAperture {
                phys: page.phys,
                level: page.level,
            });
            continue;
        }
        let width = usize::from(fmt.entry_size(page.level));
        let mut bytes = vec![0u8; geo.entries as usize * width];
        if !img.read(page.phys, &mut bytes) {
            out.faults.push(Fault::Unbacked {
                phys: page.phys,
                level: page.level,
            });
            continue;
        }
        out.pages_visited += 1;
        let mut children = Vec::new();
        for i in 0..geo.entries {
            let at = i as usize * width;
            let mut raw = [0u8; 16];
            raw[..width].copy_from_slice(&bytes[at..at + width]);
            let va = page.vabase | (u64::from(i) << geo.shift);
            match fmt.decode_entry(page.level, u128::from_le_bytes(raw)) {
                Decode::Invalid => {}
                Decode::Sparse => out.sparse.push(va),
                Decode::Pde { edge, also } => {
                    for e in [Some(edge), also].into_iter().flatten() {
                        if e.next == 0 && zero_child == ZeroChild::Skipped {
                            continue;
                        }
                        children.push(Page {
                            phys: e.next,
                            aperture: e.aperture,
                            level: e.child_level,
                            vabase: va,
                        });
                    }
                }
                Decode::Leaf {
                    phys,
                    aperture,
                    size,
                    read_only,
                } => out.leaves.push(Leaf {
                    va,
                    phys,
                    aperture,
                    size,
                    read_only,
                }),
            }
        }
        // Reverse, so the pop order is ascending in VA.
        for c in children.into_iter().rev() {
            stack.push((c, depth + 1));
        }
    }
    Ok(out)
}
