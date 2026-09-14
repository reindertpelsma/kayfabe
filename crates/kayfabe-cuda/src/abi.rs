//! ★★★★★ **THE LAUNCH ABI OF `cuda/walk/kf_walk.cu`, MIRRORED** — and mirrored is the
//! operative word: every struct here is a second, independent statement of a layout whose
//! authority is the `.cu`.
//!
//! # ⊘⊘⊘ WHY THIS IS A MIRROR AND NOT A BINDING
//!
//! `kf_walk_kernel` takes `KfArgs` **by value**, and `cuLaunchKernel` passes a
//! by-value parameter as *"a pointer to its bytes"*. So the kernel's whole contract with
//! this crate is a **byte layout** — 216 bytes of it, which is what the committed PTX
//! declares (`.param .align 8 .b8 _Z14kf_walk_kernel6KfArgs_param_0[216]`).
//!
//! ⚠ **A mirror that is merely believed is the defect this tree keeps paying for.** So it
//! is not believed: `tests/walk_abi_matches_the_cu.rs` extracts the struct definitions from
//! `cuda/walk/kf_walk.cu` **by name**, compiles a `g++` program that prints every
//! `sizeof`/`offsetof`, and asserts it against these types field by field. That test needs
//! no GPU, no CUDA toolkit and no nvcc, so it runs wherever `cargo test` runs.
//!
//! ⊘ `bindgen` was the alternative and was rejected for a reason that is not taste: it would
//! make the C the *only* statement of the layout, and the failure this guards against —
//! a Rust/PTX skew — is then invisible until a kernel decodes garbage field offsets and it
//! looks like a page-table bug (`THE_CONSTRAINTS.md` §21). Two statements plus a
//! differential is what makes the skew *loud*.
//!
//! ★ And there is a second, independent guard **at launch**: [`KfFormat::abi_version`] is
//! checked against [`KF_ABI_VERSION`] before any launch, exactly as the `.cu`'s own
//! `kf_create` does. The offsets test catches a layout skew at build time; the version check
//! catches a *semantic* skew a layout test cannot see.

#![allow(clippy::unreadable_literal)]

/// Nesting slots for page directories. Five, because VER3 has one more directory level than
/// VER2. ⊘ Mirrors `KF_DIRS` in the `.cu`, where `make check-invariants` asserts it is a
/// `#define` rather than a descriptor field — that is what keeps invariant I1 structural.
pub const KF_DIRS: usize = 5;

/// The compile-time cap on any level's fan-out. Mirrors `KF_MAX_ENT`.
pub const KF_MAX_ENT: u32 = 512;

/// `KF_PS_NONE` — "a valid entry at this level is not a leaf".
pub const KF_PS_NONE: u8 = 0xFF;

/// Address spaces the kernel's own table can hold. Mirrors `KF_MAX_PDB`.
pub const KF_MAX_PDB: usize = 64;

/// Scopes one refresh may carry. Mirrors `KF_MAX_SCOPE`.
pub const KF_MAX_SCOPE: usize = 256;

/// ★★★ Bumped whenever the format descriptor's layout changes.
///
/// ⚠ A host/PTX skew must fail **loudly at launch** rather than decode garbage field offsets
/// and look like a page-table bug (`THE_CONSTRAINTS.md` §21). Mirrors `KF_ABI_VERSION`.
pub const KF_ABI_VERSION: u32 = 1;

/// Pascal…Ada. GA10x is the tested one.
pub const KF_TBL_VER2: u32 = 2;
/// Hopper/Blackwell — **sketched, never run**, and refused by this crate as by the `.cu`.
pub const KF_TBL_VER3: u32 = 3;

/// The report's magic. Mirrors `KFWR_MAGIC`.
///
/// ⚠ **`0x5257_464B`, not `0x4B46_5752`.** It spells `"KFWR"` as bytes in memory, so written
/// as a `u32` literal the characters appear REVERSED — and it was transcribed the other way
/// round, which a real boot caught: the kernel produced a perfectly good report
/// (`runs=1 entries=1796 refusals=0`) and this crate's own validator refused it as *"not a
/// walk report"*. ⊘ **The struct differential could not see this**, because a `#define` is
/// not a field; that is why `the_report_constants_match_the_header` exists beside it.
pub const KFWR_MAGIC: u32 = 0x5257_464B;

