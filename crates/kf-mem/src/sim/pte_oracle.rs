//! ★ 2026-11-18 — **THE GMMU REFERENCE TRANSLATOR** of the guest-PTE → host-PTE differential fuzz
//! (`docs/design/V3_PTE_MODEL_FUZZ.md`). TEST-ONLY. Written from the pinned ogkm sources, never from
//! kayfabe's own constants: every field below names the `ogkm-580` (580.159.04,
//! `third_party/ogkm-580`) file and line it was transcribed from, and `pte_fuzz::oracle_fields_*`
//! re-reads those lines from the checkout when it is present.
//!
//! Paths are abbreviated: `sw/` = `src/common/inc/swref/published/`, `mmu/` =
//! `src/nvidia/src/kernel/gpu/mmu/`.
//!
//! What it answers, for one guest VA: the translation the GPU's MMU makes from the guest's own
//! tables — a leaf (page size, aperture, address, KIND, permissions), SPARSE (reads zero, no fault),
//! INVALID (faults), or a state the oracle cannot decide from the sources (`Undecided`, reported and
//! never asserted on).

use std::collections::BTreeMap;

/// The level geometry ogkm builds per chip (`mmu/arch/*/kern_gmmu_fmt_*.c`, selected per chip by
/// `src/nvidia/generated/g_kern_gmmu_nvoc.c:670-690`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Geo {
    /// `kgmmuFmtInitLevels_GP10X` (`mmu/arch/pascal/kern_gmmu_fmt_gp10x.c:48-105`): TU10x/TU11x
    /// (`g_kern_gmmu_nvoc.c:679-682`). PD1 holds NO page.
    Gp10x,
    /// `kgmmuFmtInitLevels_GA10X` (`mmu/arch/ampere/kern_gmmu_fmt_ga10x.c:46-56`): GA10x and AD10x
    /// (`g_kern_gmmu_nvoc.c:683-686`): GP10X plus a 512 MiB page at PD1 (`:55`).
    Ga10x,
    /// `kgmmuFmtInitLevels_GH10X` (`mmu/arch/hopper/kern_gmmu_fmt_gh10x.c:63-116`): GH100.
    Gh10x,
    /// `kgmmuFmtInitLevels_GB10X` (`mmu/arch/blackwell/kern_gmmu_fmt_gb10x.c:50-61`): every other
    /// chip (`g_kern_gmmu_nvoc.c:687-690`, the Blackwell parts): GH10X plus a 256 GiB page at PD2
    /// (`:60`).
    Gb10x,
}

impl Geo {
    /// VER2 (`GMMU_FMT_VERSION_2`) or VER3.
    #[must_use]
    pub const fn ver3(self) -> bool {
        matches!(self, Geo::Gh10x | Geo::Gb10x)
    }

    /// The directory levels above the dual PD0, root first: `(va_lo, va_hi, holds_a_page)`.
    /// VER2: `gp10x.c:59-80` (PD3 48:47, PD2 46:38, PD1 37:29). VER3: `gh10x.c:63-93` (PD4 56:56,
    /// PD3 55:47, PD2 46:38, PD1 37:29 with `bPageTable` at `:93`).
    #[must_use]
    pub fn dirs(self) -> Vec<(u32, u32, bool)> {
        match self {
            Geo::Gp10x => vec![(47, 48, false), (38, 46, false), (29, 37, false)],
            Geo::Ga10x => vec![(47, 48, false), (38, 46, false), (29, 37, true)],
            Geo::Gh10x => vec![(56, 56, false), (47, 55, false), (38, 46, false), (29, 37, true)],
            Geo::Gb10x => vec![(56, 56, false), (47, 55, false), (38, 46, true), (29, 37, true)],
        }
    }
}

/// PD0 is a dual level `28:21` that also holds a 2 MiB page (`gp10x.c:83-89`, `gh10x.c:97-101`);
/// big table `20:16` (bigPageShift 16), small `20:12` (`gp10x.c:93-103`, `gh10x.c:106-116`).
pub const PD0_LO: u32 = 21;
/// The big (64 KiB) table's first VA bit.
pub const BIG_LO: u32 = 16;
/// The small (4 KiB) table's first VA bit.
pub const SMALL_LO: u32 = 12;

/// `bits(raw, hi, lo)` = `raw[hi:lo]`.
#[must_use]
pub const fn bits(raw: u64, hi: u32, lo: u32) -> u64 {
    (raw >> lo) & (u64::MAX >> (63 - (hi - lo)))
}

