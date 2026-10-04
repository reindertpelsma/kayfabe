// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ **The walker's GPU half — perimeter tier 2** (`v3-sec-rawaddr`, 2026-10-04; audit S1-04;
//! `OWNER_RULINGS.md` §R(b): launch arguments are perimeter material).
//!
//! `walk.rs` keeps the walker's LOGIC — capacity planning, the retry decision, ack and reset
//! staging, report decoding — and reaches the GPU only through [`WalkGpu`]'s `pub(crate)` methods,
//! none of which takes or returns an address. This file holds the walker's device buffers, its
//! pinned stage, its kernels and its graph, and the validation sites the design numbers:
//!
//! - **V5** [`WalkGpu::bring_up`]: [`kf_format_check`] (a port of the `.cu`'s own), the
//!   configuration ([`validate_cfg`]) and the requirement table ([`requirements`]) checked against
//!   the sizes actually allocated;
//! - **V6** [`WalkGpu::launch`]: [`validate_stage`] — every layout row, every slot row, the entry
//!   list — and the window;
//! - **V7** [`WalkGpu::move_slot`]: [`validate_move`];
//! - **V8** [`WalkGpu::read_walk_region`] and [`WalkGpu::read_report`]'s clamps.
//!
//! ⊘ No `unsafe` here, and no address: every buffer is a `raw` handle, every pointer argument a
//! `raw` range minted by its owner, and the kernels' by-value `KfArgs` is encoded by `raw`.

use super::raw::{
    Arg, ArgBlock, Captured, Ctx, DevMem, Event, Flight, GraphExec, KF_ENT_BYTES, KF_MAX_FRONTIER,
    KF_MAX_SCRATCH, KF_RUN_BYTES, KF_SUM_BYTES, Kernel, KfArgsRanges, Module, PinnedStage, Ptx,
    Region, Stream, encode_kf_args,
};
use super::{CompletionFd, CudaError, refused};
use crate::abi::{
    KF_ABI_VERSION, KF_DIRS, KF_MAX_ENT, KF_MAX_PDB, KF_MAX_RESET, KF_MAX_SLOTS, KF_PS_NONE,
    KF_TBL_VER2, KF_TBL_VER3, KFWR_RF_KEY_PERM_ALL, KfAck, KfDev, KfFormat, KfLayout, KfMapRun,
    KfPdbEntry, KfReportHeader, KfSlot,
};
use crate::capacity::Region as PoolRegion;

/// The mangled entry points of the committed PTX (`kf_walk.cu` is C++). Each is in
/// `raw::KERNEL_SIGS`, which pins its parameters to the PTX's own declaration.
pub(crate) const SYM_BEGIN: &str = "_Z15kf_begin_kernel6KfArgs";
pub(crate) const SYM_WALK: &str = "_Z14kf_walk_kernel6KfArgs";
pub(crate) const SYM_DIFF_SLOTS: &str = "_Z13kf_diff_slots6KfArgs";
pub(crate) const SYM_DIFF_EMIT: &str = "_Z12kf_diff_emit6KfArgs";
pub(crate) const SYM_COMMIT: &str = "_Z16kf_commit_kernel6KfArgs";
/// The parallel walk's entries, in `kf_run_parallel`'s order.
pub(crate) const SYM_PAR: [&str; 9] = [
    "_Z11kf_par_seed6KfArgsP5KfEntPjS2_",
    "_Z13kf_par_expand6KfArgsjPK5KfEntPKjPS0_jPjS6_S6_",
    "_Z11kf_par_scanPKjS0_PjS1_",
    "_Z14kf_par_compact6KfArgsPK5KfEntPKjS4_S4_S4_PS0_j",
    "_Z11kf_par_leaf6KfArgsPK5KfEntPKjP8KfMapRunjPjP5KfSum",
    "_Z12kf_par_heads6KfArgsPK5KfEntPK5KfSumPKjPjPh",
    "_Z12kf_par_bases6KfArgsPj",
    "_Z11kf_par_emit6KfArgsPK5KfEntPK5KfSumPKjS7_PKhS7_PK8KfMapRun",
    "_Z11kf_par_join6KfArgsPK5KfEntPK5KfSumPKjS7_PKhS7_",
];

/// `kf_walk.cu` launch geometry — pinned to the `.cu`'s `#define`s by T6 below.
pub(crate) const KF_DIFF_BLOCK: u32 = 512;
pub(crate) const KF_PAR_BLOCK: u32 = 128;
pub(crate) const KF_PAR_GRID: u32 = 128;
pub(crate) const KF_SCAN_BLOCK: u32 = 1024;
pub(crate) const KF_WARP: u32 = 32;
pub(crate) const KF_SHWORDS: u32 = KF_MAX_ENT + 64;
/// ★ P4b: `used` holds one slot per directory level plus one for the leaf pass, all zeroed by ONE
/// memset at the start of the walk.
pub(crate) const KF_USED_SLOTS: u64 = KF_DIRS as u64 + 1;

// ═══ V5 — the format, the configuration, the requirement table ══════════════════════════════

fn pow2(v: u64) -> bool {
    v != 0 && v & (v - 1) == 0
}

/// ★★★ **V5 — the walk descriptor, checked exactly as the `.cu`'s `kf_format_check`
/// (`kf_walk.cu:1444-1478`)** — except that VER3 is ACCEPTED (w826, owner: every family is
/// first-class; the `.cu` gates it behind `KF_ALLOW_UNTESTED_VER3`). `KfFormat` has public fields
/// and arrives from safe callers, and the kernels index their own tables by its counts, so every
/// count is bounded here, before any launch. `Err` names the failed check.
///
/// # Errors
/// The first check that failed, by name.
pub fn kf_format_check(f: &KfFormat) -> Result<(), &'static str> {
    if f.abi_version != KF_ABI_VERSION {
        return Err("abi_version");
    }
    if f.table_version != KF_TBL_VER2 && f.table_version != KF_TBL_VER3 {
        return Err("table_version");
    }
    let first = usize::from(f.first_dir);
    if first >= KF_DIRS {
        return Err("first_dir");
    }
    if f.dir[KF_DIRS - 1].active == 0 {
        return Err("the deepest directory slot must be active");
    }
    for (k, d) in f.dir.iter().enumerate() {
        if d.active == 0 {
            if k >= first {
                return Err("an inactive slot below first_dir");
            }
            continue;
        }
        if k < first {
            return Err("an active slot above first_dir");
        }
        if d.entries == 0 || u32::from(d.entries) > KF_MAX_ENT {
            return Err("level fan-out exceeds KF_MAX_ENT");
        }
        if !pow2(u64::from(d.entries)) {
            return Err("level fan-out is not a power of two");
        }
        if d.entry_bytes != 8 && d.entry_bytes != 16 {
            return Err("entry_bytes");
        }
        if d.va_lo >= 64 {
            return Err("va_lo");
        }
        if !pow2(u64::from(d.entries) * u64::from(d.entry_bytes)) {
            return Err("table bytes not a power of two");
        }
        if d.leaf_ps != KF_PS_NONE && d.leaf_ps > 3 {
            return Err("leaf_ps");
        }
    }
    if f.dir[KF_DIRS - 1].entry_bytes != 16 {
        return Err("the deepest directory must carry a dual entry");
    }
    if f.small_entries == 0 || u32::from(f.small_entries) > KF_MAX_ENT {
        return Err("small_entries");
    }
    if f.big_entries == 0 || u32::from(f.big_entries) > KF_MAX_ENT {
        return Err("big_entries");
    }
    if f.big_va_lo <= f.small_va_lo || f.big_va_lo - f.small_va_lo > 4 {
        return Err("big/small stride");
    }
    if u32::from(f.big_entries) << (f.big_va_lo - f.small_va_lo) != u32::from(f.small_entries) {
        return Err("the big and small tables do not cover the same VA range");
    }
    if !pow2(u64::from(f.small_entries) * u64::from(f.small_entry_bytes)) {
        return Err("small table bytes");
    }
    if !pow2(u64::from(f.big_entries) * u64::from(f.big_entry_bytes)) {
        return Err("big table bytes");
    }
    if !pow2(u64::from(f.root_align)) {
        return Err("root_align");
    }
    if f.ap_bits == 0 || f.ap_bits > 2 {
        return Err("ap_bits");
    }
    if f.ps_log2.iter().any(|&p| !(12..=40).contains(&p)) {
        return Err("ps_log2");
    }
    Ok(())
}

