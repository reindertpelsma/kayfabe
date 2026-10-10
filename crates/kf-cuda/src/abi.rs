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
/// ★ 5 (v3-roperm): permission bits joined the diff key, selected per launch by
/// [`KfArgs::key_perm`] — a PTX built before it would keep a guest RW→RO downgrade as "same" while
/// this crate's model re-maps, and would read the field as padding.
///
/// ★ 6 (2026-10-09, refusal samples): [`KfReportHeader`] grew 64 → 392 bytes ([`KfRefusalSample`]
/// array) and [`KfDev`] gained the sample accumulator. [`KfFormat`] and [`KfArgs`] did not move,
/// so only this number (and the PTX's `.param`-independent semantics) tells a PTX built before it
/// — which writes a 64-byte header — from one that writes 392: the host would otherwise read
/// 328 bytes of the next region as samples.
pub const KF_ABI_VERSION: u32 = 6;

/// Refusal samples one walk's report carries. Mirrors `KF_REFUSAL_SAMPLES` in `kf_walk.h`.
pub const KF_REFUSAL_SAMPLES: usize = 8;
/// A [`KfRefusalSample::level`] naming the BIG (64 KiB) leaf-table entry; directory entries carry
/// the descriptor's `dir[]` index instead. Mirrors `KF_SAMPLE_LVL_BIG` (`== KF_DIRS`).
pub const KF_SAMPLE_LVL_BIG: u16 = 5;
/// A [`KfRefusalSample::level`] naming the SMALL (4 KiB) leaf-table entry. Mirrors
/// `KF_SAMPLE_LVL_SMALL` (`== KF_DIRS + 1`).
pub const KF_SAMPLE_LVL_SMALL: u16 = 6;
/// `KFWR_R_MISALIGNED_LEAF`: a leaf whose target is not aligned to its own page size.
pub const KFWR_R_MISALIGNED_LEAF: u32 = 1 << 10;
/// `KFWR_R_LEAF_OOB`: a leaf whose `[gpga, gpga+len)` leaves the GPGA window (or is PEER).
pub const KFWR_R_LEAF_OOB: u32 = 1 << 13;
/// `KFWR_R_OOB`: a table would leave the GPGA buffer.
pub const KFWR_R_OOB: u32 = 1 << 0;
/// `KFWR_R_UNALIGNED`: a table pointer is not naturally aligned.
pub const KFWR_R_UNALIGNED: u32 = 1 << 1;
/// `KFWR_R_FOREIGN_AP`: a table page is not in vidmem.
pub const KFWR_R_FOREIGN_AP: u32 = 1 << 2;

/// `KFWR_OP_UNMAP` — the run names a VA being RETIRED, so it carries no `gpga` and is exempt
/// from the §39(c) containment check. Mirrors `kf_walk.h:88`.
pub const KFWR_OP_UNMAP: u16 = 2;

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
/// Report header flag: a full resync. ⊘ Never set since the diff protocol (kept: the header's
/// bit is still defined).
pub const KFWR_HF_RESYNC: u16 = 1 << 1;
/// ★ Report header flag: the runs are each entry's DIFF against its slot's committed placements.
pub const KFWR_HF_DIFF: u16 = 1 << 7;
/// `KFWR_OP_MAP`.
pub const KFWR_OP_MAP: u16 = 1;
/// ★ A committed placement the host answered "already held" — its UNMAP is retired without asking
/// the host (P6b ruling (a)). Mirrors `KFWR_RF_HELD`.
pub const KFWR_RF_HELD: u32 = 1 << 31;
/// A leaf's `READ_ONLY` bit, decoded (never the raw PTE). Mirrors `KFWR_RF_READ_ONLY`.
pub const KFWR_RF_READ_ONLY: u32 = 1 << 3;
/// A leaf's `ATOMIC_DISABLE` bit (VER3: PCF `NO_ATOMIC`). Mirrors `KFWR_RF_ATOMIC_DISABLE`.
pub const KFWR_RF_ATOMIC_DISABLE: u32 = 1 << 4;
/// A leaf's `VOLATILE` bit (VER3: PCF `UNCACHED`). Mirrors `KFWR_RF_VOLATILE`.
pub const KFWR_RF_VOLATILE: u32 = 1 << 5;
/// A leaf's `PRIVILEGE` bit. ⊘ Not placeable on the host (RM takes it from the memory descriptor,
/// `ogkm-580 virt_mem_allocator_gm107.c:2849-2850`): a privileged leaf is WITHHELD from a user twin
/// instead (`kf_mem::apply`). Mirrors `KFWR_RF_PRIVILEGE`.
pub const KFWR_RF_PRIVILEGE: u32 = 1 << 6;
/// ★★★ v3-roperm: the permission bits that MAY join the diff key (`kf_hkey` /
/// [`crate::diffmodel::host_key_with`]); which of them do is the host's policy, passed per launch
/// in [`KfArgs::key_perm`]. Mirrors `KFWR_RF_KEY_PERM_ALL`.
pub const KFWR_RF_KEY_PERM_ALL: u32 =
    KFWR_RF_READ_ONLY | KFWR_RF_ATOMIC_DISABLE | KFWR_RF_VOLATILE | KFWR_RF_PRIVILEGE;