/// A PTE aperture (`sw/pascal/gp100/dev_mmu.h:80-83` for VER2; `sw/hopper/gh100/dev_mmu.h:134-138`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Aperture {
    /// `_VIDEO_MEMORY` 0.
    Video,
    /// `_PEER_MEMORY` 1.
    Peer,
    /// `_SYSTEM_COHERENT_MEMORY` 2.
    SysCoherent,
    /// `_SYSTEM_NON_COHERENT_MEMORY` 3.
    SysNonCoherent,
}

impl Aperture {
    const fn of(v: u64) -> Self {
        match v & 3 {
            0 => Aperture::Video,
            1 => Aperture::Peer,
            2 => Aperture::SysCoherent,
            _ => Aperture::SysNonCoherent,
        }
    }
}

/// One decoded leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Leaf {
    /// The page's first VA.
    pub va: u64,
    /// log2 of its size.
    pub ps_log2: u32,
    /// Its aperture.
    pub ap: Aperture,
    /// The physical address of the page's first byte (vidmem offset or guest-physical).
    pub pa: u64,
    /// `KIND`, raw.
    pub kind: u8,
    /// READ_ONLY.
    pub ro: bool,
    /// VOL / UNCACHED.
    pub vol: bool,
    /// PRIVILEGE.
    pub privileged: bool,
    /// ATOMIC_DISABLE / NO_ATOMIC.
    pub atomic_disable: bool,
    /// The raw PTE word.
    pub raw: u64,
}

/// A PTE as the oracle reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pte {
    /// VALID.
    Valid,
    /// SPARSE: VER2 `VALID=0 VOL=1` (`mmu/arch/maxwell/kern_gmmu_gm200.c:44-47`); VER3
    /// `VALID=0 PCF=SPARSE(1)` (`sw/hopper/gh100/dev_mmu.h:141`).
    Sparse,
    /// A big PTE that hides the 4 KiB PTEs under it: VER2 `VALID=0 VOL=0 PRIVILEGE=1`
    /// (`kernel-open/nvidia-uvm/uvm_mmu.h:201-210`, the "unmapped" pattern); VER3
    /// `PCF=NO_VALID_4KB_PAGE(3)` (`gh100/dev_mmu.h:143`).
    Unmapped,
    /// All zeros, or VER3 `PCF=0`: invalid ("there might be 4k PTEs present instead",
    /// `uvm_mmu.h:205-207`).
    Invalid,
    /// An invalid PTE in a state the sources do not define for a big PTE (VER3 `MAPPING_NOWHERE`,
    /// any other VER2 combination with VALID=0 and stray bits).
    Other,
}

/// Classify a PTE word.
#[must_use]
pub fn pte_state(geo: Geo, raw: u64) -> Pte {
    if bits(raw, 0, 0) == 1 {
        return Pte::Valid; // VER2 `_VALID 0:0` (`gp100/dev_mmu.h:120`); VER3 (`gh100/dev_mmu.h:131`).
    }
    if geo.ver3() {
        // `NV_MMU_VER3_PTE_PCF 7:3` (`gh100/dev_mmu.h:139`), invalid enumerants `:140-143`.
        match bits(raw, 7, 3) {
            0 => Pte::Invalid,
            1 => Pte::Sparse,
            3 => Pte::Unmapped,
            _ => Pte::Other,
        }
    } else if bits(raw, 3, 3) == 1 {
        Pte::Sparse // `NV_MMU_VER2_PTE_VOL 3:3` (`gp100/dev_mmu.h:124`).
    } else if bits(raw, 5, 5) == 1 {
        Pte::Unmapped // `NV_MMU_VER2_PTE_PRIVILEGE 5:5` (`gp100/dev_mmu.h:130`).
    } else if raw == 0 {
        Pte::Invalid
    } else {
        // VALID=0 VOL=0 PRIV=0 with other bits set: the MMU reads only VALID/VOL/PRIV of an
        // invalid entry as far as the sources say; treated as invalid for a small PTE, undecided
        // for a big one.
        Pte::Other
    }
}