/// ★ The walker's validated shape: every count the kernels and the pinned stage are sized by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shape {
    /// The most runs one space may hold (one walk region, one slot).
    pub runs_per_pdb: u32,
    /// The walk pool, in runs (and `4×` of it in scratch).
    pub walk_pool: u32,
    /// The committed-placement pool, in runs.
    pub slot_pool: u32,
    /// Committed-placement slots.
    pub max_slots: u32,
    /// The report's run capacity.
    pub run_capacity: u32,
    /// The report's `PdbEntry` capacity.
    pub pdb_capacity: u32,
    /// Entries one space's walk may examine.
    pub entry_budget: u32,
    /// The diff key's permission bits.
    pub key_perm: u32,
}

/// ★★ **V5 — the configuration.** `key_perm ⊆ KFWR_RF_KEY_PERM_ALL`; `1 ≤ walk_pool ≤
/// KF_MAX_SCRATCH/4` (the diff's scratch is `4 ×` the walk pool, carved from the leaf stage);
/// `slot_pool ≥ 1`; `1 ≤ max_slots ≤ KF_MAX_SLOTS`; `runs_per_pdb`, `run_capacity` and
/// `pdb_capacity` each at least 1.
///
/// # Errors
/// The first failed check, by name.
pub(crate) fn validate_cfg(cfg: &crate::walk::WalkCfg) -> Result<Shape, String> {
    if cfg.key_perm & !KFWR_RF_KEY_PERM_ALL != 0 {
        return Err(format!(
            "key_perm {:#x} names bits outside KFWR_RF_KEY_PERM_ALL {KFWR_RF_KEY_PERM_ALL:#x}",
            cfg.key_perm
        ));
    }
    if cfg.walk_pool == 0 || 4 * u64::from(cfg.walk_pool) > KF_MAX_SCRATCH {
        return Err(format!(
            "walk_pool {} does not fit the diff's scratch ({KF_MAX_SCRATCH} runs = 4 × the pool)",
            cfg.walk_pool
        ));
    }
    if cfg.slot_pool == 0 {
        return Err("slot_pool 0".to_string());
    }
    if cfg.max_slots == 0 || cfg.max_slots as usize > KF_MAX_SLOTS {
        return Err(format!(
            "max_slots {} outside 1..={KF_MAX_SLOTS}",
            cfg.max_slots
        ));
    }
    for (name, v) in [
        ("runs_per_pdb", cfg.runs_per_pdb),
        ("run_capacity", cfg.run_capacity),
        ("pdb_capacity", cfg.pdb_capacity),
    ] {
        if v == 0 {
            return Err(format!("{name} 0"));
        }
    }
    Ok(Shape {
        runs_per_pdb: cfg.runs_per_pdb,
        walk_pool: cfg.walk_pool,
        slot_pool: cfg.slot_pool,
        max_slots: cfg.max_slots,
        run_capacity: cfg.run_capacity,
        pdb_capacity: cfg.pdb_capacity,
        entry_budget: cfg.entry_budget,
        key_perm: cfg.key_perm,
    })
}

/// The walker's buffers, by name, in the order of [`requirements`].
pub(crate) const BUFFERS: [&str; 21] = [
    "dev",
    "walk",
    "com",
    "slot",
    "iscratch",
    "par.fr0",
    "par.fr1",
    "par.stage",
    "par.task",
    "par.runstage",
    "par.cnt",
    "par.off",
    "par.start",
    "par.nfr",
    "par.pdbbase",
    "par.used",
    "par.sum",
    "par.head",
    "stage.ackcode",
    "stage.rpdb",
    "stage.rrun",
];

/// ★★★ **V5 — the requirement table, written from the KERNELS' side**: the bytes each buffer
/// must hold for every index the `.cu` can form from this shape (`kf_walk.cu`: the walk table is
/// indexed by `walk_off + walk_cap ≤ walk_pool`, `kf_scr`/`kf_iscr` by `4×`/`3×` the walk pool,
/// the slot pool by `slot_off + slot_cap`, the frontier arrays by `KF_MAX_FRONTIER`, the leaf
/// stage by `KF_MAX_SCRATCH`, the report by its capacities). `None` on an overflow.
pub(crate) fn requirements(s: &Shape) -> Option<[u64; 21]> {
    let run = KF_RUN_BYTES;
    let f = KF_MAX_FRONTIER;
    Some([
        core::mem::size_of::<KfDev>() as u64,
        u64::from(s.walk_pool).checked_mul(run)?,
        u64::from(s.slot_pool).checked_mul(run)?,
        u64::from(s.max_slots).checked_mul(core::mem::size_of::<KfSlot>() as u64)?,
        u64::from(s.walk_pool).checked_mul(3 * 4)?,
        f * KF_ENT_BYTES,
        f * KF_ENT_BYTES,
        f * KF_ENT_BYTES,
        f * KF_ENT_BYTES,
        // the leaf stage, which the diff reuses as `4 × walk_pool` runs of scratch
        KF_MAX_SCRATCH
            .max(4 * u64::from(s.walk_pool))
            .checked_mul(run)?,
        f * 4,
        f * 4,
        f * 4,
        4 * 4,
        KF_MAX_PDB as u64 * 4,
        KF_USED_SLOTS * 4,
        f * KF_SUM_BYTES,
        f,
        u64::from(s.run_capacity),
        u64::from(s.pdb_capacity).checked_mul(KfPdbEntry::BYTES as u64)?,
        u64::from(s.run_capacity).checked_mul(KfMapRun::BYTES as u64)?,
    ])
}

/// ★ V5: refused unless every buffer holds what [`requirements`] says the kernels can address.
///
/// # Errors
/// The first short buffer, by name.
pub(crate) fn check_requirements(need: &[u64; 21], have: &[u64; 21]) -> Result<(), String> {
    for ((name, n), h) in BUFFERS.iter().zip(need).zip(have) {
        if h < n {
            return Err(format!(
                "{name} holds {h:#x} bytes; the kernels can address {n:#x}"
            ));
        }
    }
    Ok(())
}

// ═══ V6 — one walk's stage ══════════════════════════════════════════════════════════════════

/// What one walk writes into the pinned stage.
pub(crate) struct WalkStage<'a> {
    /// The roots, one per entry.
    pub pdbs: &'a [u64],
    /// The slots, one per entry.
    pub slots: &'a [u32],
    /// The capacity layout the kernels read in place.
    pub layout: &'a KfLayout,
    /// The verdict on the previous report: its generation and one code per run.
    pub ack: Option<(u64, &'a [u8])>,
    /// Slots the commit node empties.
    pub resets: &'a [u32],
}

/// ★★★ **V6 — a walk's stage against the shape**, in `u64` arithmetic: `npdb = pdbs = slots ≤
/// KF_MAX_PDB`; every slot below `max_slots`, none repeated; EVERY layout row (all `KF_MAX_PDB`,
/// not only the walked ones — the commit node reads the previous walk's rows) with `off + cap ≤
/// walk_pool` and `cap ≤ runs_per_pdb`; every slot row (all `KF_MAX_SLOTS`) with `off + cap ≤
/// slot_pool` and `cap ≤ runs_per_pdb`; at most `KF_MAX_RESET` resets. Because
/// `4·walk_pool ≤ KF_MAX_SCRATCH` (V5) and the integer scratch is `3·walk_pool` words, `kf_scr`
/// and `kf_iscr` stay inside their buffers for every accepted layout.
///
/// # Errors
/// The first failed check, by name.
pub(crate) fn validate_stage(sh: &Shape, s: &WalkStage<'_>) -> Result<(), String> {
    if s.pdbs.len() != s.slots.len() || s.pdbs.len() > KF_MAX_PDB {
        return Err(format!(
            "{} roots and {} slots (at most {KF_MAX_PDB}, one each)",
            s.pdbs.len(),
            s.slots.len()
        ));
    }
    let mut seen = [false; KF_MAX_SLOTS];
    for &slot in s.slots {
        if slot >= sh.max_slots || slot as usize >= KF_MAX_SLOTS || seen[slot as usize] {
            return Err(format!(
                "slot {slot} is out of range (max {}) or repeated in one walk",
                sh.max_slots
            ));
        }
        seen[slot as usize] = true;
    }
    let l = s.layout;
    let row = |off: u32, cap: u32, pool: u32| {
        u64::from(off) + u64::from(cap) <= u64::from(pool) && cap <= sh.runs_per_pdb
    };
    for t in 0..KF_MAX_PDB {
        if !row(l.walk_off[t], l.walk_cap[t], sh.walk_pool)
            || !row(l.prev_off[t], l.prev_cap[t], sh.walk_pool)
        {
            return Err(format!(
                "walk row {t}: [{}+{}) / previous [{}+{}) leaves the {}-run walk pool, or exceeds \
                 {} runs per space",
                l.walk_off[t],
                l.walk_cap[t],
                l.prev_off[t],
                l.prev_cap[t],
                sh.walk_pool,
                sh.runs_per_pdb
            ));
        }
    }
    for i in 0..KF_MAX_SLOTS {
        if !row(l.slot_off[i], l.slot_cap[i], sh.slot_pool) {
            return Err(format!(
                "slot row {i}: [{}+{}) leaves the {}-run slot pool, or exceeds {} runs per space",
                l.slot_off[i], l.slot_cap[i], sh.slot_pool, sh.runs_per_pdb
            ));
        }
    }
    if s.resets.len() > KF_MAX_RESET {
        return Err(format!(
            "{} resets; one verdict carries {KF_MAX_RESET}",
            s.resets.len()
        ));
    }
    Ok(())
}