/// ★★★ v3-roperm: the default key — what the host carries by default (read-only, volatile) plus
/// PRIVILEGE (whose flip re-decides whether a user twin may hold the leaf at all). ATOMIC_DISABLE
/// joins only with `KF3_CARRY_ATOMIC_DISABLE=1` (`kf_mem::apply::PermPolicy`). Mirrors
/// `KFWR_RF_KEY_PERM_DEFAULT`.
pub const KFWR_RF_KEY_PERM_DEFAULT: u32 = KFWR_RF_READ_ONLY | KFWR_RF_VOLATILE | KFWR_RF_PRIVILEGE;
/// `KFWR_R_RUN_CAP`: out of run capacity (a walk region, a slot, or the report).
pub const KFWR_R_RUN_CAP: u32 = 1 << 4;
/// `KFWR_R_BUDGET`: the walk's entry budget stopped it.
pub const KFWR_R_BUDGET: u32 = 1 << 5;
/// `KFWR_R_PDB_CAP`: more address spaces than the report's `PdbEntry` capacity.
pub const KFWR_R_PDB_CAP: u32 = 1 << 6;
/// `KFWR_R_FRONTIER_CAP`: the parallel walk's frontier/stage ran out.
pub const KFWR_R_FRONTIER_CAP: u32 = 1 << 12;
/// Per-entry flag: the entry's MAPs were withheld (slot capacity); re-walk after its UNMAPs.
pub const KFWR_V_PARTIAL: u32 = 1 << 3;
/// Per-entry flag: the slot is full and nothing can be retired; no runs.
pub const KFWR_V_OVERFLOW: u32 = 1 << 4;
/// ★ Per-entry flag: the entry's walk refused something (`KfPdbEntry::reserved2` = the
/// `KFWR_R_*` bits); its refused leaves are absent from the walk (owner ruling 2026-09-25).
pub const KFWR_V_REFUSED: u32 = 1 << 5;

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
    /// ★★★ **The PTE's KIND, which joins RUN IDENTITY** — added to the `.cu` by §w725b
    /// (*"run identity must include KIND — don't coalesce across a field you don't
    /// propagate"*). VER2 spells it 63:56, VER3 spells it 11:8; a **moved field**, so the
    /// descriptor carries it and no switch arm is needed.
    ///
    /// ⚠ **This field is why the ABI differential exists.** It was added to `kf_walk.cu` by
    /// someone else, and this mirror did not move: `KfFormat` grew 116 → 120 bytes and every
    /// field after offset 112 would have been read at the wrong place. The test named the
    /// struct, the size and the byte.
    pub kind: KfField,
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
    /// How many bytes of it are MAPPED, and so how far a table read may reach.
    pub len: u64,
    /// ★★★★★ §39(c): how large the guest's GPGA space is, and so how far a LEAF
    /// may point. Distinct from [`Self::len`]: in production the single store is
    /// the whole of guest vidmem and both are the store length, but a captured
    /// corpus image holds table pages and no framebuffer, so its leaves point
    /// legitimately outside the bytes it contains. `0` refuses every leaf.
    pub span: u64,
}

/// The kernel's cross-refresh state, in device memory.
///
/// ⊘ Mirrored in full because it is **written by the host at creation** (`cuMemcpyHtoD` of a
/// zeroed-but-configured image), exactly as the `.cu`'s `kf_create` does. Its tail is written
/// only by the kernel. ⊘ It holds no snapshot of the guest's tables: what persists across walks
/// is the committed placements ([`KfArgs::com`], per slot, written only on the host's ack).
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct KfDev {
    /// Reports emitted.
    pub generation: u64,
    /// The last report generation whose ack was committed.
    pub committed: u64,
    /// The walk's slice per entry, and one slot's capacity.
    pub runs_per_pdb: u32,
    /// Entries one VAS's walk may examine.
    pub entry_budget: u32,
    /// Report run-array capacity.
    pub run_capacity: u32,
    /// Report `PdbEntry` capacity.
    pub pdb_capacity: u32,
    /// Walk entries.
    pub max_pdbs: u32,
    /// Committed-placement slots.
    pub max_slots: u32,
    /// The walk's runs, per entry.
    pub tbl_run_count: [u32; KF_MAX_PDB],
    /// The staged diff's runs, per entry.
    pub diff_count: [u32; KF_MAX_PDB],
    /// `KFWR_V_PARTIAL` / `KFWR_V_OVERFLOW`, per entry.
    pub diff_vflags: [u32; KF_MAX_PDB],
    /// ★ Which refusals fired in each entry's walk (the host fails that space by name).
    pub entry_refuse: [u32; KF_MAX_PDB],
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
    /// ★ **The walk itself stopped** — budget or frontier cap.
    pub walk_abort: u32,
    /// Accumulator, zeroed by `kf_begin_kernel`.
    pub sparse_slots: u32,
    /// ★ w829: per entry, the capacity it NEEDED (the walk's uncapped run count, or a diff's
    /// placements + maps when its slot could not hold them).
    pub need: [u32; KF_MAX_PDB],
    /// ★ ABI 6: the refusal-sample ticket counter, zeroed by `kf_begin_kernel`.
    pub nsample: u32,
    /// Explicit padding: [`KfRefusalSample`] is 8-aligned.
    pub sample_pad: u32,
    /// ★ ABI 6: the samples; only the kernel writes them.
    pub sample: [KfRefusalSample; KF_REFUSAL_SAMPLES],
}

impl Default for KfDev {
    fn default() -> Self {
        // ⊘ Written out rather than `mem::zeroed()`: `zeroed` is `unsafe`, and `unsafe` in this
        // crate lives in `driver_unsafe.rs`. Every field's zero IS its correct initial value; the
        // fields the host must configure are set by `WalkKernel::bring_up` immediately after.
        KfDev {
            generation: 0,
            committed: 0,
            runs_per_pdb: 0,
            entry_budget: 0,
            run_capacity: 0,
            pdb_capacity: 0,
            max_pdbs: 0,
            max_slots: 0,
            tbl_run_count: [0; KF_MAX_PDB],
            diff_count: [0; KF_MAX_PDB],
            diff_vflags: [0; KF_MAX_PDB],
            entry_refuse: [0; KF_MAX_PDB],
            entries_visited: 0,
            refusals: 0,
            refuse_mask: 0,
            hdr_flags: 0,
            walk_trunc: 0,
            walk_abort: 0,
            sparse_slots: 0,
            need: [0; KF_MAX_PDB],
            nsample: 0,
            sample_pad: 0,
            sample: [KfRefusalSample::default(); KF_REFUSAL_SAMPLES],
        }
    }
}

/// ★ One slot of committed placements: its count per page-size class (`KfSlot` in `kf_walk.h`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct KfSlot {
    /// Placements per class; class 0's come first in the slot's run array.
    pub n: [u32; 4],
}

/// Committed-placement slots a [`KfLayout`] describes. Mirrors `KF_MAX_SLOTS`.
pub const KF_MAX_SLOTS: usize = 128;

/// ★★★★★ **The capacity layout** (`KfLayout` in `kf_walk.h`, w829) — per walk entry, its region
/// of the walk pool (and so of the scratch); the previous walk's (the commit's scratch); per slot,
/// its region of the committed-placement pool. In run units. Written by the host into pinned
/// memory before each submit; read in place by the kernels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct KfLayout {
    /// This walk's per-entry table offset.
    pub walk_off: [u32; KF_MAX_PDB],
    /// This walk's per-entry table capacity.
    pub walk_cap: [u32; KF_MAX_PDB],
    /// The previous walk's `walk_off`.
    pub prev_off: [u32; KF_MAX_PDB],
    /// The previous walk's `walk_cap`.
    pub prev_cap: [u32; KF_MAX_PDB],
    /// Per slot: its committed-placement region's offset.
    pub slot_off: [u32; KF_MAX_SLOTS],
    /// Per slot: its capacity (`0` = no region yet).
    pub slot_cap: [u32; KF_MAX_SLOTS],
}