/// Decode a VALID PTE word into a leaf at `va` of size `1 << ps_log2`.
#[must_use]
pub fn decode_pte(geo: Geo, raw: u64, va: u64, ps_log2: u32) -> Leaf {
    let ap = Aperture::of(bits(raw, 2, 1)); // VER2 `_APERTURE 2:1` (`gp100:123`), VER3 (`gh100:134`).
    if geo.ver3() {
        // `NV_MMU_VER3_PTE_PCF 7:3` (`gh100/dev_mmu.h:139`), valid enumerants `:144-159`:
        // bit0 UNCACHED, bit1 PRIVILEGE, bit2 RO, bit3 NO_ATOMIC (bit4 = the `_ACD` set).
        let pcf = bits(raw, 7, 3);
        Leaf {
            va,
            ps_log2,
            ap,
            pa: bits(raw, 51, 12) << 12, // `NV_MMU_VER3_PTE_ADDRESS 51:12`, shift 12 (`gh100:177,187`).
            kind: bits(raw, 11, 8) as u8, // `NV_MMU_VER3_PTE_KIND 11:8` (`gh100:176`).
            vol: pcf & 1 != 0,
            privileged: pcf & 2 != 0,
            ro: pcf & 4 != 0,
            atomic_disable: pcf & 8 != 0,
            raw,
        }
    } else {
        let pa = match ap {
            // `NV_MMU_VER2_PTE_ADDRESS_VID (35-3):8` (`gp100/dev_mmu.h:140`), shift 12 (`:156`).
            Aperture::Video | Aperture::Peer => bits(raw, 32, 8) << 12,
            // `NV_MMU_VER2_PTE_ADDRESS_SYS 53:8` (`gp100/dev_mmu.h:139`).
            _ => bits(raw, 53, 8) << 12,
        };
        Leaf {
            va,
            ps_log2,
            ap,
            pa,
            kind: bits(raw, 63, 56) as u8, // `NV_MMU_VER2_PTE_KIND 63:56` (`gp100/dev_mmu.h:151`).
            vol: bits(raw, 3, 3) == 1,     // `_VOL 3:3` (`:124`).
            privileged: bits(raw, 5, 5) == 1, // `_PRIVILEGE 5:5` (`:130`).
            ro: bits(raw, 6, 6) == 1,      // `_READ_ONLY 6:6` (`:133`).
            atomic_disable: bits(raw, 7, 7) == 1, // `_ATOMIC_DISABLE 7:7` (`:136`).
            raw,
        }
    }
}

/// A PDE (or one half of a dual PDE) as the oracle reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pde {
    /// A sub-table at this address, in this aperture (PDE aperture values: VER2
    /// `sw/pascal/gp100/dev_mmu.h:27-30` — INVALID 0, VIDEO 1, SYS_COH 2, SYS_NC 3; VER3
    /// `gh100/dev_mmu.h:60-63`, the same values).
    Table(u8, u64),
    /// No sub-table, SPARSE.
    Sparse,
    /// No sub-table.
    Invalid,
}

/// Decode a (non-dual) PDE: aperture `2:1`, VOL `3:3`, address VID `32:8` / SYS `53:8` (VER2,
/// `gp100/dev_mmu.h:113-119`); aperture `2:1`, PCF `5:3`, address `51:12` (VER3,
/// `gh100/dev_mmu.h:59-78`; sparse = PCF bit 0 with an INVALID aperture, `:65-76`).
#[must_use]
pub fn decode_pde(geo: Geo, raw: u64) -> Pde {
    let ap = bits(raw, 2, 1) as u8;
    if ap == 0 {
        let sparse = if geo.ver3() {
            bits(raw, 5, 3) & 1 == 1
        } else {
            bits(raw, 3, 3) == 1
        };
        return if sparse { Pde::Sparse } else { Pde::Invalid };
    }
    let addr = if geo.ver3() {
        bits(raw, 51, 12) << 12
    } else if ap == 1 {
        bits(raw, 32, 8) << 12
    } else {
        bits(raw, 53, 8) << 12
    };
    Pde::Table(ap, addr)
}

/// The dual PDE's BIG half (the low word): aperture `2:1`, VOL `3:3`, address VID `(35-3):(8-4)`
/// / SYS `53:(8-4)` shift 8 (VER2, `gp100/dev_mmu.h:98-104`); aperture `2:1`, PCF `5:3`, address
/// `51:8` shift 8 (VER3, `gh100/dev_mmu.h:88-106,127`).
#[must_use]
pub fn decode_dual_big(geo: Geo, lo: u64) -> Pde {
    let ap = bits(lo, 2, 1) as u8;
    if ap == 0 {
        let sparse = if geo.ver3() {
            bits(lo, 5, 3) & 1 == 1
        } else {
            bits(lo, 3, 3) == 1
        };
        return if sparse { Pde::Sparse } else { Pde::Invalid };
    }
    let addr = if geo.ver3() {
        bits(lo, 51, 8) << 8
    } else if ap == 1 {
        bits(lo, 32, 4) << 8
    } else {
        bits(lo, 53, 4) << 8
    };
    Pde::Table(ap, addr)
}