/// ★ V7, pure: a slot's committed placements move from `from` to `to`: both inside the slot pool
/// (`u64`), the destination at least as large, the two disjoint, neither empty.
///
/// # Errors
/// The failed check, by name.
pub(crate) fn validate_move(
    slot_pool: u32,
    from: PoolRegion,
    to: PoolRegion,
) -> Result<(), String> {
    let end = |r: PoolRegion| u64::from(r.off) + u64::from(r.cap);
    if from.cap == 0 || end(from) > u64::from(slot_pool) || end(to) > u64::from(slot_pool) {
        return Err(format!(
            "{from:?} -> {to:?} leaves the {slot_pool}-run slot pool"
        ));
    }
    if from.cap > to.cap {
        return Err(format!("{from:?} does not fit {to:?}"));
    }
    if !(end(from) <= u64::from(to.off) || end(to) <= u64::from(from.off)) {
        return Err(format!("{from:?} and {to:?} overlap"));
    }
    Ok(())
}

// ═══ The handles ════════════════════════════════════════════════════════════════════════════

/// ★ V8, pure: a walk-pool read-back `[off, off+cap)` stays inside the pool.
fn walk_region_fits(walk_pool: u32, off: u32, cap: u32) -> Result<(), String> {
    if u64::from(off) + u64::from(cap) > u64::from(walk_pool) {
        return Err(format!(
            "[{off}+{cap}) leaves the {walk_pool}-run walk pool"
        ));
    }
    Ok(())
}

/// ★ V8, pure: the report's counts, clamped to the capacities this perimeter allocated (a
/// truncated report legitimately declares more than it carries, invariant I3).
fn report_counts(shape: &Shape, header: &KfReportHeader) -> (usize, usize) {
    (
        header.pdb_count.min(shape.pdb_capacity) as usize,
        header.run_count.min(shape.run_capacity) as usize,
    )
}

/// ★ A host image uploaded to device memory — the walker's window for a test or the self-test.
///
/// Opaque: its address never leaves the perimeter. Freed when dropped (after its context drained;
/// a walk that still uses it keeps it alive). Not `Copy`, not `Clone`, no `Hash` or `PartialEq`.
pub struct DeviceImage {
    mem: DevMem,
}

impl DeviceImage {
    /// Its length (never zero).
    #[must_use]
    pub fn len(&self) -> u64 {
        self.mem.len()
    }

    /// Never true: an empty upload is refused.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mem.len() == 0
    }
}

impl core::fmt::Debug for DeviceImage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DeviceImage")
            .field("len", &self.mem.len())
            .finish_non_exhaustive()
    }
}

/// Which window a walk reads.
pub(crate) enum Window<'a> {
    /// The imported store.
    Store,
    /// An uploaded image of THIS walker's context.
    Image(&'a DeviceImage),
    /// The window of the previous walk (a capacity re-walk).
    Same,
}

/// What [`WalkGpu::poll`] found.
pub(crate) enum Polled {
    /// No walk is in flight.
    Idle,
    /// The walk has not finished.
    Pending,
    /// It finished; the stage is the host's again. GPU time from the first node to the report.
    Done {
        /// Microseconds.
        gpu_us: u64,
    },
    /// A completion query failed: the walker is poisoned (F2) and refuses everything after.
    Poisoned(String),
}

/// The parallel walk's device scratch (`struct KfPar`).
struct ParBufs {
    fr: [DevMem; 2],
    stage: DevMem,
    task: DevMem,
    runstage: DevMem,
    cnt: DevMem,
    off: DevMem,
    start: DevMem,
    nfr: DevMem,
    pdbbase: DevMem,
    used: DevMem,
    sum: DevMem,
    head: DevMem,
}

/// The walk's kernels.
struct Kernels {
    begin: Kernel,
    walk: Kernel,
    diff_slots: Kernel,
    diff_emit: Kernel,
    commit: Kernel,
    par: [Kernel; 9],
}

/// ★★★ **CUDA, up and holding the walk kernel** — everything the walker needs on the GPU. One per
/// `WalkKernel`. Its drop releases every object after its context drained (or leaks them, counted,
/// when it cannot — F1).
pub(crate) struct WalkGpu {
    ctx: Ctx,
    k: Kernels,
    stream: Stream,
    ev_start: Event,
    ev_copied: Event,
    ev_done: Event,
    stage: PinnedStage,
    done_fd: CompletionFd,
    graph: Option<GraphExec>,
    par: ParBufs,
    fmt: KfFormat,
    shape: Shape,
    dev: DevMem,
    walk: DevMem,
    com: DevMem,
    slot: DevMem,
    iscratch: DevMem,
    store: Option<DevMem>,
    /// The window of the walk in flight / the last walk (kept alive for `Window::Same`).
    last_window: Option<DevMem>,
}

impl core::fmt::Debug for WalkGpu {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WalkGpu")
            .field("device", &self.ctx.device_name())
            .field("shape", &self.shape)
            .field("store_bytes", &self.store.as_ref().map(DevMem::len))
            .finish_non_exhaustive()
    }
}

/// What a bring-up reports besides the walker.
pub(crate) struct Ident {
    /// `cuDeviceGetName`.
    pub device_name: String,
    /// How long `cuModuleLoadData` (the PTX JIT) took, in microseconds.
    pub jit_us: u64,
}

fn us_since(t: std::time::Instant) -> u64 {
    u64::try_from(t.elapsed().as_micros()).unwrap_or(u64::MAX)
}