impl Default for KfLayout {
    fn default() -> Self {
        KfLayout {
            walk_off: [0; KF_MAX_PDB],
            walk_cap: [0; KF_MAX_PDB],
            prev_off: [0; KF_MAX_PDB],
            prev_cap: [0; KF_MAX_PDB],
            slot_off: [0; KF_MAX_SLOTS],
            slot_cap: [0; KF_MAX_SLOTS],
        }
    }
}

/// Slots one verdict may empty. Mirrors `KF_MAX_RESET`.
pub const KF_MAX_RESET: usize = 64;

/// ★★★★★ **The host's verdict on one report** (`KfAck` in `kf_walk.h`) — COMMIT-ON-ACK. The codes
/// (one byte per report run, [`KFWR_ACK_FAILED`] …) are a separate array.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct KfAck {
    /// The report this answers; `0` = no verdict.
    pub generation: u64,
    /// Must equal that report's `run_count`.
    pub nrun: u32,
    /// Slots to empty (their object is gone).
    pub nreset: u32,
    /// The slots.
    pub reset: [u32; KF_MAX_RESET],
}

impl Default for KfAck {
    fn default() -> Self {
        KfAck {
            generation: 0,
            nrun: 0,
            nreset: 0,
            reset: [0; KF_MAX_RESET],
        }
    }
}

/// Not applied: commit nothing for this run.
pub const KFWR_ACK_FAILED: u8 = 0;
/// Applied.
pub const KFWR_ACK_APPLIED: u8 = 1;
/// A MAP the host already held — satisfied, not ours.
pub const KFWR_ACK_HELD: u8 = 2;

/// ★★★★★ **The kernel's one parameter, passed BY VALUE**, and the committed PTX says how many
/// bytes it is in its own `.param` declaration.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct KfArgs {
    /// Invariant I2's bounds.
    pub win: KfWin,
    /// The setup data — immutable for the VM's lifetime.
    pub fmt: KfFormat,
    /// Device pointer to [`KfDev`].
    pub dev: u64,
    /// The walk's runs: `runs_per_pdb` per entry.
    pub walk: u64,
    /// The committed placements: `runs_per_pdb` per slot.
    pub com: u64,
    /// [`KfSlot`] per slot.
    pub slot: u64,
    /// Per entry: the root walked.
    pub pdbs: u64,
    /// Per entry: the slot it is diffed against.
    pub slots: u64,
    /// How many entries.
    pub npdb: u32,
    /// ★ v3-roperm: the permission bits that join the diff key — a subset of
    /// [`KFWR_RF_KEY_PERM_ALL`], the host's policy. Occupies what was padding after `npdb`.
    pub key_perm: u32,
    /// The host's verdict on the PREVIOUS report ([`KfAck`]); `0` = none.
    pub ack: u64,
    /// One `KFWR_ACK_*` byte per previous report run.
    pub ack_code: u64,
    /// `4 * runs_per_pdb` runs of scratch per entry.
    pub scratch: u64,
    /// `3 * runs_per_pdb` words of scratch per entry.
    pub iscratch: u64,
    /// Device pointer to the report header.
    pub hdr: u64,
    /// Device pointer to the report's `PdbEntry` array.
    pub rpdb: u64,
    /// Device pointer to the report's run array.
    pub rrun: u64,
    /// ★ w829: device pointer to the [`KfLayout`] (host-managed capacity).
    pub lay: u64,
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
    /// ★ ABI 6: how many of [`Self::samples`] are valid (`<=` [`KF_REFUSAL_SAMPLES`] as the kernel
    /// writes it; ALWAYS read through [`KfReportHeader::refusal_samples`], which clamps again).
    pub sample_count: u32,
    /// ★ ABI 6: how many sampleable refusals (those naming a guest entry) the walk offered. May
    /// exceed the cap; is not `refusals`, which also counts the unsampled kinds.
    pub sample_total: u32,
    /// ★ ABI 6: the first [`KF_REFUSAL_SAMPLES`] refusals that named a guest entry. OBSERVATION
    /// ONLY: nothing acts on these.
    pub samples: [KfRefusalSample; KF_REFUSAL_SAMPLES],
}

/// ★★★ **One refusal, as the kernel saw it** (`KfRefusalSample` in `kf_walk.h`, ABI 6) — what a
/// count and a mask cannot say. Every field is a value the kernel held in a register when it
/// refused: `raw` is the 64-bit entry as read (once), `gpga` what the format decoded from it,
/// `ps_bytes` what that had to be aligned to, `bit` the `KFWR_R_*` it set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct KfRefusalSample {
    /// The entry's VA (the base of the page it spells, or of the table it points at).
    pub va: u64,
    /// The raw 64-bit entry, as read.
    pub raw: u64,
    /// The target the format decoded from `raw`.
    pub gpga: u64,
    /// What `gpga` was required to be aligned to (leaf page size; a table's size).
    pub ps_bytes: u64,
    /// The `KFWR_R_*` bit this refusal set.
    pub bit: u32,
    /// The descriptor's `dir[]` index, or [`KF_SAMPLE_LVL_BIG`] / [`KF_SAMPLE_LVL_SMALL`].
    pub level: u16,
    /// The walk entry (address space) index — the `KfPdbEntry` index in the report.
    pub entry: u16,
}

impl KfReportHeader {
    /// The valid samples, CLAMPED to the array here as well as by the kernel: a count the device
    /// wrote is guest-influenced state's neighbour, and a slice out of range would panic the
    /// thread that collects.
    #[must_use]
    pub fn refusal_samples(&self) -> &[KfRefusalSample] {
        let n = (self.sample_count as usize).min(KF_REFUSAL_SAMPLES);
        &self.samples[..n]
    }
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

impl KfPdbEntry {
    /// ★ The committed-placement SLOT this entry was diffed against — `reserved`, which the
    /// kernel's emit writes as `a.slots[t]` and its commit reads back (`kf_walk.cu`,
    /// `kf_diff_emit` / `kf_commit_kernel`). ⊘ The C field keeps its name: a frozen seam compiles
    /// against it.
    #[must_use]
    pub fn slot(&self) -> u32 {
        self.reserved
    }

    /// The `KFWR_R_*` bits this entry's walk refused (`reserved2` low 32).
    #[must_use]
    pub fn refused_bits(&self) -> u32 {
        (self.reserved2 & 0xFFFF_FFFF) as u32
    }