/// Report header flag: the walk was truncated, so it is **not** a delta and the walker must
/// not be acked.
pub const KFWR_HF_TRUNCATED: u16 = 1 << 0;
/// Report header flag: this report is a full resync rather than a delta.
pub const KFWR_HF_RESYNC: u16 = 1 << 1;

/// `value = ((raw >> lo) & ((1 << bits) - 1)) << shift`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct KfField {
    /// First bit of the field in the raw entry.
    pub lo: u8,
    /// Width of the field.
    pub bits: u8,
    /// Left shift applied to the extracted value.
    pub shift: u8,
    /// Explicit padding — present in the `.cu` and therefore present here.
    pub pad: u8,
}

/// One page-directory level's geometry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct KfDir {
    /// `0` ⇒ a pass-through: this format is shallower than `KF_DIRS`.
    pub active: u8,
    /// First VA bit this level indexes.
    pub va_lo: u8,
    /// `8`, or `16` for a dual entry.
    pub entry_bytes: u8,
    /// The page-size **code** a valid entry here means, or [`KF_PS_NONE`].
    pub leaf_ps: u8,
    /// `1 << (va_hi - va_lo + 1)`; `<= KF_MAX_ENT`.
    pub entries: u16,
    /// Explicit padding.
    pub pad: u16,
}

/// ★★★★★ **THE SETUP DATA** — `THE_CONSTRAINTS.md` §21: *"no bit position lives in the
/// kernel"*. The host derives this once per VM and hands it over; the kernel holds the
/// algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct KfFormat {
    /// Checked against [`KF_ABI_VERSION`] **at launch**, by name.
    pub abi_version: u32,
    /// [`KF_TBL_VER2`] or [`KF_TBL_VER3`].
    pub table_version: u32,
    /// Per-level geometry; `dir[KF_DIRS - 1]` is the dual level in both formats.
    pub dir: [KfDir; KF_DIRS],
    /// First VA bit of the big (64 KiB) leaf table.
    pub big_va_lo: u8,
    /// First VA bit of the small (4 KiB) leaf table.
    pub small_va_lo: u8,
    /// Entry width of the big leaf table.
    pub big_entry_bytes: u8,
    /// Entry width of the small leaf table.
    pub small_entry_bytes: u8,
    /// Entries in the big leaf table.
    pub big_entries: u16,
    /// Entries in the small leaf table.
    pub small_entries: u16,
    /// Page-size code a big leaf means.
    pub big_ps: u8,
    /// Page-size code a small leaf means.
    pub small_ps: u8,
    /// A page-directory base is a page address: this is its alignment.
    pub root_align: u32,
    /// The first active slot — where the root table sits.
    pub first_dir: u8,
    /// Explicit padding.
    pub pad0: [u8; 3],
    /// The "entry is valid" bit.
    pub valid_bit: u8,
    /// First bit of the aperture nibble.
    pub ap_lo: u8,
    /// Width of the aperture nibble.
    pub ap_bits: u8,
    /// The PDE aperture **value** meaning "no sub-level".
    pub pde_ap_invalid: u8,
    /// Raw nibble → `KFWR_AP_*`, for a leaf.
    pub pte_ap_map: [u8; 4],
    /// Raw nibble → `KFWR_AP_*`, for a directory.
    pub pde_ap_map: [u8; 4],
    /// Raw nibble → `0` = the local address spec, `1` = the sys spec.
    pub addr_sel: [u8; 4],
    /// A normal PDE/PTE target, and the dual entry's small half.
    pub addr_local: KfField,
    /// The same, for a system-memory target.
    pub addr_sys: KfField,
    /// The dual entry's big half (shift 8).
    pub big_addr_local: KfField,
    /// The same, for a system-memory target.
    pub big_addr_sys: KfField,
    /// Bit position of VOLATILE / uncached.
    pub bit_volatile: u8,
    /// Bit position of PRIVILEGE.
    pub bit_privilege: u8,
    /// Bit position of READ_ONLY.
    pub bit_read_only: u8,
    /// Bit position of ATOMIC_DISABLE.
    pub bit_atomic_disable: u8,
    /// VER3's PCF field. ⊘ Unused on VER2 — the one thing that is **not** a moved field.
    pub pcf: KfField,
    /// The PCF value meaning SPARSE (VER3).
    pub pcf_sparse: u8,
    /// Explicit padding.
    pub pad1: [u8; 3],
    /// Page-size code → `log2(bytes)`. ★ Carried in the report too, so the host's parser
    /// needs no format knowledge at all.
    pub ps_log2: [u8; 4],
}