impl WalkGpu {
    /// ★★★ **V5 — bring CUDA up and load the walk.** The format check runs FIRST, before the
    /// library is even opened, so a Rust/PTX skew is refused by name and never mistaken for a
    /// CUDA problem (§21). Then the configuration, the context, the PTX JIT, every buffer, the
    /// requirement table against the sizes actually allocated, the stage, the graph, and a warm
    /// host signal.
    ///
    /// # Errors
    /// The failed check or the refusing call, by name.
    pub(crate) fn bring_up(
        cfg: &crate::walk::WalkCfg,
        fmt: &KfFormat,
        on: crate::walk::WalkDevice<'_>,
    ) -> Result<(WalkGpu, Ident), CudaError> {
        if let Err(why) = kf_format_check(fmt) {
            let what = if why == "abi_version" {
                "the walk kernel's setup data (abi_version)"
            } else {
                "the walk kernel's setup data (KfFormat)"
            };
            return Err(CudaError::Refused {
                what,
                code: i32::try_from(fmt.abi_version).unwrap_or(-1),
                name: format!(
                    "{why}: this build speaks KfFormat abi_version {KF_ABI_VERSION}; a descriptor \
                     the kernel's compile-time bounds cannot hold is REFUSED at launch, by name, \
                     rather than read as a page-table bug"
                ),
            });
        }
        let shape = validate_cfg(cfg).map_err(|e| refused("WalkKernel::bring_up (WalkCfg)", e))?;
        let bdf = match on {
            crate::walk::WalkDevice::FirstOrdinal => None,
            crate::walk::WalkDevice::PciBusId(b) => Some(b),
        };
        let ctx = Ctx::create(bdf)?;
        let tj = std::time::Instant::now();
        let module = Module::load(&ctx, Ptx::Walk)?;
        let jit_us = us_since(tj);
        let par_k = |i: usize| module.kernel(SYM_PAR[i]);
        let k = Kernels {
            begin: module.kernel(SYM_BEGIN)?,
            walk: module.kernel(SYM_WALK)?,
            diff_slots: module.kernel(SYM_DIFF_SLOTS)?,
            diff_emit: module.kernel(SYM_DIFF_EMIT)?,
            commit: module.kernel(SYM_COMMIT)?,
            par: [
                par_k(0)?,
                par_k(1)?,
                par_k(2)?,
                par_k(3)?,
                par_k(4)?,
                par_k(5)?,
                par_k(6)?,
                par_k(7)?,
                par_k(8)?,
            ],
        };
        let need = requirements(&shape).ok_or_else(|| {
            refused(
                "WalkKernel::bring_up (V5)",
                "a buffer size overflows".to_string(),
            )
        })?;
        let a = |i: usize| DevMem::alloc_zeroed(&ctx, need[i]);
        let (dev, walk, com, slot, iscratch) = (a(0)?, a(1)?, a(2)?, a(3)?, a(4)?);
        let par = ParBufs {
            fr: [a(5)?, a(6)?],
            stage: a(7)?,
            task: a(8)?,
            runstage: a(9)?,
            cnt: a(10)?,
            off: a(11)?,
            start: a(12)?,
            nfr: a(13)?,
            pdbbase: a(14)?,
            used: a(15)?,
            sum: a(16)?,
            head: a(17)?,
        };
        let to_usize =
            |v: u64| usize::try_from(v).map_err(|_| refused("WalkKernel::bring_up", "size".into()));
        let stage = PinnedStage::new(
            &ctx,
            &[
                KF_MAX_PDB * 8,
                KF_MAX_PDB * 4,
                core::mem::size_of::<KfAck>(),
                to_usize(need[18])?,
                KfReportHeader::BYTES,
                to_usize(need[19])?,
                to_usize(need[20])?,
                core::mem::size_of::<KfLayout>(),
            ],
        )?;
        let have = [
            dev.len(),
            walk.len(),
            com.len(),
            slot.len(),
            iscratch.len(),
            par.fr[0].len(),
            par.fr[1].len(),
            par.stage.len(),
            par.task.len(),
            par.runstage.len(),
            par.cnt.len(),
            par.off.len(),
            par.start.len(),
            par.nfr.len(),
            par.pdbbase.len(),
            par.used.len(),
            par.sum.len(),
            par.head.len(),
            stage.region_len(Region::AckCode) as u64,
            stage.region_len(Region::Rpdb) as u64,
            stage.region_len(Region::Rrun) as u64,
        ];
        check_requirements(&need, &have).map_err(|e| refused("WalkKernel::bring_up (V5)", e))?;
        // The device-side configuration, written exactly as the `.cu`'s `kf_create` does.
        let h = KfDev {
            runs_per_pdb: shape.runs_per_pdb,
            entry_budget: shape.entry_budget,
            run_capacity: shape.run_capacity,
            pdb_capacity: shape.pdb_capacity,
            max_pdbs: KF_MAX_PDB as u32,
            max_slots: shape.max_slots,
            ..KfDev::default()
        };
        dev.write(0, &h.encode())?;
        let stream = Stream::create(&ctx)?;
        let (ev_start, ev_copied, ev_done) = (
            Event::create(&ctx)?,
            Event::create(&ctx)?,
            Event::create(&ctx)?,
        );
        let done_fd = CompletionFd::new()?;
        let mut g = WalkGpu {
            ctx,
            k,
            stream,
            ev_start,
            ev_copied,
            ev_done,
            stage,
            done_fd,
            graph: None,
            par,
            fmt: *fmt,
            shape,
            dev,
            walk,
            com,
            slot,
            iscratch,
            store: None,
            last_window: None,
        };
        // ★ P4b: capture + instantiate the walk graph HERE, with every other lazy path (§w724d).
        g.graph = g.build_graph()?;
        g.warm_up()?;
        let ident = Ident {
            device_name: g.ctx.device_name().to_string(),
            jit_us,
        };
        Ok((g, ident))
    }

    /// The `KfArgs` of a walk over `win` (`None`: the empty window) with `npdb` entries.
    fn block(&self, win: Option<&DevMem>, npdb: u32) -> Result<ArgBlock, CudaError> {
        encode_kf_args(
            &KfArgsRanges {
                win: win.map(DevMem::whole),
                dev: self.dev.whole(),
                walk: self.walk.whole(),
                com: self.com.whole(),
                slot: self.slot.whole(),
                // ⊘ The walk's run stage: done with before the diff runs, free before the commit.
                scratch: self.par.runstage.whole(),
                iscratch: self.iscratch.whole(),
                stage: &self.stage,
            },
            &self.fmt,
            npdb,
            self.shape.key_perm,
        )
    }

    /// ★ P4b: pay the two remaining first-use costs at bring-up — the graph upload, and the
    /// driver's lazily started callback thread (one host signal, waited for on the fd and drained
    /// so it cannot complete the first real walk). ⊘ A `poll(2)`, never a context synchronize.
    fn warm_up(&self) -> Result<(), CudaError> {
        if let Some(g) = &self.graph {
            g.upload(&self.stream)?;
        }
        self.stream.host_signal(&self.done_fd)?;
        if !self.done_fd.wait_readable(10_000) || self.done_fd.drain() == 0 {
            return Err(refused(
                "WalkKernel::bring_up (warm-up host signal)",
                "the warm-up host function did not signal the completion fd within 10 s"
                    .to_string(),
            ));
        }
        Ok(())
    }

    /// Capture the whole walk once (placeholder `KfArgs`: no window, no spaces — the first launch
    /// rewrites them) and instantiate it. `Ok(None)` when the driver has no graph API; any failure
    /// of a capture the API claimed to support is a refusal, never a silent fallback.
    fn build_graph(&self) -> Result<Option<GraphExec>, CudaError> {
        if !self.ctx.has_graph_api() {
            return Ok(None);
        }
        let block = self.block(None, 0)?;
        self.stream.begin_capture()?;
        let mut nodes = Vec::new();
        let queued = self.enqueue_walk(&block, 0, &mut nodes);
        // ⊘ End the capture on EVERY path: a stream left capturing records every later walk.
        let ended = self.stream.end_capture(nodes);
        queued?;
        Ok(Some(ended?))
    }