    /// ★ w829: the capacity this entry NEEDED (`reserved2` high 32) — the walk's uncapped run
    /// count, or placements + maps when the diff could not fit its slot.
    #[must_use]
    pub fn need(&self) -> u32 {
        (self.reserved2 >> 32) as u32
    }
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

/// `KFWR_RF_AP_*` sys-coherent (`cuda/walk/kf_walk.h:109`): the run's `gpga` is guest-PHYSICAL.
pub const KFWR_AP_SYS_COHERENT: u8 = 2;
/// `KFWR_RF_AP_*` sys-noncoherent: the run's `gpga` is guest-PHYSICAL.
pub const KFWR_AP_SYS_NONCOHERENT: u8 = 3;

// ── KfMapRun::flags — the DECODED fields, never the raw entry (`kf_walk.h`) ───────────────────
//
// ★ Mirrors of the header's `KFWR_RF_*_{SHIFT,MASK}` pairs, by the header's own names, and pinned
// against it by `tests/walk_abi_matches_the_cu.rs` (`the_report_constants_match_the_header`). The
// kernel builds `flags` in `kf_leaf_flags` from these and nothing else; every Rust reader goes
// through [`RfField`] rather than restating a shift.

/// `KFWR_RF_AP_SHIFT`: the leaf aperture code's first bit.
pub const KFWR_RF_AP_SHIFT: u32 = 0;
/// `KFWR_RF_AP_MASK`: the leaf aperture code, after the shift (0 vidmem, 1 peer, 2 sys-coherent,
/// 3 sys-noncoherent).
pub const KFWR_RF_AP_MASK: u32 = 0x7;
/// `KFWR_RF_PS_SHIFT`: the page-size code's first bit.
pub const KFWR_RF_PS_SHIFT: u32 = 8;
/// `KFWR_RF_PS_MASK`: the page-size code ([`PS_4K`] … [`PS_512M`]), after the shift.
pub const KFWR_RF_PS_MASK: u32 = 0xF;
/// `KFWR_RF_KIND_SHIFT`: the guest PTE's KIND, which joins run identity (`kf_walk.h` §w725b).
pub const KFWR_RF_KIND_SHIFT: u32 = 16;
/// `KFWR_RF_KIND_MASK`: the KIND, after the shift.
pub const KFWR_RF_KIND_MASK: u32 = 0xFF;
/// ★ The page-size CLASS mask: the kernel's `kf_pcls` and `kf_ps_bytes_of` read the page-size
/// field as `(flags >> KFWR_RF_PS_SHIFT) & 3u` — the four codes a format has — not through
/// [`KFWR_RF_PS_MASK`]. ⊘ The header names no macro for the `3u`, so this is not a `KFWR_*`
/// mirror; it is kept exactly (a code above 3 would alias a class, as it does on the GPU).
pub const KF_PS_CLASS_MASK: u32 = 3;

/// ★ One named bit range of [`KfMapRun::flags`]: `value = (flags >> shift) & mask` — a
/// `KFWR_RF_*_{SHIFT,MASK}` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RfField {
    /// The field's first bit (`KFWR_RF_*_SHIFT`).
    pub shift: u32,
    /// Its mask, after the shift (`KFWR_RF_*_MASK`).
    pub mask: u32,
}

impl RfField {
    /// The field's value in `flags`.
    #[must_use]
    pub const fn get(self, flags: u32) -> u32 {
        (flags >> self.shift) & self.mask
    }

    /// `value` placed in the field (masked), to be OR'd into a flags word.
    #[must_use]
    pub const fn put(self, value: u32) -> u32 {
        (value & self.mask) << self.shift
    }

    /// The field's bits, in place.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.mask << self.shift
    }
}

/// The leaf aperture code (`KFWR_RF_AP_*`).
pub const RF_AP: RfField = RfField {
    shift: KFWR_RF_AP_SHIFT,
    mask: KFWR_RF_AP_MASK,
};
/// The page-size code (`KFWR_RF_PS_*`).
pub const RF_PS: RfField = RfField {
    shift: KFWR_RF_PS_SHIFT,
    mask: KFWR_RF_PS_MASK,
};
/// The page-size CLASS as the kernel reads it (`kf_pcls`: `KFWR_RF_PS_SHIFT`, [`KF_PS_CLASS_MASK`]).
pub const RF_CLASS: RfField = RfField {
    shift: KFWR_RF_PS_SHIFT,
    mask: KF_PS_CLASS_MASK,
};
/// The guest PTE's KIND (`KFWR_RF_KIND_*`).
pub const RF_KIND: RfField = RfField {
    shift: KFWR_RF_KIND_SHIFT,
    mask: KFWR_RF_KIND_MASK,
};

impl KfMapRun {
    /// The leaf aperture code: `flags` bits `KFWR_RF_AP_SHIFT`/`KFWR_RF_AP_MASK`
    /// (`cuda/walk/kf_walk.h:92-97` — 0 vidmem, 1 peer, 2 sys-coherent, 3 sys-noncoherent).
    #[must_use]
    pub fn aperture(&self) -> u8 {
        RF_AP.get(self.flags) as u8
    }

    /// The page-size code ([`PS_4K`] … [`PS_512M`]): `KFWR_RF_PS_SHIFT`/`KFWR_RF_PS_MASK`.
    #[must_use]
    pub fn page_size(&self) -> u8 {
        RF_PS.get(self.flags) as u8
    }

    /// The page-size class the diff groups by, exactly as the kernel's `kf_pcls` reads it
    /// ([`RF_CLASS`]).
    #[must_use]
    pub fn class(&self) -> usize {
        RF_CLASS.get(self.flags) as usize
    }

    /// The guest PTE's KIND: `KFWR_RF_KIND_SHIFT`/`KFWR_RF_KIND_MASK`.
    #[must_use]
    pub fn kind(&self) -> u8 {
        RF_KIND.get(self.flags) as u8
    }
}

// ── The report, decoded: ONE decoder per `#[repr(C)]` struct ─────────────────────────────────
//
// ★★★ `STATUS_AND_HANDOFF.md` §4 item 6. The report used to be decoded two ways — a raw
// `read_struct` copy in `try_collect` and hand-written byte offsets (`c[24..28]`, `c[28..30]`, …)
// in `debug_walk_runs` — so a field move in `kf_walk.h` would have been caught by the layout
// differential for one path and read at the wrong offset by the other. Now every report struct
// has exactly one decoder, generated from its field list at `offset_of!`, in safe code:
// - the field list is a STRUCT LITERAL, so a field added to the struct and not to the list is a
//   compile error, never a silently-zero field;
// - a `const` assertion proves the fields TILE the struct (no padding a decoder would skip, no
//   byte `encode` leaves unwritten), which is what makes `encode ∘ decode` the identity on bytes;
// - the report is little-endian by the header's contract (*"little-endian, naturally aligned,
//   no bitfields"*), so it is decoded as little-endian whatever the host.
// ⊘ `offset_of!` is this struct's layout as the Rust compiler lays it out; that it is also the C
// compiler's is `tests/walk_abi_matches_the_cu.rs`'s job, field by field.