/// The dual PDE's SMALL half (the high word, bits 64+): aperture `66:65`, VOL `67:67`, address VID
/// `(99-3):72` / SYS `117:72` shift 12 (VER2, `gp100/dev_mmu.h:105-111`); aperture `66:65`, PCF
/// `69:67`, address `115:76` shift 12 (VER3, `gh100/dev_mmu.h:107-126`).
#[must_use]
pub fn decode_dual_small(geo: Geo, hi: u64) -> Pde {
    let ap = bits(hi, 2, 1) as u8;
    if ap == 0 {
        let sparse = if geo.ver3() {
            bits(hi, 5, 3) & 1 == 1
        } else {
            bits(hi, 3, 3) == 1
        };
        return if sparse { Pde::Sparse } else { Pde::Invalid };
    }
    let addr = if geo.ver3() {
        bits(hi, 51, 12) << 12
    } else if ap == 1 {
        bits(hi, 32, 8) << 12
    } else {
        bits(hi, 53, 8) << 12
    };
    Pde::Table(ap, addr)
}

/// What the MMU makes of one VA range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Xlate {
    /// A valid leaf.
    Map(Leaf),
    /// SPARSE: reads return zero, writes are dropped, no fault (`uvm_mmu.h:195-199`).
    Sparse,
    /// The sources do not say what the MMU does here (never asserted on).
    Undecided(&'static str),
    /// A page table the MMU would read lives outside what kayfabe can read (system memory, or
    /// beyond the guest's vidmem): kayfabe refuses it by name (`KFWR_R_FOREIGN_AP` / `_OOB`).
    Unreadable(&'static str),
}

/// Guest vidmem: `len` bytes of page tables and data, page-sparse.
#[derive(Debug, Clone, Default)]
pub struct Mem {
    /// Bytes of guest vidmem (the store; also the walker's window and span).
    pub len: u64,
    /// 4 KiB pages, by offset; absent = zeros.
    pub pages: BTreeMap<u64, Box<[u64; 512]>>,
}

impl Mem {
    /// The 8-byte word at `off` (`None` past the end).
    #[must_use]
    pub fn word(&self, off: u64) -> Option<u64> {
        if off.checked_add(8)? > self.len {
            return None;
        }
        Some(
            self.pages
                .get(&(off & !0xFFF))
                .map_or(0, |p| p[((off & 0xFFF) / 8) as usize]),
        )
    }
    /// Store an 8-byte word (must be in range).
    pub fn set(&mut self, off: u64, v: u64) {
        assert!(off + 8 <= self.len && off.is_multiple_of(8), "oracle mem write {off:#x}");
        self.pages
            .entry(off & !0xFFF)
            .or_insert_with(|| Box::new([0; 512]))[((off & 0xFFF) / 8) as usize] = v;
    }
}

/// Whether a table of `bytes` at `addr` in aperture `ap` (PDE values) is readable by kayfabe:
/// vidmem, inside guest vidmem, aligned to its size (ogkm allocates every table naturally aligned).
fn table_ok(mem: &Mem, ap: u8, addr: u64, bytes: u64) -> Result<(), &'static str> {
    if ap != 1 {
        return Err("page table in system memory");
    }
    if !addr.is_multiple_of(bytes) {
        return Err("unaligned page table");
    }
    if addr.checked_add(bytes).is_none_or(|e| e > mem.len) {
        return Err("page table beyond guest vidmem");
    }
    Ok(())
}

/// ★ THE REFERENCE: every translated range of the space rooted at `root`, as `(va, len, Xlate)`,
/// sorted, disjoint; a VA not listed is INVALID (a fault). Recursive and obviously-bounded: one
/// visit per entry of every reachable table.
#[must_use]
pub fn enumerate(geo: Geo, mem: &Mem, root: u64) -> Vec<(u64, u64, Xlate)> {
    let mut out = Vec::new();
    let dirs = geo.dirs();
    let (lo0, hi0, _) = dirs[0];
    let root_bytes = 8u64 << (hi0 - lo0 + 1);
    if let Err(why) = table_ok(mem, 1, root, root_bytes.max(4096)) {
        out.push((0, 1u64 << (hi0 + 1), Xlate::Unreadable(why)));
        return out;
    }
    dir(geo, mem, &dirs, 0, root, 0, &mut out);
    out.sort_by_key(|x| x.0);
    out
}