/// ★★★ Invariant I2's window: the base and length of the buffer standing in for GPGA. The
/// kernel bounds-checks **every** dereference against this.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct KfWin {
    /// Device pointer to the buffer.
    pub base: u64,
    /// Its length in bytes.
    pub len: u64,
}

/// The kernel's cross-refresh state, in device memory.
///
/// ⊘ Mirrored in full because it is **written by the host at creation** (`cuMemcpyHtoD` of a
/// zeroed-but-configured image), exactly as the `.cu`'s `kf_create` does. Its tail is written
/// only by the kernel.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct KfDev {
    /// Monotonic refresh counter.
    pub generation: u64,
    /// The generation the host has acked.
    pub acked: u64,
    /// Whether an installed table exists to diff against.
    pub have_prev: u32,
    /// Which of the two tables is the installed one.
    pub cur_buf: u32,
    /// Slice size of the kernel's own table, per VAS.
    pub runs_per_pdb: u32,
    /// Entries one VAS's walk may examine.
    pub entry_budget: u32,
    /// Report run-array capacity.
    pub run_capacity: u32,
    /// Report `PdbEntry` capacity.
    pub pdb_capacity: u32,
    /// Address spaces the kernel's own table can hold.
    pub max_pdbs: u32,
    /// Per-buffer PDB counts.
    pub tbl_pdb_count: [u32; 2],
    /// Per-buffer PDB identities.
    pub tbl_pdb: [[u64; KF_MAX_PDB]; 2],
    /// Per-buffer per-PDB run counts.
    pub tbl_run_count: [[u32; KF_MAX_PDB]; 2],
    /// Accumulator, zeroed by `kf_begin_kernel`.
    pub entries_visited: u64,
    /// Accumulator, zeroed by `kf_begin_kernel`.
    pub refusals: u32,
    /// Accumulator, zeroed by `kf_begin_kernel`.
    pub refuse_mask: u32,
    /// Accumulator, zeroed by `kf_begin_kernel`.
    pub hdr_flags: u32,
    /// Accumulator, zeroed by `kf_begin_kernel`.
    pub walk_trunc: u32,
    /// Accumulator, zeroed by `kf_begin_kernel`.
    pub sparse_slots: u32,
}

impl Default for KfDev {
    fn default() -> Self {
        // ⊘ Written out rather than `mem::zeroed()`, and the reason is the whole of this
        // crate's containment rule: `zeroed` is `unsafe`, and `unsafe` in this workspace
        // lives in `*_unsafe.rs` files in audited crates only. A `#[repr(C)]` aggregate of
        // integers is exactly the case where the safe spelling costs nothing.
        // ⚠ Every field's zero IS its correct initial value; the fields the host must
        // configure are set by `WalkKernel::bring_up` immediately after.
        KfDev {
            generation: 0,
            acked: 0,
            have_prev: 0,
            cur_buf: 0,
            runs_per_pdb: 0,
            entry_budget: 0,
            run_capacity: 0,
            pdb_capacity: 0,
            max_pdbs: 0,
            tbl_pdb_count: [0; 2],
            tbl_pdb: [[0; KF_MAX_PDB]; 2],
            tbl_run_count: [[0; KF_MAX_PDB]; 2],
            entries_visited: 0,
            refusals: 0,
            refuse_mask: 0,
            hdr_flags: 0,
            walk_trunc: 0,
            sparse_slots: 0,
        }
    }
}

/// ★★★★★ **The kernel's one parameter, passed BY VALUE.** 216 bytes, and the committed PTX
/// says so in its own `.param` declaration.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct KfArgs {
    /// Invariant I2's bounds.
    pub win: KfWin,
    /// The setup data — immutable for the VM's lifetime.
    pub fmt: KfFormat,
    /// Device pointer to [`KfDev`].
    pub dev: u64,
    /// Device pointers to the kernel's two run tables.
    pub tbl: [u64; 2],
    /// Device pointer to the ascending PDB array.
    pub pdbs: u64,
    /// How many PDBs.
    pub npdb: u32,
    /// Device pointer to the scope array. Padding to 8 is explicit in the C layout and is
    /// reproduced by the field order, not by a manual `pad`.
    pub scopes: u64,
    /// How many scopes.
    pub nscope: u32,
    /// Device pointer to the report header.
    pub hdr: u64,
    /// Device pointer to the report's `PdbEntry` array.
    pub rpdb: u64,
    /// Device pointer to the report's run array.
    pub rrun: u64,
}