/// A fixed-width little-endian field of a report struct.
trait LeField: Sized {
    fn get(b: &[u8], at: usize) -> Self;
    fn put(&self, b: &mut [u8], at: usize);
}

macro_rules! le_int_field {
    ($($t:ty),+) => {$(
        impl LeField for $t {
            fn get(b: &[u8], at: usize) -> Self {
                let mut w = [0u8; core::mem::size_of::<$t>()];
                w.copy_from_slice(&b[at..at + core::mem::size_of::<$t>()]);
                <$t>::from_le_bytes(w)
            }
            fn put(&self, b: &mut [u8], at: usize) {
                b[at..at + core::mem::size_of::<$t>()].copy_from_slice(&self.to_le_bytes());
            }
        }
    )+};
}
le_int_field!(u16, u32, u64);

impl<const N: usize> LeField for [u8; N] {
    fn get(b: &[u8], at: usize) -> Self {
        let mut w = [0u8; N];
        w.copy_from_slice(&b[at..at + N]);
        w
    }
    fn put(&self, b: &mut [u8], at: usize) {
        b[at..at + N].copy_from_slice(self);
    }
}

impl LeField for KfRefusalSample {
    fn get(b: &[u8], at: usize) -> Self {
        KfRefusalSample::decode(&b[at..at + KfRefusalSample::BYTES])
    }
    fn put(&self, b: &mut [u8], at: usize) {
        b[at..at + KfRefusalSample::BYTES].copy_from_slice(&self.encode());
    }
}

impl<const N: usize> LeField for [KfRefusalSample; N] {
    fn get(b: &[u8], at: usize) -> Self {
        core::array::from_fn(|i| {
            <KfRefusalSample as LeField>::get(b, at + i * KfRefusalSample::BYTES)
        })
    }
    fn put(&self, b: &mut [u8], at: usize) {
        for (i, s) in self.iter().enumerate() {
            LeField::put(s, b, at + i * KfRefusalSample::BYTES);
        }
    }
}

/// Generate a report struct's ONE decoder (and its inverse) from its field list.
macro_rules! report_codec {
    ($t:ident { $($f:ident: $ty:ty),+ $(,)? }) => {
        impl $t {
            /// Bytes in the report: `size_of`, which the layout differential pins to the C
            /// compiler's `sizeof`.
            pub const BYTES: usize = core::mem::size_of::<$t>();

            /// ★ Decode one from the bytes the device wrote — little-endian, every field at its
            /// `offset_of!`.
            ///
            /// # Panics
            /// If `b` is shorter than [`Self::BYTES`]. ⊘ A panic and not a truncation: a short
            /// read would decode whatever follows in the buffer — usually zeros — with no marker
            /// distinguishing it from a real value.
            #[must_use]
            pub fn decode(b: &[u8]) -> Self {
                assert!(
                    b.len() >= Self::BYTES,
                    "a {}-byte buffer cannot hold a {}-byte {}",
                    b.len(),
                    Self::BYTES,
                    stringify!($t)
                );
                $t { $($f: <$ty as LeField>::get(b, core::mem::offset_of!($t, $f)),)+ }
            }

            /// ★ Decode an array of them — every WHOLE struct in `b`, in order (a trailing
            /// partial one is not a struct and is not read).
            #[must_use]
            pub fn decode_all(b: &[u8]) -> Vec<Self> {
                let (whole, _partial) = b.as_chunks::<{ core::mem::size_of::<$t>() }>();
                whole.iter().map(|c| Self::decode(c)).collect()
            }

            /// The bytes [`Self::decode`] reads this value back from (little-endian, every
            /// field at its `offset_of!`) — for a test that must hand the host a report.
            #[must_use]
            pub fn encode(&self) -> [u8; core::mem::size_of::<$t>()] {
                let mut b = [0u8; core::mem::size_of::<$t>()];
                $(LeField::put(&self.$f, &mut b, core::mem::offset_of!($t, $f));)+
                b
            }
        }
        // The fields TILE the struct: their widths sum to its size, so there is no padding.
        const _: () = assert!(0 $(+ core::mem::size_of::<$ty>())+ == core::mem::size_of::<$t>());
    };
}

report_codec!(KfReportHeader {
    magic: u32,
    version: u16,
    flags: u16,
    generation: u64,
    acked_generation: u64,
    pdb_count: u32,
    pdb_capacity: u32,
    run_count: u32,
    run_capacity: u32,
    entries_visited: u64,
    refusals: u32,
    refuse_mask: u32,
    sparse_slots: u32,
    ps_log2: [u8; 4],
    sample_count: u32,
    sample_total: u32,
    samples: [KfRefusalSample; KF_REFUSAL_SAMPLES],
});

report_codec!(KfRefusalSample {
    va: u64,
    raw: u64,
    gpga: u64,
    ps_bytes: u64,
    bit: u32,
    level: u16,
    entry: u16,
});

report_codec!(KfPdbEntry {
    pdb: u64,
    first_run: u32,
    run_count: u32,
    vas_flags: u32,
    reserved: u32,
    reserved2: u64,
});