fn dir(
    geo: Geo,
    mem: &Mem,
    dirs: &[(u32, u32, bool)],
    k: usize,
    tbl: u64,
    base: u64,
    out: &mut Vec<(u64, u64, Xlate)>,
) {
    let (lo, hi, holds_page) = dirs[k];
    let n = 1u64 << (hi - lo + 1);
    for i in 0..n {
        let va = base | (i << lo);
        let span = 1u64 << lo;
        let Some(raw) = mem.word(tbl + i * 8) else {
            continue;
        };
        // `gmmuFmtEntryIsPte` (`src/nvidia/src/libraries/mmu/gmmu_fmt.c:64-86`): on a level that is
        // both a page table and a directory, the PTE's VALID bit decides; on a directory-only level
        // the entry is a PDE whatever bit 0 holds.
        if holds_page && bits(raw, 0, 0) == 1 {
            out.push((va, span, Xlate::Map(decode_pte(geo, raw, va, lo))));
            continue;
        }
        match decode_pde(geo, raw) {
            Pde::Invalid => {}
            Pde::Sparse => out.push((va, span, Xlate::Sparse)),
            Pde::Table(ap, addr) => {
                let child_bytes = if k + 1 < dirs.len() {
                    let (l, h, _) = dirs[k + 1];
                    8u64 << (h - l + 1)
                } else {
                    16u64 << (28 - PD0_LO + 1)
                };
                match table_ok(mem, ap, addr, child_bytes) {
                    Err(why) => out.push((va, span, Xlate::Unreadable(why))),
                    Ok(()) if k + 1 < dirs.len() => dir(geo, mem, dirs, k + 1, addr, va, out),
                    Ok(()) => pd0(geo, mem, addr, va, out),
                }
            }
        }
    }
}

fn pd0(geo: Geo, mem: &Mem, tbl: u64, base: u64, out: &mut Vec<(u64, u64, Xlate)>) {
    for i in 0..(1u64 << (28 - PD0_LO + 1)) {
        let va = base | (i << PD0_LO);
        let (Some(lo), Some(hi)) = (mem.word(tbl + i * 16), mem.word(tbl + i * 16 + 8)) else {
            continue;
        };
        // PD0 holds a 2 MiB page (`bPageTable`, `gp10x.c:87`, `gh10x.c:101`): the low word's VALID.
        if bits(lo, 0, 0) == 1 {
            out.push((va, 1 << PD0_LO, Xlate::Map(decode_pte(geo, lo, va, PD0_LO))));
            continue;
        }
        let big = decode_dual_big(geo, lo);
        let small = decode_dual_small(geo, hi);
        // ogkm `_gmmuIsInvalidPdeOk` (`mmu/gmmu_trace.c:430-466`): an invalid SPARSE big half
        // ends the translation of the whole 2 MiB — the small half is never read.
        if big == Pde::Sparse {
            out.push((va, 1 << PD0_LO, Xlate::Sparse));
            continue;
        }
        let big_tbl = match big {
            Pde::Table(ap, a) => match table_ok(mem, ap, a, 8 << (PD0_LO - BIG_LO)) {
                Ok(()) => Some(a),
                Err(why) => {
                    // The MMU reads the big PTE first; kayfabe cannot, so whether a big PTE owns a
                    // slot is unknown to it. It refuses the big half by name and still publishes the
                    // small table (PF-4, `V3_PTE_MODEL_FUZZ.md`): not decidable from the guest bytes
                    // kayfabe can read.
                    let small_readable = matches!(small, Pde::Table(sap, sa)
                        if table_ok(mem, sap, sa, 8 << (PD0_LO - SMALL_LO)).is_ok());
                    out.push((va, 1 << PD0_LO, if small_readable {
                        Xlate::Undecided("big half unreadable, small half readable (PF-4)")
                    } else {
                        Xlate::Unreadable(why)
                    }));
                    continue;
                }
            },
            _ => None,
        };
        let small_tbl = match small {
            Pde::Table(ap, a) => match table_ok(mem, ap, a, 8 << (PD0_LO - SMALL_LO)) {
                Ok(()) => Some(a),
                Err(why) => {
                    out.push((va, 1 << PD0_LO, Xlate::Unreadable(why)));
                    continue;
                }
            },
            _ => None,
        };
        for b in 0..(1u64 << (PD0_LO - BIG_LO)) {
            let bva = va | (b << BIG_LO);
            if let Some(t) = big_tbl {
                let e = mem.word(t + b * 8).unwrap_or(0);
                match pte_state(geo, e) {
                    // A valid big PTE owns its 64 KiB (`mmu_trace.c` tries sub-level 0 first and
                    // stops at the first OK translation).
                    Pte::Valid => {
                        out.push((bva, 1 << BIG_LO, Xlate::Map(decode_pte(geo, e, bva, BIG_LO))));
                        continue;
                    }
                    // `uvm_mmu.h:208-210`: no 4 KiB PTEs below an unmapped big PTE.
                    Pte::Unmapped => continue,
                    Pte::Invalid => {}
                    Pte::Sparse => {
                        out.push((bva, 1 << BIG_LO, Xlate::Undecided("sparse big PTE")));
                        continue;
                    }
                    Pte::Other => {
                        out.push((bva, 1 << BIG_LO, Xlate::Undecided("big PTE, invalid, stray bits")));
                        continue;
                    }
                }
            }
            // (A SPARSE small half alone marks nothing: only the big half's VOL marks a sparse dual
            // PDE, `kern_gmmu_gm200.c:57-69`.)
            let Some(t) = small_tbl else {
                continue;
            };
            for s in 0..(1u64 << (BIG_LO - SMALL_LO)) {
                let sva = bva | (s << SMALL_LO);
                let idx = (b << (BIG_LO - SMALL_LO)) | s;
                let e = mem.word(t + idx * 8).unwrap_or(0);
                match pte_state(geo, e) {
                    Pte::Valid => out.push((sva, 1 << SMALL_LO, Xlate::Map(decode_pte(geo, e, sva, SMALL_LO)))),
                    Pte::Sparse => out.push((sva, 1 << SMALL_LO, Xlate::Sparse)),
                    _ => {}
                }
            }
        }
    }
}