    /// One launch on the walker's stream; under capture, its node is kept.
    fn kl(
        &self,
        nodes: &mut Vec<Captured>,
        k: &Kernel,
        grid: u32,
        block: u32,
        shm: u32,
        args: &[Arg<'_>],
    ) -> Result<(), CudaError> {
        if let Some(c) = k.launch(&self.stream, grid, block, shm, args)? {
            nodes.push(c);
        }
        Ok(())
    }

    /// Queue (or, under capture, record) the whole walk: the cursor reset, the commit of the
    /// previous verdict, the parallel walk, the diff, the report (written by the kernels straight
    /// into the pinned stage), the completion signal. Every pointer argument is a range of a
    /// buffer this walker owns; every capacity is the length of its own buffer (V3).
    fn enqueue_walk(
        &self,
        block: &ArgBlock,
        npdb: u32,
        nodes: &mut Vec<Captured>,
    ) -> Result<(), CudaError> {
        let s = &self.stream;
        self.ev_start.record(s)?;
        // ⊘ No root upload: the kernels read the pinned stage in place. Every level's staging
        // cursor is zeroed here, once (see `KF_USED_SLOTS`).
        self.par.used.whole().fill_async(s, 0)?;
        let grid = KF_MAX_PDB as u32;
        // ★ COMMIT-ON-ACK first: the previous report and its verdict, before anything is walked.
        self.kl(
            nodes,
            &self.k.commit,
            grid,
            KF_DIFF_BLOCK,
            0,
            &[Arg::Block(block)],
        )?;
        self.kl(nodes, &self.k.begin, 1, 1, 0, &[Arg::Block(block)])?;
        self.run_parallel(block, npdb, nodes)?;
        self.kl(
            nodes,
            &self.k.diff_slots,
            grid,
            KF_DIFF_BLOCK,
            0,
            &[Arg::Block(block)],
        )?;
        self.kl(nodes, &self.k.diff_emit, grid, 256, 0, &[Arg::Block(block)])?;
        self.ev_copied.record(s)?;
        // ⊘ ORDER: the host signal BEFORE `ev_done`. Then `ev_done` complete ⇒ the fd was
        // written, so a collect that saw the event can always drain the signal.
        s.host_signal(&self.done_fd)?;
        self.ev_done.record(s)
    }

    /// ★★★★★ w826 — THE PARALLEL WALK (`kf_run_parallel`, ported to the driver API).
    fn run_parallel(
        &self,
        block: &ArgBlock,
        npdb: u32,
        nodes: &mut Vec<Captured>,
    ) -> Result<(), CudaError> {
        let p = &self.par;
        let ab = || Arg::Block(block);
        let shm = (KF_PAR_BLOCK / KF_WARP) * KF_SHWORDS * 8;
        let [seed, expand, scan, compact, leaf, heads, bases, emit, join] = &self.k.par;
        let fr = [p.fr[0].whole(), p.fr[1].whole()];
        let nfr = p.nfr.whole();
        let used = p.used.whole();
        let (stage, task, runstage) = (p.stage.whole(), p.task.whole(), p.runstage.whole());
        let (cnt, off, start) = (p.cnt.whole(), p.off.whole(), p.start.whole());
        let (sum, head, pdbbase) = (p.sum.whole(), p.head.whole(), p.pdbbase.whole());
        self.kl(
            nodes,
            seed,
            npdb.div_ceil(128).max(1),
            128,
            0,
            &[ab(), Arg::Ptr(fr[0]), Arg::Ptr(nfr), Arg::Ptr(used)],
        )?;
        let mut src = 0usize;
        // The last level's output count IS the task count; it is read where it lies (no copy).
        let mut ntask = None;
        for k in u32::from(self.fmt.first_dir)..KF_DIRS as u32 {
            let used_k = used.sub(u64::from(k) * 4, 4)?;
            let nin = nfr.sub(src as u64 * 4, 4)?;
            let nout = nfr.sub((src ^ 1) as u64 * 4, 4)?;
            let dst = if k + 1 < KF_DIRS as u32 {
                fr[src ^ 1]
            } else {
                task
            };
            self.kl(
                nodes,
                expand,
                KF_PAR_GRID,
                KF_PAR_BLOCK,
                shm,
                &[
                    ab(),
                    Arg::U32(k),
                    Arg::Ptr(fr[src]),
                    Arg::Ptr(nin),
                    Arg::Ptr(stage),
                    Arg::CapOf(stage, KF_ENT_BYTES),
                    Arg::Ptr(used_k),
                    Arg::Ptr(start),
                    Arg::Ptr(cnt),
                ],
            )?;
            self.kl(
                nodes,
                scan,
                1,
                KF_SCAN_BLOCK,
                0,
                &[Arg::Ptr(cnt), Arg::Ptr(nin), Arg::Ptr(off), Arg::Ptr(nout)],
            )?;
            self.kl(
                nodes,
                compact,
                KF_PAR_GRID,
                KF_PAR_BLOCK,
                0,
                &[
                    ab(),
                    Arg::Ptr(stage),
                    Arg::Ptr(start),
                    Arg::Ptr(cnt),
                    Arg::Ptr(off),
                    Arg::Ptr(nin),
                    Arg::Ptr(dst),
                    Arg::CapOf(dst, KF_ENT_BYTES),
                ],
            )?;
            if k + 1 < KF_DIRS as u32 {
                src ^= 1;
            } else {
                ntask = Some(nout);
            }
        }
        // V5 bounds `first_dir < KF_DIRS`, so the loop ran and the last level set the task count.
        let ntask = ntask.ok_or_else(|| {
            refused(
                "WalkGpu::run_parallel",
                "no directory level was expanded".to_string(),
            )
        })?;
        self.kl(
            nodes,
            leaf,
            KF_PAR_GRID,
            KF_PAR_BLOCK,
            shm,
            &[
                ab(),
                Arg::Ptr(task),
                Arg::Ptr(ntask),
                Arg::Ptr(runstage),
                Arg::CapOf(runstage, KF_RUN_BYTES),
                Arg::Ptr(used.sub(KF_DIRS as u64 * 4, 4)?),
                Arg::Ptr(sum),
            ],
        )?;
        self.kl(
            nodes,
            heads,
            KF_PAR_GRID,
            128,
            0,
            &[
                ab(),
                Arg::Ptr(task),
                Arg::Ptr(sum),
                Arg::Ptr(ntask),
                Arg::Ptr(cnt),
                Arg::Ptr(head),
            ],
        )?;
        self.kl(
            nodes,
            scan,
            1,
            KF_SCAN_BLOCK,
            0,
            &[
                Arg::Ptr(cnt),
                Arg::Ptr(ntask),
                Arg::Ptr(off),
                Arg::Ptr(nfr.sub(12, 4)?),
            ],
        )?;
        self.kl(nodes, bases, 1, 1, 0, &[ab(), Arg::Ptr(pdbbase)])?;
        self.kl(
            nodes,
            emit,
            KF_PAR_GRID,
            KF_PAR_BLOCK,
            0,
            &[
                ab(),
                Arg::Ptr(task),
                Arg::Ptr(sum),
                Arg::Ptr(ntask),
                Arg::Ptr(off),
                Arg::Ptr(head),
                Arg::Ptr(pdbbase),
                Arg::Ptr(runstage),
            ],
        )?;
        self.kl(
            nodes,
            join,
            KF_PAR_GRID,
            128,
            0,
            &[
                ab(),
                Arg::Ptr(task),
                Arg::Ptr(sum),
                Arg::Ptr(ntask),
                Arg::Ptr(off),
                Arg::Ptr(head),
                Arg::Ptr(pdbbase),
            ],
        )
    }

    /// ★ Make this walker's context current on the calling thread.
    ///
    /// # Errors
    /// [`CudaError`].
    pub(crate) fn make_current(&self) -> Result<(), CudaError> {
        self.ctx.make_current()
    }

    /// ★★ V2 — import the RM-exported store (its length is the export's own). A second import is
    /// refused.
    ///
    /// # Errors
    /// The refusing step, by name.
    pub(crate) fn import_store(&mut self, store: &kf_host::RmExport) -> Result<(), CudaError> {
        if self.store.is_some() {
            return Err(refused(
                "import_store",
                "a store is already imported".to_string(),
            ));
        }
        self.store = Some(DevMem::import(&self.ctx, store)?);
        Ok(())
    }

    /// The imported store's length.
    pub(crate) fn store_len(&self) -> Option<u64> {
        self.store.as_ref().map(DevMem::len)
    }

    fn store(&self, what: &'static str) -> Result<&DevMem, CudaError> {
        self.store
            .as_ref()
            .ok_or_else(|| refused(what, "no store imported".to_string()))
    }

    /// V1 — write `b` at store offset `off`.
    ///
    /// # Errors
    /// Refused by name outside the store; the CUDA error otherwise.
    pub(crate) fn store_write(&self, off: u64, b: &[u8]) -> Result<(), CudaError> {
        self.store("WalkKernel::write_store")?.write(off, b)
    }

    /// V1 — read `b.len()` bytes at store offset `off`.
    ///
    /// # Errors
    /// As [`WalkGpu::store_write`].
    pub(crate) fn store_read(&self, off: u64, b: &mut [u8]) -> Result<(), CudaError> {
        self.store("WalkKernel::read_store")?.read(off, b)
    }

    /// Copy a host image (non-empty) into fresh device memory.
    ///
    /// # Errors
    /// Refused for an empty image; the CUDA error otherwise.
    pub(crate) fn upload(&self, b: &[u8]) -> Result<DeviceImage, CudaError> {
        if b.is_empty() {
            return Err(refused("WalkKernel::upload", "an empty image".to_string()));
        }
        let mem = DevMem::alloc_zeroed(&self.ctx, b.len() as u64)?;
        mem.write(0, b)?;
        Ok(DeviceImage { mem })
    }

    fn own<'a>(&self, img: &'a DeviceImage, what: &'static str) -> Result<&'a DevMem, CudaError> {
        if img.mem.ctx_id() != self.ctx.id() {
            return Err(refused(
                what,
                "the image belongs to another walker's CUDA context".to_string(),
            ));
        }
        Ok(&img.mem)
    }

    /// V1 — read `b.len()` bytes of an image of THIS walker at `off`.
    ///
    /// # Errors
    /// Refused by name for another context's image or outside it; the CUDA error otherwise.
    pub(crate) fn image_read(
        &self,
        img: &DeviceImage,
        off: u64,
        b: &mut [u8],
    ) -> Result<(), CudaError> {
        self.own(img, "WalkKernel::read_image")?.read(off, b)
    }

    /// V1 — write `b` into an image of THIS walker at `off`.
    ///
    /// # Errors
    /// As [`WalkGpu::image_read`].
    pub(crate) fn image_write(
        &self,
        img: &DeviceImage,
        off: u64,
        b: &[u8],
    ) -> Result<(), CudaError> {
        self.own(img, "WalkKernel::write_image")?.write(off, b)
    }

    /// ★★★ **V6 + V4 + V3 — queue one walk.** Refused unless the stage is `Idle` (no walk in
    /// flight, not poisoned), the stage passes [`validate_stage`], and the window is this walker's
    /// (`Store` needs an import; `Image` this context; `Same` a retained window). The window's
    /// length is the allocation's own, never a caller's. Then the stage is written (V4), the
    /// `KfArgs` encoded by `raw`, and the walk queued — one `cuGraphLaunch`, or launch by launch.
    /// **Nothing here waits on the GPU** on the success path.
    ///
    /// # Errors
    /// The failed check or the refusing call, by name.
    pub(crate) fn launch(&mut self, w: Window<'_>, s: &WalkStage<'_>) -> Result<(), CudaError> {
        let what = "WalkGpu::launch";
        match self.stage.flight() {
            Flight::Idle => {}
            Flight::InFlight => {
                return Err(refused(
                    what,
                    "a walk is already in flight; collect it first".to_string(),
                ));
            }
            Flight::Poisoned(why) => {
                return Err(refused(what, format!("the walker is poisoned: {why}")));
            }
        }
        validate_stage(&self.shape, s).map_err(|e| refused("WalkGpu::launch (V6)", e))?;
        let win = match w {
            Window::Store => self.store(what)?.share(),
            Window::Image(img) => self.own(img, what)?.share(),
            Window::Same => self
                .last_window
                .as_ref()
                .ok_or_else(|| refused(what, "no previous window to re-walk".to_string()))?
                .share(),
        };
        self.ctx.make_current()?;
        self.stage.write(Region::Lay, 0, &s.layout.encode())?;
        let mut pdb_bytes = Vec::with_capacity(s.pdbs.len() * 8);
        let mut slot_bytes = Vec::with_capacity(s.slots.len() * 4);
        for (p, sl) in s.pdbs.iter().zip(s.slots) {
            pdb_bytes.extend_from_slice(&p.to_le_bytes());
            slot_bytes.extend_from_slice(&sl.to_le_bytes());
        }
        self.stage.write(Region::Pdbs, 0, &pdb_bytes)?;
        self.stage.write(Region::Slots, 0, &slot_bytes)?;
        // ★ The verdict on the previous report — or "none" (generation 0), so a verdict is
        // consumed by exactly one walk. The kernel commits only when `nrun` equals the report's
        // `run_count`, so codes beyond the stage's capacity simply commit nothing.
        let mut ack = KfAck::default();
        if let Some((generation, codes)) = s.ack {
            let n = codes.len().min(self.stage.region_len(Region::AckCode));
            ack.generation = generation;
            ack.nrun = u32::try_from(codes.len()).unwrap_or(u32::MAX);
            self.stage.write(Region::AckCode, 0, &codes[..n])?;
        }
        for (i, r) in s.resets.iter().enumerate() {
            ack.reset[i] = *r;
        }
        ack.nreset = s.resets.len() as u32;
        self.stage.write(Region::Ack, 0, &ack.encode())?;
        let npdb = s.pdbs.len() as u32;
        let block = self.block(Some(&win), npdb)?;
        let queued = match self.graph.as_mut() {
            Some(g) => g.launch(
                &self.stream,
                &block,
                &[(SYM_PAR[0], npdb.div_ceil(128).max(1))],
            ),
            None => self.enqueue_walk(&block, npdb, &mut Vec::new()),
        };
        if let Err(e) = queued {
            self.recover_after_failed_enqueue();
            return Err(e);
        }
        self.last_window = Some(win);
        Ok(())
    }

    /// An enqueue failed part-way: some of the walk may be queued, and the stage is in flight with
    /// no completion signal coming. Drain the context (an ERROR path only) and give the stage back;
    /// a drain that fails poisons the walker (F2).
    fn recover_after_failed_enqueue(&mut self) {
        match self.ctx.drain() {
            Ok(d) => {
                if let Err(e) = self.stage.end_flight(d) {
                    self.stage.poison(e.to_string());
                }
            }
            Err(e) => self
                .stage
                .poison(format!("a failed enqueue could not be drained: {e}")),
        }
    }

    /// ★★ **V4 / F2 — has the walk in flight finished? Never blocks.** A signalled fd means the
    /// host function ran, so `ev_copied` (recorded before it) proves the report landed; a silent fd
    /// asks `ev_done`, so a failed stream — whose host function never runs — is named rather than
    /// waited on. The proof returns the stage to `Idle`; a failed query POISONS the walker.
    pub(crate) fn poll(&mut self) -> Polled {
        match self.stage.flight() {
            Flight::Idle => return Polled::Idle,
            Flight::Poisoned(why) => return Polled::Poisoned(why),
            Flight::InFlight => {}
        }
        let signalled = self.done_fd.drain() > 0;
        let ev = if signalled {
            &self.ev_copied
        } else {
            &self.ev_done
        };
        match ev.query() {
            Ok(None) => {
                if signalled {
                    // Stream order says this cannot happen; if it does, re-arm the wake so the
                    // worker's epoll comes back.
                    self.done_fd.signal();
                }
                Polled::Pending
            }
            Ok(Some(d)) => {
                if !signalled {
                    // `ev_done` follows the host signal, so the fd was written: consume it now so
                    // it cannot complete the next walk.
                    let _ = self.done_fd.drain();
                }
                if let Err(e) = self.stage.end_flight(d) {
                    let w = e.to_string();
                    self.stage.poison(w.clone());
                    return Polled::Poisoned(w);
                }
                Polled::Done {
                    gpu_us: self.ev_start.elapsed_us(&self.ev_copied).unwrap_or(0),
                }
            }
            Err(e) => {
                let w = e.to_string();
                self.stage.poison(w.clone());
                Polled::Poisoned(w)
            }
        }
    }

    /// Whether the walk in flight has finished, by `cuEventQuery` — non-consuming, non-blocking.
    ///
    /// # Errors
    /// The stream's failure, by name.
    pub(crate) fn gpu_done_now(&self) -> Result<bool, CudaError> {
        if self.stage.flight() == Flight::Idle {
            return Ok(true);
        }
        Ok(self.ev_done.query()?.is_some())
    }

    /// ★★ **V4 + V8 — the collected report**: refused unless the stage is `Idle`, and its counts
    /// clamped to the capacities here, inside the perimeter (a truncated report legitimately
    /// declares more runs than it carries, invariant I3).
    ///
    /// # Errors
    /// Refused while a walk is in flight or after a poisoning.
    pub(crate) fn read_report(
        &self,
    ) -> Result<(KfReportHeader, Vec<KfPdbEntry>, Vec<KfMapRun>), CudaError> {
        let header =
            KfReportHeader::decode(&self.stage.read(Region::Hdr, 0, KfReportHeader::BYTES)?);
        let (npdb, nrun) = report_counts(&self.shape, &header);
        let pdbs =
            KfPdbEntry::decode_all(&self.stage.read(Region::Rpdb, 0, npdb * KfPdbEntry::BYTES)?);
        let runs =
            KfMapRun::decode_all(&self.stage.read(Region::Rrun, 0, nrun * KfMapRun::BYTES)?);
        Ok((header, pdbs, runs))
    }

    /// ★★ **V7 — grow a slot**: its committed placements copied `from` → `to` on the walk stream,
    /// ordered before the next walk's commit. Refused unless `Idle` and [`validate_move`] holds.
    ///
    /// # Errors
    /// The failed check or the CUDA error, by name.
    pub(crate) fn move_slot(&mut self, from: PoolRegion, to: PoolRegion) -> Result<(), CudaError> {
        if self.stage.flight() != Flight::Idle {
            return Err(refused(
                "WalkGpu::move_slot (V7)",
                "not between walks".to_string(),
            ));
        }
        validate_move(self.shape.slot_pool, from, to)
            .map_err(|e| refused("WalkGpu::move_slot (V7)", e))?;
        let run = KF_RUN_BYTES;
        let n = u64::from(from.cap) * run;
        let src = self.com.range(u64::from(from.off) * run, n)?;
        let dst = self.com.range(u64::from(to.off) * run, n)?;
        src.copy_async(&self.stream, &dst)
    }

    /// ★ V8 — the last walk's runs in walk-pool region `[off, off+cap)`: refused unless `Idle` and
    /// inside the pool. A read-back of OUR table, never a guest's.
    ///
    /// # Errors
    /// The failed check or the CUDA error, by name.
    pub(crate) fn read_walk_region(&self, off: u32, cap: u32) -> Result<Vec<KfMapRun>, CudaError> {
        if self.stage.flight() != Flight::Idle {
            return Err(refused(
                "WalkGpu::read_walk_region (V8)",
                "a walk is in flight".to_string(),
            ));
        }
        walk_region_fits(self.shape.walk_pool, off, cap)
            .map_err(|e| refused("WalkGpu::read_walk_region (V8)", e))?;
        let mut buf = vec![0u8; cap as usize * KfMapRun::BYTES];
        self.walk.read(u64::from(off) * KF_RUN_BYTES, &mut buf)?;
        Ok(KfMapRun::decode_all(&buf))
    }

    /// ★★ **A launch that must FAIL** (§w724d probe (b)): `kf_walk_kernel` with a 2048-thread
    /// block and an empty window. `Some(why)` when the DRIVER refused it, as intended; `None` when
    /// it was not refused by the driver (it launched — a finding — or it could not be attempted).
    /// Either way the context is drained afterwards: this is the one deliberate synchronize of the
    /// walker (the known positive of `tests/walk_never_synchronizes.rs`).
    pub(crate) fn probe_oversized_block(&mut self) -> Option<String> {
        if self.stage.flight() != Flight::Idle {
            return None;
        }
        let block = self.block(None, 0).ok()?;
        let r = self
            .k
            .walk
            .probe_oversized_block(&self.stream, &[Arg::Block(&block)]);
        match self.ctx.drain() {
            Ok(d) => {
                let _ = self.stage.end_flight(d);
            }
            Err(e) => self.stage.poison(format!("the probe's drain failed: {e}")),
        }
        match r {
            Err(
                e @ CudaError::Refused {
                    what: "cuLaunchKernel (deliberately malformed)",
                    ..
                },
            ) => Some(e.to_string()),
            _ => None,
        }
    }

    /// The completion fd.
    pub(crate) fn completion_fd(&self) -> &CompletionFd {
        &self.done_fd
    }

    /// `cuCtxSynchronize` calls this walker's context made.
    pub(crate) fn ctx_sync_calls(&self) -> u64 {
        self.ctx.sync_calls()
    }

    /// Whether a walk is ONE `cuGraphLaunch`.
    pub(crate) fn submits_as_graph(&self) -> bool {
        self.graph.is_some()
    }

    /// Walks that had to rewrite the graph's by-value arguments.
    pub(crate) fn graph_param_updates(&self) -> u64 {
        self.graph.as_ref().map_or(0, GraphExec::updates)
    }

    /// The walk pool, slot pool, integer scratch and leaf stage, in bytes.
    pub(crate) fn pool_bytes(&self) -> u64 {
        self.walk.len() + self.com.len() + self.iscratch.len() + self.par.runstage.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{kf_format_ver2, kf_format_ver3};
    use crate::walk::WalkCfg;

    fn shape() -> Shape {
        validate_cfg(&WalkCfg::default()).expect("the default configuration")
    }

    fn lay() -> KfLayout {
        let mut l = KfLayout::default();
        l.walk_cap[0] = 256;
        l.slot_cap[3] = 1024;
        l
    }

    fn stage<'a>(l: &'a KfLayout, pdbs: &'a [u64], slots: &'a [u32]) -> WalkStage<'a> {
        WalkStage {
            pdbs,
            slots,
            layout: l,
            ack: None,
            resets: &[],
        }
    }