report_codec!(KfMapRun {
    va: u64,
    gpga: u64,
    len: u64,
    flags: u32,
    op: u16,
    pdb_index: u16,
});

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
    // ★★★ **ZEROED FIRST, AS THE `.cu` DOES** (`memset(&F, 0, sizeof(F))`). A Rust struct
    // literal leaves interior PADDING undefined, and this value's bytes are handed to
    // `cuLaunchKernel` verbatim — so the padding is part of the ABI whether we name it or not.
    // ⊘ The descriptor differential caught this at byte 58, which is padding.
    let mut f: KfFormat = crate::driver_unsafe::zeroed();
    // ⊘ FIELD BY FIELD, not a struct literal: a literal produces a FRESH value whose
    // padding is undefined again, which is what the first attempt at this fix did and
    // why the differential still failed at byte 58. Assigning into the zeroed value
    // leaves the padding alone.
    f.abi_version = KF_ABI_VERSION;
    f.table_version = KF_TBL_VER2;
    f.dir = [KfDir::default(); KF_DIRS];
    f.big_va_lo = 16;
    f.small_va_lo = 12;
    f.big_entry_bytes = 8;
    f.small_entry_bytes = 8;
    f.big_entries = 32;
    f.small_entries = 512;
    f.big_ps = PS_64K;
    f.small_ps = PS_4K;
    f.root_align = 4096;
    f.first_dir = 1;
    f.pad0 = [0; 3];
    f.valid_bit = 0;
    f.ap_lo = 1;
    f.ap_bits = 2;
    f.pde_ap_invalid = 0;
    f.pte_ap_map = [AP_VID, AP_PEER, AP_SYS, AP_SYS_NC];
    f.pde_ap_map = [AP_INVALID, AP_VID, AP_SYS, AP_SYS_NC];
    f.addr_sel = [0, 0, 1, 1];
    f.addr_local = KfField {
        lo: 8,
        bits: 25,
        shift: 12,
        pad: 0,
    };
    f.addr_sys = KfField {
        lo: 8,
        bits: 46,
        shift: 12,
        pad: 0,
    };
    f.big_addr_local = KfField {
        lo: 4,
        bits: 29,
        shift: 8,
        pad: 0,
    };
    f.big_addr_sys = KfField {
        lo: 4,
        bits: 50,
        shift: 8,
        pad: 0,
    };
    f.bit_volatile = 3;
    f.bit_privilege = 5;
    f.bit_read_only = 6;
    f.bit_atomic_disable = 7;
    f.pcf = KfField::default();
    f.pcf_sparse = 0;
    f.pad1 = [0; 3];
    // ★ VER2's KIND is 63:56, shift 0 — read from the `.cu`'s own `kf_format_ver2`, and
    // pinned byte-for-byte by the descriptor differential.
    f.kind = KfField {
        lo: 56,
        bits: 8,
        shift: 0,
        pad: 0,
    };
    f.ps_log2 = [12, 16, 21, 29];
    // ⊘ PD4 is inactive on VER2 — the slot exists only so VER3 needs no new nesting.
    // ⚠ `entries: 2`, not 1, and it is not a typo: the `.cu` fills every slot through
    // `kf_set_dir(d, active, va_lo, va_hi, …)`, which computes `1 << (va_hi - va_lo + 1)`,
    // and the inactive slot is declared `(0, 0)` ⇒ **2**. It is dead — `active == 0` makes
    // the level a pass-through — but the descriptor is compared BYTE FOR BYTE against the
    // `.cu`'s, so a "tidier" 1 here is a failing differential.
    f.dir[0] = KfDir {
        active: 0,
        va_lo: 0,
        entry_bytes: 8,
        leaf_ps: KF_PS_NONE,
        entries: 2,
        pad: 0,
    };
    f.dir[1] = KfDir {
        active: 1,
        va_lo: 47,
        entry_bytes: 8,
        leaf_ps: KF_PS_NONE,
        entries: 4,
        pad: 0,
    };
    f.dir[2] = KfDir {
        active: 1,
        va_lo: 38,
        entry_bytes: 8,
        leaf_ps: KF_PS_NONE,
        entries: 512,
        pad: 0,
    };
    // ★ PD1 is a leaf level too: a valid entry here is a **512 MiB** page. ⚠ Also transcribed
    // as `KF_PS_NONE` first — the second of two levels whose dual role is easy to miss, and
    // the reason the descriptor is compared byte for byte rather than spot-checked.
    f.dir[3] = KfDir {
        active: 1,
        va_lo: 29,
        entry_bytes: 8,
        leaf_ps: PS_512M,
        entries: 512,
        pad: 0,
    };
    // ★★ The DUAL level, and the one directory slot whose `leaf_ps` is NOT `KF_PS_NONE`: a
    // valid *non-dual* entry at PD0 on VER2 IS a 2 MiB page, so the level is both a directory
    // and a leaf. ⚠ Transcribed as `KF_PS_NONE` first, which would have made the kernel and
    // this descriptor disagree about whether 2 MiB pages exist at all.
    f.dir[4] = KfDir {
        active: 1,
        va_lo: 21,
        entry_bytes: 16,
        leaf_ps: PS_2M,
        entries: 256,
        pad: 0,
    };
    f
}

