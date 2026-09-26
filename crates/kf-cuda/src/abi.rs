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
pub const KF_ABI_VERSION: u32 = 5;

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
pub const KFWR_RF_KEY_PERM_ALL: u32 = KFWR_RF_READ_ONLY | KFWR_RF_ATOMIC_DISABLE | KFWR_RF_VOLATILE | KFWR_RF_PRIVILEGE;
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
        KfAck { generation: 0, nrun: 0, nreset: 0, reset: [0; KF_MAX_RESET] }
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

impl KfMapRun {
    /// The leaf aperture code: `flags` bits `KFWR_RF_AP_SHIFT`/`KFWR_RF_AP_MASK`
    /// (`cuda/walk/kf_walk.h:92-97` — 0 vidmem, 1 peer, 2 sys-coherent, 3 sys-noncoherent).
    #[must_use]
    pub fn aperture(&self) -> u8 {
        (self.flags & 0x7) as u8
    }
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
    f.addr_local = KfField { lo: 12, bits: 40, shift: 12, pad: 0 };
    f.addr_sys = f.addr_local;
    f.big_addr_local = KfField { lo: 8, bits: 44, shift: 8, pad: 0 };
    f.big_addr_sys = f.big_addr_local;
    f.bit_volatile = 3;
    f.bit_privilege = 4;
    f.bit_read_only = 5;
    f.bit_atomic_disable = 6;
    f.pcf = KfField { lo: 3, bits: 5, shift: 0, pad: 0 };
    f.pcf_sparse = 1;
    f.kind = KfField { lo: 8, bits: 4, shift: 0, pad: 0 };
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