/// The report header. `KfReportHeader` in the `.cu`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct KfReportHeader {
    /// [`KFWR_MAGIC`].
    pub magic: u32,
    /// Report format version.
    pub version: u16,
    /// [`KFWR_HF_TRUNCATED`] / [`KFWR_HF_RESYNC`].
    pub flags: u16,
    /// This report's generation.
    pub generation: u64,
    /// The generation the host had acked when the walk ran.
    pub acked_generation: u64,
    /// Address spaces described.
    pub pdb_count: u32,
    /// Capacity of the `PdbEntry` array.
    pub pdb_capacity: u32,
    /// Runs described.
    pub run_count: u32,
    /// Capacity of the run array.
    pub run_capacity: u32,
    /// Table entries the walk examined.
    pub entries_visited: u64,
    /// How many refusals fired.
    pub refusals: u32,
    /// Which refusals fired — the bit mask that lets a test assert the refusal is *the one it
    /// intended* rather than merely that something was refused.
    pub refuse_mask: u32,
    /// Slots the guest **declared** empty, as opposed to never having written.
    pub sparse_slots: u32,
    /// Page-size code → `log2(bytes)`. ★ The report is self-describing, so this parser needs
    /// no format-version knowledge.
    pub ps_log2: [u8; 4],
}

/// One address space's slice of the run array.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct KfPdbEntry {
    /// The page-directory base this slice describes.
    pub pdb: u64,
    /// Index of its first run.
    pub first_run: u32,
    /// How many runs.
    pub run_count: u32,
    /// Per-VAS flags.
    pub vas_flags: u32,
    /// Reserved.
    pub reserved: u32,
    /// Reserved.
    pub reserved2: u64,
}

/// One coalesced mapping run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct KfMapRun {
    /// Guest virtual address.
    pub va: u64,
    /// The GPGA offset it maps to.
    pub gpga: u64,
    /// Length in bytes.
    pub len: u64,
    /// Aperture, page size and permission bits.
    pub flags: u32,
    /// `MAP` / `UNMAP` / …
    pub op: u16,
    /// Index into the `PdbEntry` array.
    pub pdb_index: u16,
}

/// `{pdb, va_base, va_len}`; `va_len == 0` means "walk this whole PDB".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct KfScope {
    /// The address space.
    pub pdb: u64,
    /// Where the hint starts.
    pub va_base: u64,
    /// How far it runs; `0` = the whole space.
    pub va_len: u64,
}