/// ★ The VER3 (Hopper, Blackwell) descriptor — a hand transcription of `kf_walk.cu`'s own
/// `kf_format_ver3_untested()`, pinned byte for byte by the descriptor differential exactly as VER2
/// is. Every field was re-checked against `ogkm-580 hopper/gh100/dev_mmu.h:53-187` and the level
/// geometry against `kern_gmmu_fmt_gh10x.c:53-114` (w826): PD4 `56:56` (2 entries) → PD3 `55:47` →
/// PD2 `46:38` → PD1 `37:29` (512 MiB leaf) → PD0 `28:21` dual (2 MiB leaf) → big `20:16` / small
/// `20:12`; ONE address field `51:12` (no vid/sys split); PCF `7:3` whose low four enumerant bits
/// are UNCACHED/PRIVILEGE/RO/NO_ATOMIC; `PCF_SPARSE = 1`; KIND `11:8`.
///
/// ⚠ Built from [`kf_format_ver2`] and overridden field by field, so the zeroed padding the ABI
/// depends on is inherited, never re-created by a struct literal.
#[must_use]
pub fn kf_format_ver3() -> KfFormat {
    let mut f = kf_format_ver2();
    f.table_version = KF_TBL_VER3;
    f.first_dir = 0;
    let dir = |va_lo: u8, va_hi: u8, entry_bytes: u8, leaf_ps: u8| KfDir {
        active: 1,
        va_lo,
        entry_bytes,
        leaf_ps,
        entries: 1u16 << (va_hi - va_lo + 1),
        pad: 0,
    };
    f.dir[0] = dir(56, 56, 8, KF_PS_NONE);
    f.dir[1] = dir(47, 55, 8, KF_PS_NONE);
    f.dir[2] = dir(38, 46, 8, KF_PS_NONE);
    f.dir[3] = dir(29, 37, 8, PS_512M);
    f.dir[4] = dir(21, 28, 16, PS_2M);
    f.addr_sel = [0, 0, 0, 0];
    f.addr_local = KfField {
        lo: 12,
        bits: 40,
        shift: 12,
        pad: 0,
    };
    f.addr_sys = f.addr_local;
    f.big_addr_local = KfField {
        lo: 8,
        bits: 44,
        shift: 8,
        pad: 0,
    };
    f.big_addr_sys = f.big_addr_local;
    f.bit_volatile = 3;
    f.bit_privilege = 4;
    f.bit_read_only = 5;
    f.bit_atomic_disable = 6;
    f.pcf = KfField {
        lo: 3,
        bits: 5,
        shift: 0,
        pad: 0,
    };
    f.pcf_sparse = 1;
    f.kind = KfField {
        lo: 8,
        bits: 4,
        shift: 0,
        pad: 0,
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    /// `[1, 2, …, 255, 1, 2, …]`: non-zero, and distinct within any 255-byte window, so a field read
    /// at the wrong offset, at the wrong width or in the wrong byte order cannot come out right by
    /// accident. (The header is 392 bytes since ABI 6, so the pattern repeats once.)
    fn seq(n: usize) -> Vec<u8> {
        (0..n)
            .map(|i| u8::try_from(i % 255 + 1).expect("< 256"))
            .collect()
    }

    /// `n` little-endian bytes of `doc` at `at`, as the integer the documented layout means.
    fn le(doc: &[u8], at: usize, n: usize) -> u64 {
        doc[at..at + n]
            .iter()
            .rev()
            .fold(0u64, |a, b| (a << 8) | u64::from(*b))
    }

    /// ★★★ **THE HEADER'S DOCUMENTED LAYOUT, PINNED** (`cuda/walk/kf_walk.h`: *"little-endian,
    /// naturally aligned, no bitfields"*, `ReportHeader` 64 B). A third statement of it, stated as
    /// numbers, which needs no C compiler; `tests/walk_abi_matches_the_cu.rs` holds the same
    /// fields to `g++`'s own `offsetof`.
    #[test]
    fn the_report_structs_have_the_headers_documented_layout() {
        assert_eq!(
            (size_of::<KfReportHeader>(), KfReportHeader::BYTES),
            (392, 392),
            "ABI 6: 64 (the format doc's header) + 8 (counts) + 8 * 40 (samples)"
        );
        assert_eq!(
            (size_of::<KfRefusalSample>(), KfRefusalSample::BYTES),
            (40, 40)
        );
        let sm = [
            offset_of!(KfRefusalSample, va),
            offset_of!(KfRefusalSample, raw),
            offset_of!(KfRefusalSample, gpga),
            offset_of!(KfRefusalSample, ps_bytes),
            offset_of!(KfRefusalSample, bit),
            offset_of!(KfRefusalSample, level),
            offset_of!(KfRefusalSample, entry),
        ];
        assert_eq!(sm, [0, 8, 16, 24, 32, 36, 38]);
        assert_eq!((size_of::<KfPdbEntry>(), KfPdbEntry::BYTES), (32, 32));
        assert_eq!((size_of::<KfMapRun>(), KfMapRun::BYTES), (32, 32));
        let h = [
            offset_of!(KfReportHeader, magic),
            offset_of!(KfReportHeader, version),
            offset_of!(KfReportHeader, flags),
            offset_of!(KfReportHeader, generation),
            offset_of!(KfReportHeader, acked_generation),
            offset_of!(KfReportHeader, pdb_count),
            offset_of!(KfReportHeader, pdb_capacity),
            offset_of!(KfReportHeader, run_count),
            offset_of!(KfReportHeader, run_capacity),
            offset_of!(KfReportHeader, entries_visited),
            offset_of!(KfReportHeader, refusals),
            offset_of!(KfReportHeader, refuse_mask),
            offset_of!(KfReportHeader, sparse_slots),
            offset_of!(KfReportHeader, ps_log2),
            offset_of!(KfReportHeader, sample_count),
            offset_of!(KfReportHeader, sample_total),
            offset_of!(KfReportHeader, samples),
        ];
        assert_eq!(
            h,
            [
                0, 4, 6, 8, 16, 24, 28, 32, 36, 40, 48, 52, 56, 60, 64, 68, 72
            ]
        );
        let p = [
            offset_of!(KfPdbEntry, pdb),
            offset_of!(KfPdbEntry, first_run),
            offset_of!(KfPdbEntry, run_count),
            offset_of!(KfPdbEntry, vas_flags),
            offset_of!(KfPdbEntry, reserved),
            offset_of!(KfPdbEntry, reserved2),
        ];
        assert_eq!(p, [0, 8, 12, 16, 20, 24]);
        let r = [
            offset_of!(KfMapRun, va),
            offset_of!(KfMapRun, gpga),
            offset_of!(KfMapRun, len),
            offset_of!(KfMapRun, flags),
            offset_of!(KfMapRun, op),
            offset_of!(KfMapRun, pdb_index),
        ];
        assert_eq!(r, [0, 8, 16, 24, 28, 30]);
    }

    /// ★★★ **EACH DECODER READS THE DOCUMENTED BYTES AS THE DOCUMENTED VALUES** — little-endian,
    /// every field at its offset — and round-trips through the struct's own layout: the bytes the
    /// compiler lays the value out in (`view_bytes`, what the device writes) decode back to the
    /// value, `encode` produces exactly those bytes, and `encode ∘ decode` is the identity on any
    /// buffer (the fields tile the struct, so no byte is skipped or left stale).
    #[test]
    fn every_report_struct_round_trips_through_its_own_layout() {
        let hdr = KfReportHeader {
            magic: 0x0403_0201,
            version: 0x0605,
            flags: 0x0807,
            generation: 0x100F_0E0D_0C0B_0A09,
            acked_generation: 0x1817_1615_1413_1211,
            pdb_count: 0x1C1B_1A19,
            pdb_capacity: 0x201F_1E1D,
            run_count: 0x2423_2221,
            run_capacity: 0x2827_2625,
            entries_visited: 0x302F_2E2D_2C2B_2A29,
            refusals: 0x3433_3231,
            refuse_mask: 0x3837_3635,
            sparse_slots: 0x3C3B_3A39,
            ps_log2: [0x3D, 0x3E, 0x3F, 0x40],
            sample_count: 0x4443_4241,
            sample_total: 0x4847_4645,
            samples: core::array::from_fn(|i| {
                let b = 72 + i * KfRefusalSample::BYTES;
                let d = seq(KfReportHeader::BYTES);
                KfRefusalSample {
                    va: le(&d, b, 8),
                    raw: le(&d, b + 8, 8),
                    gpga: le(&d, b + 16, 8),
                    ps_bytes: le(&d, b + 24, 8),
                    bit: le(&d, b + 32, 4) as u32,
                    level: le(&d, b + 36, 2) as u16,
                    entry: le(&d, b + 38, 2) as u16,
                }
            }),
        };
        let pdb = KfPdbEntry {
            pdb: 0x0807_0605_0403_0201,
            first_run: 0x0C0B_0A09,
            run_count: 0x100F_0E0D,
            vas_flags: 0x1413_1211,
            reserved: 0x1817_1615,
            reserved2: 0x201F_1E1D_1C1B_1A19,
        };
        let run = KfMapRun {
            va: 0x0807_0605_0403_0201,
            gpga: 0x100F_0E0D_0C0B_0A09,
            len: 0x1817_1615_1413_1211,
            flags: 0x1C1B_1A19,
            op: 0x1E1D,
            pdb_index: 0x201F,
        };
        macro_rules! round_trip {
            ($t:ident, $v:expr) => {{
                let v: $t = $v;
                let doc = seq($t::BYTES);
                assert_eq!($t::decode(&doc), v, "{}: the documented bytes decode to the documented values", stringify!($t));
                assert_eq!(&v.encode()[..], &doc[..], "{}: encode is decode's inverse", stringify!($t));
                // The struct's own layout — the bytes the device writes — on a little-endian host.
                if cfg!(target_endian = "little") {
                    let laid_out = crate::driver_unsafe::view_bytes(&v);
                    assert_eq!(laid_out, &doc[..], "{}: the compiler lays it out as documented", stringify!($t));
                    assert_eq!($t::decode(laid_out), v, "{}: layout → decode → equal", stringify!($t));
                }
                // Any buffer: every byte is read and written back.
                let mut x = 0x9E37_79B9_7F4A_7C15u64;
                for _ in 0..64 {
                    let b: Vec<u8> = (0..$t::BYTES)
                        .map(|_| {
                            x ^= x << 13;
                            x ^= x >> 7;
                            x ^= x << 17;
                            x.to_le_bytes()[0]
                        })
                        .collect();
                    assert_eq!(&$t::decode(&b).encode()[..], &b[..], "{}: encode ∘ decode", stringify!($t));
                }
                // A longer buffer: only the struct's own prefix is read.
                let mut longer = doc.clone();
                longer.extend([0xEE; 7]);
                assert_eq!($t::decode(&longer), v);
            }};
        }
        round_trip!(KfReportHeader, hdr);
        let d = seq(KfRefusalSample::BYTES);
        round_trip!(
            KfRefusalSample,
            KfRefusalSample {
                va: le(&d, 0, 8),
                raw: le(&d, 8, 8),
                gpga: le(&d, 16, 8),
                ps_bytes: le(&d, 24, 8),
                bit: u32::try_from(le(&d, 32, 4)).expect("4 bytes"),
                level: u16::try_from(le(&d, 36, 2)).expect("2 bytes"),
                entry: u16::try_from(le(&d, 38, 2)).expect("2 bytes"),
            }
        );
        round_trip!(KfPdbEntry, pdb);
        round_trip!(KfMapRun, run);
        // Arrays decode struct by struct, as `try_collect` reads them; a trailing partial struct
        // is not read.
        let mut two: Vec<u8> = [run.encode(), KfMapRun { va: 7, ..run }.encode()].concat();
        let both = vec![run, KfMapRun { va: 7, ..run }];
        assert_eq!(KfMapRun::decode_all(&two), both);
        two.extend([0xEE; 31]);
        assert_eq!(KfMapRun::decode_all(&two), both);
        assert!(KfPdbEntry::decode_all(&[]).is_empty());
    }

    /// A short buffer is a panic, never a truncated value padded with whatever followed.
    #[test]
    #[should_panic(expected = "cannot hold a 32-byte KfMapRun")]
    fn a_short_buffer_is_refused_loudly() {
        let _ = KfMapRun::decode(&[0u8; 31]);
    }

    /// ★ The `KFWR_RF_*` ranges and single-bit flags of a run's `flags` are pairwise DISJOINT
    /// (a field cannot bleed into its neighbour), the class mask is the page-size field's low two
    /// bits, and each accessor reads exactly its range.
    #[test]
    fn the_flags_ranges_are_disjoint_and_each_accessor_reads_its_own() {
        let parts = [
            RF_AP.bits(),
            KFWR_RF_READ_ONLY,
            KFWR_RF_ATOMIC_DISABLE,
            KFWR_RF_VOLATILE,
            KFWR_RF_PRIVILEGE,
            RF_PS.bits(),
            RF_KIND.bits(),
            KFWR_RF_HELD,
        ];
        for (i, a) in parts.iter().enumerate() {
            for b in &parts[i + 1..] {
                assert_eq!(a & b, 0, "{a:#x} overlaps {b:#x}");
            }
        }
        assert_eq!(
            parts,
            [
                0x7,
                1 << 3,
                1 << 4,
                1 << 5,
                1 << 6,
                0xF << 8,
                0xFF << 16,
                1 << 31
            ]
        );
        assert_eq!(RF_CLASS.bits(), 0x3 << 8);
        assert_eq!(
            RF_CLASS.bits() & !RF_PS.bits(),
            0,
            "the class is inside the page-size field"
        );
        assert_eq!(KF_PS_CLASS_MASK as usize, crate::diffmodel::CLASSES - 1);
        for (ap, ps, kind) in [
            (AP_VID, PS_4K, 0u8),
            (AP_SYS_NC, PS_512M, 0xFF),
            (AP_PEER, PS_2M, 0x06),
        ] {
            let noise = KFWR_RF_HELD | KFWR_RF_KEY_PERM_ALL;
            let r = KfMapRun {
                flags: RF_AP.put(ap.into())
                    | RF_PS.put(ps.into())
                    | RF_KIND.put(kind.into())
                    | noise,
                ..KfMapRun::default()
            };
            assert_eq!(
                (r.aperture(), r.page_size(), r.class(), r.kind()),
                (ap, ps, usize::from(ps), kind)
            );
        }
        // ⊘ A page-size code above 3 ALIASES a class — as `kf_pcls`'s `& 3u` does on the GPU.
        let r = KfMapRun {
            flags: RF_PS.put(0x6),
            ..KfMapRun::default()
        };
        assert_eq!((r.page_size(), r.class()), (6, 2));
        // `put` masks: a value wider than its field cannot reach the next one.
        assert_eq!(RF_AP.put(0xFF), 0x7);
        assert_eq!(RF_KIND.put(0x1FF), 0xFF << 16);
        assert_eq!(
            KfPdbEntry {
                reserved: 42,
                ..KfPdbEntry::default()
            }
            .slot(),
            42
        );
    }
}