/// ★ The PTE KINDs a family defines, by NAME, and each one's uncompressed layout. Transcribed from
/// `sw/turing/tu102/dev_mmu.h:97-112` (TU10x, and GA10x/AD10x which include it;
/// `sw/ampere/ga100/dev_mmu.h:97-98` re-defines two with the same values),
/// `sw/hopper/gh100/dev_mmu.h:35-50` and `sw/blackwell/gb202/dev_mmu.h:26-41`: the same values on
/// every family. `INVALID` (0x07) and `SMSKED_MESSAGE` (0x0F, not a surface) are kept apart.
pub const KINDS: &[(u8, &str, u8)] = &[
    (0x00, "PITCH", 0x00),
    (0x01, "Z16", 0x01),
    (0x02, "S8", 0x02),
    (0x03, "S8Z24", 0x03),
    (0x04, "ZF32_X24S8", 0x04),
    (0x05, "Z24S8", 0x05),
    (0x06, "GENERIC_MEMORY", 0x06),
    (0x08, "GENERIC_MEMORY_COMPRESSIBLE", 0x06),
    (0x09, "GENERIC_MEMORY_COMPRESSIBLE_DISABLE_PLC", 0x06),
    (0x0A, "S8_COMPRESSIBLE_DISABLE_PLC", 0x02),
    (0x0B, "Z16_COMPRESSIBLE_DISABLE_PLC", 0x01),
    (0x0C, "S8Z24_COMPRESSIBLE_DISABLE_PLC", 0x03),
    (0x0D, "ZF32_X24S8_COMPRESSIBLE_DISABLE_PLC", 0x04),
    (0x0E, "Z24S8_COMPRESSIBLE_DISABLE_PLC", 0x05),
];
/// `NV_MMU_PTE_KIND_SMSKED_MESSAGE` (`tu102/dev_mmu.h:112`).
pub const KIND_SMSKED: u8 = 0x0F;

/// The uncompressed layout of a defined surface kind; `None` for INVALID, SMSKED or a value the
/// family does not define.
#[must_use]
pub fn uncompressed(kind: u8) -> Option<u8> {
    KINDS.iter().find(|k| k.0 == kind).map(|k| k.2)
}

/// Whether `kind` is a depth/stencil layout (Z16, S8, S8Z24, ZF32_X24S8, Z24S8 by name).
#[must_use]
pub fn is_depth(kind: u8) -> bool {
    uncompressed(kind).is_some_and(|u| (0x01..=0x05).contains(&u))
}