/// ★★★★★ **THE VER2 DESCRIPTOR** — Turing→Ada, the one this project has silicon for.
///
/// ⊘ **Transcribed from `cuda/walk/kf_walk.cu`'s `kf_format_ver2()`, and checked against it**
/// by `tests/walk_format_matches_the_cu.rs`, which extracts the `.cu`'s own function body and
/// compares every number. A transcription nobody checks is a second source of truth, which is
/// exactly what this descriptor exists to abolish one layer down.
///
/// ⚠ §21's eventual answer is that this is derived from `kayfabe_mmu`'s `GmmuFmt` impls rather
/// than transcribed. That is increment 6's, because it needs the walker wired into refresh to
/// mean anything; increment 4 only has to prove the kernel runs and answers correctly.
#[must_use]
pub fn kf_format_ver2() -> KfFormat {
    let mut f = KfFormat {
        abi_version: KF_ABI_VERSION,
        table_version: KF_TBL_VER2,
        dir: [KfDir::default(); KF_DIRS],
        big_va_lo: 16,
        small_va_lo: 12,
        big_entry_bytes: 8,
        small_entry_bytes: 8,
        big_entries: 32,
        small_entries: 512,
        big_ps: PS_64K,
        small_ps: PS_4K,
        root_align: 4096,
        first_dir: 1,
        pad0: [0; 3],
        valid_bit: 0,
        ap_lo: 1,
        ap_bits: 2,
        pde_ap_invalid: 0,
        pte_ap_map: [AP_VID, AP_PEER, AP_SYS, AP_SYS_NC],
        pde_ap_map: [AP_INVALID, AP_VID, AP_SYS, AP_SYS_NC],
        addr_sel: [0, 0, 1, 1],
        addr_local: KfField { lo: 8, bits: 25, shift: 12, pad: 0 },
        addr_sys: KfField { lo: 8, bits: 46, shift: 12, pad: 0 },
        big_addr_local: KfField { lo: 4, bits: 29, shift: 8, pad: 0 },
        big_addr_sys: KfField { lo: 4, bits: 50, shift: 8, pad: 0 },
        bit_volatile: 3,
        bit_privilege: 5,
        bit_read_only: 6,
        bit_atomic_disable: 7,
        pcf: KfField::default(),
        pcf_sparse: 0,
        pad1: [0; 3],
        ps_log2: [12, 16, 21, 29],
    };
    // ⊘ PD4 is inactive on VER2 — the slot exists only so VER3 needs no new nesting.
    // ⚠ `entries: 2`, not 1, and it is not a typo: the `.cu` fills every slot through
    // `kf_set_dir(d, active, va_lo, va_hi, …)`, which computes `1 << (va_hi - va_lo + 1)`,
    // and the inactive slot is declared `(0, 0)` ⇒ **2**. It is dead — `active == 0` makes
    // the level a pass-through — but the descriptor is compared BYTE FOR BYTE against the
    // `.cu`'s, so a "tidier" 1 here is a failing differential.
    f.dir[0] = KfDir { active: 0, va_lo: 0, entry_bytes: 8, leaf_ps: KF_PS_NONE, entries: 2, pad: 0 };
    f.dir[1] = KfDir { active: 1, va_lo: 47, entry_bytes: 8, leaf_ps: KF_PS_NONE, entries: 4, pad: 0 };
    f.dir[2] = KfDir { active: 1, va_lo: 38, entry_bytes: 8, leaf_ps: KF_PS_NONE, entries: 512, pad: 0 };
    // ★ PD1 is a leaf level too: a valid entry here is a **512 MiB** page. ⚠ Also transcribed
    // as `KF_PS_NONE` first — the second of two levels whose dual role is easy to miss, and
    // the reason the descriptor is compared byte for byte rather than spot-checked.
    f.dir[3] = KfDir { active: 1, va_lo: 29, entry_bytes: 8, leaf_ps: PS_512M, entries: 512, pad: 0 };
    // ★★ The DUAL level, and the one directory slot whose `leaf_ps` is NOT `KF_PS_NONE`: a
    // valid *non-dual* entry at PD0 on VER2 IS a 2 MiB page, so the level is both a directory
    // and a leaf. ⚠ Transcribed as `KF_PS_NONE` first, which would have made the kernel and
    // this descriptor disagree about whether 2 MiB pages exist at all.
    f.dir[4] = KfDir { active: 1, va_lo: 21, entry_bytes: 16, leaf_ps: PS_2M, entries: 256, pad: 0 };
    f
}

/// Aperture code: video memory.
pub const AP_VID: u8 = 0;
/// Aperture code: peer.
pub const AP_PEER: u8 = 1;
/// Aperture code: system, coherent.
pub const AP_SYS: u8 = 2;
/// Aperture code: system, non-coherent.
pub const AP_SYS_NC: u8 = 3;
/// ★★★ Aperture code meaning **"this directory entry names no sub-level"**.
///
/// ⊘⊘ `0xFF`, and it is **not** one more than the three real apertures. The `.cu` writes
/// `F.pde_ap_map[0] = 0xFF` — an out-of-range sentinel, so the kernel's
/// `pde_ap_map[apc] != KFWR_AP_VIDMEM` test rejects it along with every foreign aperture
/// rather than needing a fourth comparison.
///
/// ⚠ **This constant was transcribed as `4` and the descriptor differential caught it on its
/// first run** (`tests/walk_abi_matches_the_cu.rs`). `4` would have compared unequal to
/// `KFWR_AP_VIDMEM` too, so the kernel would have behaved identically — and the two
/// descriptors would have differed **silently, forever**, which is precisely the second
/// source of truth the setup-data seam exists to abolish.
pub const AP_INVALID: u8 = 0xFF;

/// Page-size code: 4 KiB.
pub const PS_4K: u8 = 0;
/// Page-size code: 64 KiB.
pub const PS_64K: u8 = 1;
/// Page-size code: 2 MiB.
pub const PS_2M: u8 = 2;
/// Page-size code: 512 MiB.
pub const PS_512M: u8 = 3;