    /// ★ T9 — V8: a read-back past the walk pool is refused (an exact fit accepted), and a
    /// report's counts are clamped to the capacities allocated here.
    #[test]
    fn a_read_back_stays_inside_what_was_allocated() {
        let s = shape();
        let p = s.walk_pool;
        assert_eq!(walk_region_fits(p, 0, p), Ok(()), "the exact fit");
        assert_eq!(walk_region_fits(p, p - 1, 1), Ok(()), "the last run");
        assert!(walk_region_fits(p, p, 1).is_err(), "one past the end");
        assert!(walk_region_fits(p, 1, p).is_err(), "one run too long");
        assert!(walk_region_fits(p, u32::MAX, u32::MAX).is_err(), "no wrap");
        let cap = (s.pdb_capacity as usize, s.run_capacity as usize);
        for (pdbs, runs) in [
            (s.pdb_capacity + 1, s.run_capacity + 1),
            (u32::MAX, u32::MAX),
        ] {
            let h = KfReportHeader {
                pdb_count: pdbs,
                run_count: runs,
                ..KfReportHeader::default()
            };
            assert_eq!(report_counts(&s, &h), cap, "clamped to what was allocated");
        }
        let h = KfReportHeader {
            pdb_count: 3,
            run_count: 7,
            ..KfReportHeader::default()
        };
        assert_eq!(report_counts(&s, &h), (3, 7), "a count inside is kept");
    }

    /// ★ T20 — V5's format check: the `.cu`'s grid, each arm refused; VER2 and VER3 accepted.
    #[test]
    fn the_format_check_refuses_every_arm_of_the_cus() {
        assert_eq!(kf_format_check(&kf_format_ver2()), Ok(()));
        assert_eq!(
            kf_format_check(&kf_format_ver3()),
            Ok(()),
            "VER3 is accepted (w826)"
        );
        type Mutation = (&'static str, fn(&mut KfFormat));
        let bad: Vec<Mutation> = vec![
            ("abi_version", |f| f.abi_version += 1),
            ("table_version", |f| f.table_version = 4),
            ("first_dir", |f| f.first_dir = 5),
            ("an active slot above first_dir", |f| f.first_dir = 2),
            ("an inactive slot below first_dir", |f| f.dir[2].active = 0),
            ("the deepest directory slot must be active", |f| {
                f.dir[4].active = 0
            }),
            ("level fan-out exceeds KF_MAX_ENT", |f| {
                f.dir[2].entries = 1024
            }),
            ("level fan-out is not a power of two", |f| {
                f.dir[2].entries = 3
            }),
            ("entry_bytes", |f| f.dir[2].entry_bytes = 12),
            ("va_lo", |f| f.dir[2].va_lo = 64),
            ("leaf_ps", |f| f.dir[2].leaf_ps = 4),
            ("the deepest directory must carry a dual entry", |f| {
                f.dir[4].entry_bytes = 8;
            }),
            ("small_entries", |f| f.small_entries = 0),
            ("big_entries", |f| f.big_entries = 1024),
            ("big/small stride", |f| f.big_va_lo = f.small_va_lo + 5),
            (
                "the big and small tables do not cover the same VA range",
                |f| {
                    f.big_entries = 16;
                },
            ),
            ("root_align", |f| f.root_align = 4095),
            ("ap_bits", |f| f.ap_bits = 3),
            ("ps_log2", |f| f.ps_log2[3] = 41),
            ("ps_log2", |f| f.ps_log2[0] = 11),
        ];
        for (want, mutate) in bad {
            for base in [kf_format_ver2(), kf_format_ver3()] {
                let mut f = base;
                mutate(&mut f);
                if f == base {
                    continue;
                }
                assert_eq!(kf_format_check(&f), Err(want), "{want}");
            }
        }
    }

    /// ★ T5 — V5's configuration and requirement table.
    #[test]
    fn the_configuration_and_the_requirement_table_are_bounded() {
        let ok = WalkCfg::default();
        assert!(validate_cfg(&ok).is_ok());
        for (why, c) in [
            (
                "4·walk_pool > KF_MAX_SCRATCH",
                WalkCfg {
                    walk_pool: (KF_MAX_SCRATCH / 4 + 1) as u32,
                    ..ok
                },
            ),
            ("walk_pool 0", WalkCfg { walk_pool: 0, ..ok }),
            (
                "max_slots KF_MAX_SLOTS+1",
                WalkCfg {
                    max_slots: KF_MAX_SLOTS as u32 + 1,
                    ..ok
                },
            ),
            ("max_slots 0", WalkCfg { max_slots: 0, ..ok }),
            ("slot_pool 0", WalkCfg { slot_pool: 0, ..ok }),
            (
                "run_capacity 0",
                WalkCfg {
                    run_capacity: 0,
                    ..ok
                },
            ),
            (
                "pdb_capacity 0",
                WalkCfg {
                    pdb_capacity: 0,
                    ..ok
                },
            ),
            (
                "runs_per_pdb 0",
                WalkCfg {
                    runs_per_pdb: 0,
                    ..ok
                },
            ),
            (
                "key_perm",
                WalkCfg {
                    key_perm: 1 << 0,
                    ..ok
                },
            ),
        ] {
            assert!(validate_cfg(&c).is_err(), "{why}");
        }
        assert!(
            validate_cfg(&WalkCfg {
                walk_pool: (KF_MAX_SCRATCH / 4) as u32,
                ..ok
            })
            .is_ok(),
            "the exact fit is accepted"
        );
        let s = shape();
        let need = requirements(&s).expect("no overflow");
        assert_eq!(check_requirements(&need, &need), Ok(()));
        for i in 0..need.len() {
            let mut have = need;
            have[i] -= 1;
            let e = check_requirements(&need, &have).expect_err("one byte short");
            assert!(e.starts_with(BUFFERS[i]), "{e}");
        }
        assert_eq!(need[1], u64::from(s.walk_pool) * 32, "the walk table");
        assert_eq!(
            need[4],
            u64::from(s.walk_pool) * 12,
            "kf_iscr: 3 words per walk run"
        );
        assert!(
            need[9] >= 4 * u64::from(s.walk_pool) * 32,
            "kf_scr: 4 runs per walk run"
        );
    }

    /// ★ T7 — V6: every row, not only the walked ones; the entry list; the slots.
    #[test]
    fn a_stage_is_refused_past_any_pool_on_any_row() {
        let s = shape();
        let l = lay();
        assert_eq!(validate_stage(&s, &stage(&l, &[0x1000], &[3])), Ok(()));
        // an UNWALKED row (t = 63 with npdb = 1) one run past the pool
        let mut b = l;
        b.walk_off[63] = s.walk_pool - 10;
        b.walk_cap[63] = 11;
        assert!(validate_stage(&s, &stage(&b, &[0x1000], &[3])).is_err());
        b.walk_cap[63] = 10;
        assert_eq!(
            validate_stage(&s, &stage(&b, &[0x1000], &[3])),
            Ok(()),
            "exact fit"
        );
        let mut b = l;
        b.prev_off[7] = s.walk_pool;
        b.prev_cap[7] = 1;
        assert!(
            validate_stage(&s, &stage(&b, &[0x1000], &[3])).is_err(),
            "prev row"
        );
        let mut b = l;
        b.slot_off[127] = s.slot_pool - 1;
        b.slot_cap[127] = 2;
        assert!(
            validate_stage(&s, &stage(&b, &[0x1000], &[3])).is_err(),
            "slot row 127"
        );
        let mut b = l;
        b.walk_cap[0] = s.runs_per_pdb + 1;
        assert!(
            validate_stage(&s, &stage(&b, &[0x1000], &[3])).is_err(),
            "walk_cap > runs_per_pdb"
        );
        let roots = [0u64; 65];
        let slots: Vec<u32> = (0..65).collect();
        assert!(
            validate_stage(&s, &stage(&l, &roots, &slots)).is_err(),
            "npdb = 65"
        );
        assert!(
            validate_stage(&s, &stage(&l, &[1], &[s.max_slots])).is_err(),
            "slot >= max_slots"
        );
        assert!(
            validate_stage(&s, &stage(&l, &[1, 2], &[3, 3])).is_err(),
            "a repeated slot"
        );
        assert!(
            validate_stage(&s, &stage(&l, &[1, 2], &[3])).is_err(),
            "roots != slots"
        );
        let resets = [0u32; 65];
        let mut st = stage(&l, &[1], &[3]);
        st.resets = &resets;
        assert!(validate_stage(&s, &st).is_err(), "nreset = 65");
    }

    /// ★ T8 — V7.
    #[test]
    fn a_slot_move_stays_in_the_pool_and_never_overlaps() {
        let r = |off, cap| PoolRegion { off, cap };
        assert_eq!(validate_move(4096, r(0, 1024), r(1024, 2048)), Ok(()));
        assert!(
            validate_move(4096, r(0, 1024), r(2048, 2049)).is_err(),
            "to past the pool"
        );
        assert!(
            validate_move(4096, r(0, 1024), r(512, 2048)).is_err(),
            "overlap"
        );
        assert!(
            validate_move(4096, r(0, 2048), r(2048, 1024)).is_err(),
            "from larger than to"
        );
        assert!(
            validate_move(4096, r(0, 0), r(1024, 1024)).is_err(),
            "an empty source"
        );
    }

    /// ★ T6 — the launch geometry and the device structs this file sizes buffers by are the
    /// `.cu`'s own `#define`s and `sizeof`s (a `g++` probe, as `tests/walk_abi_matches_the_cu.rs`).
    #[test]
    fn the_walk_constants_are_the_cus() {
        let cu = include_str!("../../../../cuda/walk/kf_walk.cu");
        let def = |name: &str| -> u64 {
            let line = cu
                .lines()
                .find(|l| l.trim_start().starts_with(&format!("#define {name} ")))
                .unwrap_or_else(|| panic!("kf_walk.cu has no #define {name}"));
            let v = line
                .split_whitespace()
                .nth(2)
                .unwrap_or_else(|| panic!("#define {name} has no value"));
            let v = v.trim_start_matches('(').trim_end_matches('u');
            if let Some(rest) = line.split("(4u << ").nth(1) {
                return 4 << rest.split(')').next().unwrap().parse::<u64>().unwrap();
            }
            v.parse().unwrap_or_else(|_| panic!("#define {name} = {v}"))
        };
        assert_eq!(def("KF_MAX_FRONTIER"), KF_MAX_FRONTIER);
        assert_eq!(def("KF_MAX_SCRATCH"), KF_MAX_SCRATCH);
        assert_eq!(def("KF_DIRS"), KF_DIRS as u64);
        assert_eq!(def("KF_MAX_ENT"), u64::from(KF_MAX_ENT));
        assert_eq!(def("KF_WARP"), u64::from(KF_WARP));
        assert_eq!(def("KF_PAR_BLOCK"), u64::from(KF_PAR_BLOCK));
        assert_eq!(def("KF_SCAN_BLOCK"), u64::from(KF_SCAN_BLOCK));
        assert_eq!(def("KF_PAR_GRID"), u64::from(KF_PAR_GRID));
        assert_eq!(def("KF_DIFF_BLOCK"), u64::from(KF_DIFF_BLOCK));
        // sizeof(KfEnt), sizeof(KfSum): extracted by name and computed by the C compiler.
        let grab = |name: &str| -> String {
            let at = cu.find(&format!("struct {name} {{")).expect("the struct");
            let end = at + cu[at..].find("};").expect("its end") + 2;
            cu[at..end].to_string()
        };
        let main = r#"int main(void){printf("%zu %zu\n", sizeof(struct KfEnt), sizeof(struct KfSum));return 0;}"#;
        let prog = format!(
            "#include <stdint.h>\n#include <stdio.h>\n{}\n{}\n{main}\n",
            grab("KfEnt"),
            grab("KfSum")
        );
        let dir = std::env::temp_dir().join(format!("kf_t6_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let (src, bin) = (dir.join("t6.c"), dir.join("t6"));
        std::fs::write(&src, prog).expect("write");
        let out = std::process::Command::new("cc")
            .arg("-o")
            .arg(&bin)
            .arg(&src)
            .output()
            .expect("a C compiler must be present: a check that cannot run is not a pass");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let run = std::process::Command::new(&bin).output().expect("run");
        let text = String::from_utf8_lossy(&run.stdout);
        let mut it = text.split_whitespace().map(|v| v.parse::<u64>().unwrap());
        assert_eq!(it.next(), Some(KF_ENT_BYTES), "sizeof(KfEnt)");
        assert_eq!(it.next(), Some(KF_SUM_BYTES), "sizeof(KfSum)");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
