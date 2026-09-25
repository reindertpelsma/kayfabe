//! ★★★★★ **THE WALK KERNEL, DRIVEN FROM RUST** — load the committed PTX, allocate, launch,
//! read the report back, validate it.
//!
//! This is the half of `cuda/walk/kf_walk.cu` that the `.cu` writes against the CUDA
//! **runtime** API (`kf_create`/`kf_refresh`) and that kayfabe cannot use: the runtime API
//! lives in `libcudart`, which a driver-only box does not have, and it owns a context
//! lifecycle this process wants to own itself. ⊘ The device half is untouched — it is the
//! committed PTX, built from that same file.
//!
//! # ★★★★★ P4 (w826) — THE WALK NEVER BLOCKS THE THREAD THAT SUBMITS IT
//!
//! `V3_P4_PORT_MAP.md` §2.1(d) / Q6: `refresh` used to call `cuCtxSynchronize` **twice on the
//! caller's stack** (between the parallel walk and the diff, and again before the read-back),
//! which `THE_TRANSLATED_PLANE.md` §5 and `THE_CONSTRAINTS.md` §41 forbid for a worker. Now:
//! - [`WalkKernel::submit`] queues the whole walk on the kernel's **own stream** — the pdb
//!   upload, every launch, the report read-back into **pinned** host memory — and ends it with
//!   a `cuLaunchHostFunc` that writes an **eventfd** ([`WalkKernel::completion_fd`]). It returns
//!   as soon as the work is queued.
//! - [`WalkKernel::try_collect`] never blocks: it drains the fd (or, when the fd is silent,
//!   asks `cuEventQuery` so a device fault is still named) and, once the walk is done, decodes
//!   the report from pinned memory.
//! - [`WalkKernel::refresh`] survives as `submit` + a `poll(2)` on the fd, **for harnesses and
//!   the selftest only**. It runs no `cuCtxSynchronize` either; [`WalkKernel::ctx_sync_calls`]
//!   is the counter gate 8 reads to prove it.
//!
//! # ★★★★★ 2026-09-25 — THE WALK EMITS A DIFF, COMMITTED ON ACK
//!
//! Owner design + ruling (COMMIT-ON-ACK), `crate::diffmodel` for the full statement: the GPU
//! holds, per VA-space object (a **slot**), the placements the host confirmed it made; every walk
//! reports each entry's DIFF against its slot; the host applies it and answers one verdict per run
//! ([`WalkKernel::ack`]); the next walk's first node ([`kf_commit_kernel`]) folds exactly the
//! acknowledged runs into the slot. A refused map stays a difference and is re-emitted.
//!
//! ⊘ **Why this is not the snapshot `V3_BUILD.md` ruled out (P4 deleted `ack` for it):** that
//! snapshot was the previous WALK — guest table content, committed whether or not the host acted
//! on it (a §18.2 shadow, and the walkmirror CPU copy beside it). A slot is a record of OUR host
//! actions, written only on confirmation: the ledger v3 §4.2 (w825) sanctions, kept in vidmem.
//!
//! [`kf_commit_kernel`]: ../../../cuda/walk/kf_walk.cu

use crate::abi::{
    KF_ABI_VERSION, KF_MAX_PDB, KF_MAX_RESET, KF_TBL_VER2, KFWR_HF_DIFF, KFWR_HF_TRUNCATED,
    KFWR_MAGIC, KFWR_OP_UNMAP, KfAck, KfArgs, KfDev, KfFormat, KfMapRun, KfPdbEntry,
    KfReportHeader, KfSlot,
};
use crate::driver_unsafe::{
    CUdeviceptr, CompletionFd, CtxHandle, Cuda, CudaError, EventHandle, Func, GraphExecHandle,
    GraphHandle, GraphNode, PinnedBuf, StreamHandle,
};

/// ★★★ **The committed PTX.** Built from `cuda/walk/kf_walk.cu` by
/// `cuda/walk/make_ptx.py` — NVRTC, no GPU and no nvcc, so it is generated where the rest of
/// this tree is generated rather than on a rented box.
///
/// ⊘ `THE_CONSTRAINTS.md` §20: *"it is not code injection: the PTX is ours, built at build
/// time"*. Embedding it makes that literally true of the shipped artifact — there is no path
/// at run time from which a different program could be read, which also means the sandbox has
/// nothing to grant for it.
pub static WALK_PTX: &[u8] = include_bytes!("../../../cuda/walk/kf_walk.ptx");

/// The mangled entry points of the committed PTX. ⊘ **Mangled**, because `kf_walk.cu` is C++
/// and its `__global__` functions are not `extern "C"`. Asserted present at module load, so a
/// rename in the `.cu` is a named refusal here rather than a null function pointer later.
const SYM_BEGIN: &str = "_Z15kf_begin_kernel6KfArgs";
const SYM_WALK: &str = "_Z14kf_walk_kernel6KfArgs";
const SYM_DIFF_SLOTS: &str = "_Z13kf_diff_slots6KfArgs";
const SYM_DIFF_EMIT: &str = "_Z12kf_diff_emit6KfArgs";
const SYM_COMMIT: &str = "_Z16kf_commit_kernel6KfArgs";
/// `KF_DIFF_BLOCK` in the `.cu`: the diff and commit kernels' block (one block per entry).
const KF_DIFF_BLOCK: u32 = 512;

// ★ w826 — THE PARALLEL WALK's entry points (mangled; `kf_walk.cu`'s `kf_run_parallel`).
const SYM_PAR: [&str; 9] = [
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
/// `kf_walk.cu` constants the parallel sequence is sized by. ⊘ Must match the `.cu`.
const KF_DIRS: u32 = 5;
const KF_MAX_FRONTIER: usize = 131_072;
const KF_MAX_SCRATCH: usize = 4 << 20;
const KF_PAR_BLOCK: u32 = 128;
const KF_PAR_GRID: u32 = 128;
const KF_SCAN_BLOCK: u32 = 1024;
const KF_WARP: u32 = 32;
const KF_MAX_ENT: u32 = 512;
const KF_SHWORDS: u32 = KF_MAX_ENT + 64;
/// ★ P4b: `used` (the staging cursor) holds one slot PER LEVEL plus one for the leaf pass, all
/// zeroed by ONE memset at the start of the walk — instead of a reset node after every level.
/// Slot `KF_DIRS` is the leaf pass's; slots `0..KF_DIRS` the expand levels'.
const KF_USED_SLOTS: usize = KF_DIRS as usize + 1;
/// `sizeof(KfEnt)` — 3×u64 + 2×u32 + u16 + 2×u8, padded to 8.
const KF_ENT_BYTES: usize = 40;
/// `sizeof(KfSum)` — 6×u64 + 4×u32.
const KF_SUM_BYTES: usize = 64;

/// The parallel walk's device scratch (`struct KfPar`).
#[derive(Debug)]
struct ParBufs {
    fr: [DevBuf; 2],
    stage: DevBuf,
    task: DevBuf,
    runstage: DevBuf,
    cnt: DevBuf,
    off: DevBuf,
    start: DevBuf,
    nfr: DevBuf,
    pdbbase: DevBuf,
    used: DevBuf,
    sum: DevBuf,
    head: DevBuf,
}

/// How large a report this driver asks the kernel for.
#[derive(Debug, Clone, Copy)]
pub struct WalkCfg {
    /// Slice size of the kernel's own table, per address space.
    pub runs_per_pdb: u32,
    /// Report run-array capacity.
    pub run_capacity: u32,
    /// Report `PdbEntry` capacity.
    pub pdb_capacity: u32,
    /// Entries one address space's walk may examine.
    pub entry_budget: u32,
    /// [`KF_TBL_VER2`] or [`KF_TBL_VER3`].
    pub table_version: u32,
    /// ★ Committed-placement slots — one per VA-space object that is ever walked.
    pub max_slots: u32,
}

impl Default for WalkCfg {
    fn default() -> Self {
        // ★★★ SIZED FOR A REAL GUEST'S TABLES, not for the synthetic fixture.
        //
        // ⊘ The previous values (`runs_per_pdb: 256`, `run_capacity: 4096`) were the
        // selftest's, where one address space maps eight contiguous pages and coalesces to a
        // single run. `[w725]`'s capture of a REAL driver has **6 254 leaves in one address
        // space**, and the live shadow (`kayfabe_mmu::walkshadow`) runs against tables like
        // those. A per-address-space slice of 256 makes `KFWR_R_RUN_CAP` fire and the whole
        // report **truncate** — which the shadow refuses by name, so the symptom would be a
        // permanently VACUOUS census with the cause one indirection away.
        //
        // ⚠ **`run_capacity` is bounded from ABOVE by the wire, and that bound is real.** The
        // report crosses as one `Reply::Payload` and the isolate protocol's `FRAME_MAX` is
        // 1 MiB; a `KfMapRun` is 32 bytes. 16 384 runs is 512 KiB — half the frame, with room
        // for the header and the `PdbEntry` array. Raising this further requires chunking the
        // reply, not a bigger number.
        // ★★★★★ w826 — `runs_per_pdb` RAISED 2048 → 16 384, to the report's own ceiling.
        // `[measured w826 q9]` `--ce-client-guest-ram` maps 13 000 separate 4 KiB guest-RAM
        // pages in ONE space; past 2 048 runs every walk truncated (`KFWR_R_RUN_CAP`), a
        // truncated walk is never reconciled, and new mappings STOPPED being published — the
        // arm read as slow and was stalled. With C4's deltas a report carries a space's full
        // state only on a RESYNC, so the table, not the wire, was the binding limit. Cost:
        // 64 spaces × 16 384 × 32 B × 2 snapshots = 64 MiB of device memory.
        // ⚠ A space with more than 16 384 non-coalescing runs still truncates: that needs a
        // chunked reply (the wire), not a bigger number here.
        // ★ 2026-09-25: `runs_per_pdb` is also ONE SLOT's capacity (committed placements), and
        // the diff's scratch (4 × runs per entry) reuses the walk's run stage — exactly full at
        // 16 384. Memory: walk 64 × 16 384 × 32 B = 32 MiB, slots 128 × 16 384 × 32 B = 64 MiB
        // (was two 32 MiB snapshot tables).
        WalkCfg {
            runs_per_pdb: 16384,
            run_capacity: 16384,
            pdb_capacity: 64,
            entry_budget: 1 << 22,
            table_version: KF_TBL_VER2,
            max_slots: 128,
        }
    }
}

/// ★ One walk entry: the root to walk, and the slot (VA-space object) whose committed placements
/// its diff is taken against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkEntry {
    /// The root.
    pub pdb: u64,
    /// The slot.
    pub slot: u32,
}

fn refused(what: &'static str, name: String) -> CudaError {
    CudaError::Refused { what, code: 0, name }
}

/// What came back from one refresh.
#[derive(Debug, Clone)]
pub struct Report {
    /// The header.
    pub header: KfReportHeader,
    /// One entry per address space described.
    pub pdbs: Vec<KfPdbEntry>,
    /// The runs, in the order the kernel emitted them.
    pub runs: Vec<KfMapRun>,
    /// ★★★★★ §39(c): the GPGA span this walk was bounded by, carried so that
    /// [`Report::validate`] can check CONTAINMENT without the caller having to remember to
    /// supply it. ⊘ Recorded at construction from the refresh that produced the report — a
    /// span the caller passes separately is a span the caller can forget, and this is the
    /// check standing between a malicious guest and a mapping outside its own store.
    pub gpga_span: u64,
}

/// Why a report is not well formed. ⊘ Each variant names **which** property failed, because
/// *"the report is bad"* is not actionable and this is the only check standing between the
/// kernel and a caller that would act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportError {
    /// The magic is wrong — the buffer is not a report at all.
    BadMagic(u32),
    /// `pdb_count` exceeds `pdb_capacity`.
    PdbOverflow {
        /// What the header claimed.
        count: u32,
        /// What it was given.
        capacity: u32,
    },
    /// `run_count` exceeds `run_capacity` on a report **not** marked truncated.
    RunOverflow {
        /// What the header claimed.
        count: u32,
        /// What it was given.
        capacity: u32,
    },
    /// A `PdbEntry`'s slice runs past the end of the run array.
    SliceOutOfRange {
        /// Which entry.
        index: usize,
        /// Its first run.
        first: u32,
        /// Its run count.
        count: u32,
    },
    /// A run names a `pdb_index` that does not exist.
    RunPdbIndex {
        /// Which run.
        index: usize,
        /// The index it named.
        pdb_index: u16,
    },
    /// A run has zero length. ⊘ A zero-length mapping contributes nothing and can never appear
    /// in a coverage residual, so it is refused rather than counted.
    ZeroLenRun(usize),
    /// ★★★★★ §39(c): the run names memory OUTSIDE the guest's own GPGA span. Mapping it
    /// would hand the guest memory that is not its own — the escalation, not a malformation.
    RunOutsideGpga {
        /// Which run.
        index: usize,
        /// The GPGA it named.
        gpga: u64,
        /// Its length.
        len: u64,
        /// The span it had to lie inside.
        span: u64,
    },
    /// ★ The report is not a DIFF against the committed placements (`KFWR_HF_DIFF` clear): it
    /// came from a kernel speaking another protocol, and applying it as a diff would be wrong.
    NotDiff {
        /// The header flags seen.
        flags: u16,
    },
}

impl core::fmt::Display for ReportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ReportError::BadMagic(m) => write!(f, "magic {m:#x} is not a walk report"),
            ReportError::PdbOverflow { count, capacity } => {
                write!(f, "pdb_count {count} > capacity {capacity}")
            }
            ReportError::RunOverflow { count, capacity } => {
                write!(
                    f,
                    "run_count {count} > capacity {capacity}, and not TRUNCATED"
                )
            }
            ReportError::SliceOutOfRange {
                index,
                first,
                count,
            } => write!(f, "pdb[{index}] slice {first}+{count} runs past the array"),
            ReportError::RunPdbIndex { index, pdb_index } => {
                write!(
                    f,
                    "run[{index}] names pdb_index {pdb_index}, which does not exist"
                )
            }
            ReportError::ZeroLenRun(i) => write!(f, "run[{i}] has len 0"),
            ReportError::RunOutsideGpga {
                index,
                gpga,
                len,
                span,
            } => write!(
                f,
                "run[{index}] leaves the guest's GPGA: gpga={gpga:#x} len={len:#x} ends at \
                 {:#x}, span is {span:#x} — mapping it would hand the guest memory that is \
                 not its own",
                gpga.saturating_add(*len)
            ),
            ReportError::NotDiff { flags } => write!(
                f,
                "not a diff report (flags={flags:#x}): the kernel does not speak the \
                 commit-on-ack protocol this host applies"
            ),
        }
    }
}

impl Report {
    /// ★★ **Property 3 of `the_walk_kernel_report_format.md`, on the host.**
    ///
    /// ⊘ This is a **second** implementation of the `.cu`'s own `kf_validate_report`, not a
    /// call into it — the `.cu`'s copy is host code we do not link. Two implementations of a
    /// validator is normally a bug factory; here one of them is the oracle the CUDA suite runs
    /// and this one is what production consults, which is the shape the tree already sanctions
    /// for the walker itself.
    ///
    /// # Errors
    /// The first property that failed, by name.
    pub fn validate(&self) -> Result<(), ReportError> {
        let h = &self.header;
        if h.magic != KFWR_MAGIC {
            return Err(ReportError::BadMagic(h.magic));
        }
        if h.pdb_count > h.pdb_capacity {
            return Err(ReportError::PdbOverflow {
                count: h.pdb_count,
                capacity: h.pdb_capacity,
            });
        }
        // ⚠ A TRUNCATED report is ALLOWED to claim more runs than it carries — that is what
        // truncation means, and I3 says it must be loud rather than short-and-plausible. It is
        // refused as a delta elsewhere; it is not malformed.
        if h.run_count > h.run_capacity && (h.flags & KFWR_HF_TRUNCATED) == 0 {
            return Err(ReportError::RunOverflow {
                count: h.run_count,
                capacity: h.run_capacity,
            });
        }
        let runs = u32::try_from(self.runs.len()).unwrap_or(u32::MAX);
        for (i, p) in self.pdbs.iter().enumerate() {
            let end = u64::from(p.first_run) + u64::from(p.run_count);
            if end > u64::from(runs) {
                return Err(ReportError::SliceOutOfRange {
                    index: i,
                    first: p.first_run,
                    count: p.run_count,
                });
            }
        }
        for (i, r) in self.runs.iter().enumerate() {
            if usize::from(r.pdb_index) >= self.pdbs.len() {
                return Err(ReportError::RunPdbIndex {
                    index: i,
                    pdb_index: r.pdb_index,
                });
            }
            if r.len == 0 {
                return Err(ReportError::ZeroLenRun(i));
            }
            // ★★★★★ §39(c) CONTAINMENT — the one property here about ESCALATION rather than
            // well-formedness. An UNMAP names a VA being retired and carries no gpga, so it is
            // exempt; every other run becomes a MAPPING, and a mapping outside the guest's own
            // store hands it memory that is not its own.
            // ⊘ The kernel refuses these at both emit chokepoints and `storemap::map` bounds
            // again at map time. This is the MIDDLE layer and it was missing: the validator
            // documented as "what production consults" checked capacities, slice ranges and
            // zero-len while saying NOTHING about where a run points.
            if r.op != KFWR_OP_UNMAP
                && (r.gpga > self.gpga_span || r.len > self.gpga_span - r.gpga)
            {
                return Err(ReportError::RunOutsideGpga {
                    index: i,
                    gpga: r.gpga,
                    len: r.len,
                    span: self.gpga_span,
                });
            }
        }
        Ok(())
    }

    /// Whether the walk was truncated — in which case it is not the whole state of any space
    /// and must never be reconciled against.
    #[must_use]
    pub fn truncated(&self) -> bool {
        (self.header.flags & KFWR_HF_TRUNCATED) != 0
    }

    /// ★ Require a DIFF report (`KFWR_HF_DIFF`). See [`ReportError::NotDiff`].
    ///
    /// # Errors
    /// [`ReportError::NotDiff`].
    pub fn require_diff(&self) -> Result<(), ReportError> {
        if (self.header.flags & KFWR_HF_DIFF) == 0 {
            return Err(ReportError::NotDiff { flags: self.header.flags });
        }
        Ok(())
    }
}

/// One collected walk: the report and what it cost.
#[derive(Debug, Clone)]
pub struct Collected {
    /// The report (validate it, and [`Report::require_diff`], before acting on it).
    pub report: Report,
    /// GPU time from the first queued operation to the last read-back copy, from CUDA events.
    pub gpu_us: u64,
    /// Wall time from `submit` returning to `try_collect` seeing the completion.
    pub submit_to_collect_us: u64,
}

/// Where the report pieces live in the pinned buffer.
#[derive(Debug, Clone, Copy)]
struct PinLayout {
    pdbs: usize,
    slots: usize,
    ack: usize,
    ack_code: usize,
    hdr: usize,
    rpdb: usize,
    rrun: usize,
    total: usize,
}

impl PinLayout {
    fn for_cfg(cfg: &WalkCfg) -> PinLayout {
        let pdbs = 0;
        let slots = KF_MAX_PDB * 8;
        let ack = (slots + KF_MAX_PDB * 4).next_multiple_of(64);
        let ack_code = (ack + core::mem::size_of::<KfAck>()).next_multiple_of(64);
        let hdr = (ack_code + cfg.run_capacity as usize).next_multiple_of(64);
        let rpdb = hdr + core::mem::size_of::<KfReportHeader>().next_multiple_of(64);
        let rrun = rpdb
            + (cfg.pdb_capacity as usize * core::mem::size_of::<KfPdbEntry>()).next_multiple_of(64);
        let total = rrun + cfg.run_capacity as usize * core::mem::size_of::<KfMapRun>();
        PinLayout { pdbs, slots, ack, ack_code, hdr, rpdb, rrun, total }
    }
}

/// The walk in flight (at most one).
#[derive(Debug, Clone, Copy)]
struct InFlight {
    gpga_len: u64,
    submitted: std::time::Instant,
}

/// A device allocation, freed on drop.
///
/// ⊘ It holds a **clone of nothing** — the freeing goes through the owner's [`Cuda`], so this
/// type deliberately cannot free itself. `WalkKernel` frees them, or neutralises them and
/// lets `cuCtxDestroy` reclaim the lot; see its `Drop`.
#[derive(Debug, Clone, Copy)]
struct DevBuf {
    ptr: CUdeviceptr,
}

/// One kernel launch of the captured walk graph, with the parameters it currently carries in
/// the instantiated graph — kept so a per-walk parameter change is ONE setter call per node,
/// never a re-capture.
#[derive(Debug)]
struct GraphKernel {
    node: GraphNode,
    f: Func,
    grid: u32,
    block: u32,
    shm: u32,
    params: Vec<Vec<u8>>,
    what: &'static str,
    /// Parameter 0 is the by-value `KfArgs` (every kernel but `kf_par_scan`).
    takes_args: bool,
    /// `kf_par_seed`, whose grid is sized by `npdb`.
    seed: bool,
}

/// ★★★★★ **P4b (w827) — THE WALK, CAPTURED ONCE AS A CUDA GRAPH.**
///
/// `[measured GA106 cc3caf1f, gate 8]` `submit_us p50=172 max=429` (VER2), `p50=212` (VER3)
/// against a 50 µs budget — ~25 stream operations, each paying the driver's per-call cost on
/// the submitting thread, while the GPU needs only ~190-240 µs for the whole walk. The launch
/// SEQUENCE is fixed per format: it depends only on `KfFormat::first_dir` (the number of
/// directory levels expanded), never on the data — every kernel is sized by fixed grids and
/// reads its live counts (`nfr`, `ntask`, `used`) from device memory, so an empty frontier is
/// a launch that exits early, not a launch that is skipped. ⇒ one graph per `WalkKernel`
/// (= per format and `WalkCfg` shape), captured at bring-up, replayed by one `cuGraphLaunch`.
///
/// ⊘ **What changes per walk, and how it reaches the graph.** Every buffer is allocated at
/// bring-up and never moves, so the graph's pointers are constants. The per-walk inputs are
/// (a) the pdb list — written into PINNED memory whose device address IS `KfArgs::pdbs`, so
/// the kernels read it in place and it needs no graph change and no upload node — and (b) the by-value `KfArgs` (`win.base`/`len`/`span` = the store, `npdb`)
/// plus `kf_par_seed`'s grid (`ceil(npdb/128)`). (b) goes through
/// `cuGraphExecKernelNodeSetParams`, **and only when it differs from what the graph already
/// carries** — in steady state the store never moves and the address-space count changes
/// only when the guest creates or destroys a space, so a walk makes zero setter calls.
/// ⚠ The preferred alternative — kernels reading `KfArgs` from a device-side parameter block —
/// needs a `.cu` edit, a PTX regeneration (`cuda/walk/make_ptx.py`) and the 58-assertion CUDA
/// suite re-run; every kernel takes `KfArgs` BY VALUE today, so no host-side choice can make
/// it indirect. The setter path is the one available without touching the device half.
///
/// ⚠ `[measured GA106, w827]` what is left is per-NODE driver cost, ~0.5 µs/node with a warm
/// submitting thread and ~2 µs/node when it was idle (sleeping) just before — the case a
/// worker woken by its epoll is in. Nodes are therefore cut wherever the device half allows
/// (one cursor reset, no `ntask` copy, pdbs read in place, one read-back copy): VER2 27
/// nodes, VER3 30. Below that needs FEWER KERNELS (fusing each level's expand/scan/compact),
/// which is a `.cu` change. A walk that must rewrite the setters (first walk; the space count
/// changed) pays ~1-2 µs more per `KfArgs`-bearing node (VER2 15, VER3 17).
#[derive(Debug)]
struct WalkGraph {
    graph: GraphHandle,
    exec: GraphExecHandle,
    kernels: Vec<GraphKernel>,
    /// The `KfArgs` bytes every `takes_args` node currently carries. Empty = unknown (an
    /// update failed part-way), which forces the next walk to rewrite every node.
    baked_args: Vec<u8>,
    baked_npdb: u32,
    updates: u64,
}

/// ★★★★★ **CUDA, up and holding the walk kernel.** One per VM, in the scratchpad isolate.
///
/// ⚠ **Everything lazy is walked during [`WalkKernel::bring_up`]**, deliberately and in
/// order: `cuInit` → `cuDeviceGet` → `cuCtxCreate` → `cuModuleLoadData` (**the PTX JIT runs
/// here**) → every allocation → and then a **real launch** by the caller.
/// `THE_CONSTRAINTS.md` §w724d: CUDA is aggressively lazy and *"each lazy path is one that
/// would otherwise fail after the drop, looking like a GPU fault rather than a sandbox
/// effect"*.
pub struct WalkKernel {
    cu: Cuda,
    ctx: CtxHandle,
    f_begin: Func,
    f_walk: Func,
    f_diff_slots: Func,
    f_diff_emit: Func,
    f_commit: Func,
    f_par: [Func; 9],
    /// ★ P4: the walk's own stream, its events, the pinned read-back and the completion fd.
    stream: StreamHandle,
    ev_start: EventHandle,
    ev_copied: EventHandle,
    ev_done: EventHandle,
    pin: PinnedBuf,
    pin_at: PinLayout,
    done_fd: CompletionFd,
    inflight: Option<InFlight>,
    /// ★ P4b: the captured walk; `None` only when this driver has no graph API (then every
    /// walk is submitted launch by launch, and [`WalkKernel::submits_as_graph`] says so).
    graph: Option<WalkGraph>,
    par: ParBufs,
    fmt: KfFormat,
    cfg: WalkCfg,
    dev: DevBuf,
    /// The walk's runs (`runs_per_pdb` per entry).
    walk: DevBuf,
    /// ★ The committed placements (`runs_per_pdb` per slot) and their per-class counts.
    com: DevBuf,
    slot: DevBuf,
    iscratch: DevBuf,
    /// ★ P4b: the DEVICE address of the pinned pdb stage (`pin` at `pin_at.pdbs`). The kernels
    /// read the list in place over PCIe — 64 × 8 bytes — so no upload node is needed. The slot
    /// list, the verdict and its codes are read the same way.
    pdbs: DevBuf,
    slots: DevBuf,
    ack: DevBuf,
    ack_code: DevBuf,
    /// ★ The verdict the next [`WalkKernel::submit`] hands the commit node, and the slots it
    /// empties ([`WalkKernel::ack`], [`WalkKernel::reset_slot`]).
    pending_ack: Option<(u64, Vec<u8>)>,
    pending_resets: Vec<u32>,
    /// ★ §13: the imported store, `(device pointer, length)` — never handed out;
    /// [`WalkKernel::write_store`] / [`WalkKernel::read_store`] bound every access by it.
    store: Option<(CUdeviceptr, u64)>,
    hdr: DevBuf,
    rpdb: DevBuf,
    rrun: DevBuf,
    /// Device name, for the census.
    pub device_name: String,
    /// How long the whole bring-up took, in microseconds.
    pub bring_up_us: u64,
    /// How long `cuModuleLoadData` alone took — the PTX JIT.
    pub jit_us: u64,
}

/// The bytes of a `#[repr(C)]` value, for `cuLaunchKernel`'s by-value parameter.
///
/// ⊘ A copy and not a cast: `launch` needs `&mut [u8]`, and handing it a view over a live
/// `KfArgs` would alias. The copy is 216 bytes once per launch.
fn param_bytes<T: Copy>(v: &T) -> Vec<u8> {
    let n = core::mem::size_of::<T>();
    let mut out = vec![0u8; n];
    // ⊘ A byte-wise copy through a `[u8]` view of ONE value, written with safe code so this
    // file stays free of `unsafe`. `KfArgs` is a `#[repr(C)]` aggregate of integers, so its
    // bytes are all initialised and none of them is a pointer Rust tracks.
    let src: &[u8] = bytes_of(v);
    out.copy_from_slice(src);
    out
}

/// A `&[u8]` over one `Copy` `#[repr(C)]` value.
fn bytes_of<T: Copy>(v: &T) -> &[u8] {
    // ⊘ `core::slice::from_raw_parts` is the usual spelling and it is `unsafe`. This crate
    // keeps `unsafe` in `driver_unsafe.rs`, so the conversion goes through the audited file.
    crate::driver_unsafe::view_bytes(v)
}

impl WalkKernel {
    /// ★★★ Bring CUDA all the way up and load the kernel.
    ///
    /// # Errors
    /// [`CudaError`], naming the call that refused. ⊘ In particular a [`KfFormat`] whose
    /// `abi_version` is not [`KF_ABI_VERSION`] is refused **here, before the library is even
    /// opened**, which is the whole of §21's *"a Rust/PTX skew must fail loudly at launch, not
    /// decode garbage field offsets and look like a page-table bug"*.
    pub fn bring_up(cfg: WalkCfg, fmt: KfFormat) -> Result<WalkKernel, CudaError> {
        let t0 = std::time::Instant::now();
        // ★★★★★ THE ABI GATE, and it is FIRST — before `dlopen`, so a skew can never be
        // mistaken for a CUDA problem or masked by one.
        if fmt.abi_version != KF_ABI_VERSION {
            return Err(CudaError::Refused {
                what: "the walk kernel's setup data (abi_version)",
                code: i32::try_from(fmt.abi_version).unwrap_or(-1),
                name: format!(
                    "this build speaks KfFormat abi_version {KF_ABI_VERSION} and was handed \
                     {}; the descriptor's field offsets would be read at the wrong places and \
                     every mapping would come out wrong in a way that reads as a page-table \
                     bug. REFUSED at launch, by name.",
                    fmt.abi_version
                ),
            });
        }
        // ★ VER3 (Hopper, Blackwell) is ACCEPTED (w826, owner: every family first-class). Its
        // descriptor is pinned byte for byte against the `.cu` (`kf_format_ver3`), every field
        // re-checked against ogkm-580's `hopper/gh100/dev_mmu.h`, and `kf-gate7` walks REAL VER3
        // tables on any GPU — the walker decodes guest bytes; the host's own MMU never sees them.
        // ⚠ KNOWN GAP (w826): the .cu's big-PTE veto (`kf_big_pte_unmapped`, the w826 ct4 fix) is
        // VER2-only. VER3 spells "no valid 4 KiB page under this big PTE" as `PCF = 0x3`
        // (`NV_MMU_VER3_PTE_PCF_NO_VALID_4KB_PAGE`, `gh100/dev_mmu.h:140`), which the kernel does
        // not yet test — a Hopper guest mixing big and small pages in one 2 MiB slot is the case.

        let cu = Cuda::open()?;
        cu.init()?;
        let count = cu.device_count()?;
        if count < 1 {
            return Err(CudaError::Refused {
                what: "cuDeviceGetCount",
                code: 0,
                name: "the driver loaded and reports ZERO devices — a fact about this host, \
                       not about CUDA"
                    .to_string(),
            });
        }
        let dev_ord = cu.device_get(0)?;
        let device_name = cu.device_name(dev_ord);
        let ctx = cu.ctx_create(dev_ord)?;

        // ★★★ THE PTX JIT RUNS HERE. Timed on its own because it is the single most expensive
        // lazy path CUDA has, and §w724d's argument is that it must happen while paths exist.
        let tj = std::time::Instant::now();
        let module = {
            // `cuModuleLoadData` reads until a NUL. The committed PTX is text and is stored
            // without one — a trailing NUL in a checked-in text file is an invitation to lose
            // it — so the terminator is added here, where losing it would be a compile error.
            let mut v = WALK_PTX.to_vec();
            v.push(0);
            cu.module_load(&v)?
        };
        let jit_us = u64::try_from(tj.elapsed().as_micros()).unwrap_or(u64::MAX);

        let f_begin = cu.module_function(module, SYM_BEGIN)?;
        let f_walk = cu.module_function(module, SYM_WALK)?;
        let f_diff_slots = cu.module_function(module, SYM_DIFF_SLOTS)?;
        let f_diff_emit = cu.module_function(module, SYM_DIFF_EMIT)?;
        let f_commit = cu.module_function(module, SYM_COMMIT)?;
        let mut f_par = [f_begin; 9];
        for (i, sym) in SYM_PAR.iter().enumerate() {
            f_par[i] = cu.module_function(module, sym)?;
        }

        // ★ The diff's scratch reuses the walk's run stage: 4 × runs_per_pdb runs per entry, and
        // the class partition packs counts in 16 bits.
        let rpp = cfg.runs_per_pdb as usize;
        if KF_MAX_PDB * 4 * rpp > KF_MAX_SCRATCH || rpp > 0xFFFF || rpp == 0 || cfg.max_slots == 0 {
            return Err(CudaError::Refused {
                what: "WalkKernel::bring_up (WalkCfg)",
                code: 0,
                name: format!(
                    "runs_per_pdb {rpp} / max_slots {} do not fit the diff's scratch ({KF_MAX_SCRATCH} \
                     runs for {KF_MAX_PDB} entries × 4) or the 16-bit class partition",
                    cfg.max_slots
                ),
            });
        }
        let tbl_runs = rpp * KF_MAX_PDB;
        let a = |bytes: usize, what: &'static str| -> Result<DevBuf, CudaError> {
            Ok(DevBuf {
                ptr: cu.mem_alloc_zeroed(bytes, what)?,
            })
        };
        let dev = a(core::mem::size_of::<KfDev>(), "cuMemAlloc(KfDev)")?;
        let walk = a(tbl_runs * core::mem::size_of::<KfMapRun>(), "cuMemAlloc(walk)")?;
        let com = a(cfg.max_slots as usize * rpp * core::mem::size_of::<KfMapRun>(), "cuMemAlloc(committed)")?;
        let slot = a(cfg.max_slots as usize * core::mem::size_of::<KfSlot>(), "cuMemAlloc(slots)")?;
        let iscratch = a(KF_MAX_PDB * 2 * rpp * 4, "cuMemAlloc(iscratch)")?;
        // ★ P4b: the report (header, pdb entries, runs) is ONE device allocation laid out
        // exactly as its pinned read-back (`PinLayout`, from `hdr` on), so the read-back is ONE
        // copy node rather than three — each node is paid again on every `cuGraphLaunch`.
        let pin_at = PinLayout::for_cfg(&cfg);
        let report = a(pin_at.total - pin_at.hdr, "cuMemAlloc(report)")?;
        let at_rep = |off: usize| DevBuf {
            ptr: report.ptr + (off - pin_at.hdr) as u64,
        };
        let (hdr, rpdb, rrun) = (at_rep(pin_at.hdr), at_rep(pin_at.rpdb), at_rep(pin_at.rrun));

        // The device-side configuration, written exactly as the `.cu`'s `kf_create` does.
        // ⊘ Same padding hazard as `args_for`: `KfDev` carries 4 uninitialised bytes and is
        // memcpy'd to the device. `..Default::default()` fills FIELDS, never padding.
        let par = ParBufs {
            fr: [
                a(KF_MAX_FRONTIER * KF_ENT_BYTES, "cuMemAlloc(par.fr0)")?,
                a(KF_MAX_FRONTIER * KF_ENT_BYTES, "cuMemAlloc(par.fr1)")?,
            ],
            stage: a(KF_MAX_FRONTIER * KF_ENT_BYTES, "cuMemAlloc(par.stage)")?,
            task: a(KF_MAX_FRONTIER * KF_ENT_BYTES, "cuMemAlloc(par.task)")?,
            runstage: a(KF_MAX_SCRATCH * core::mem::size_of::<KfMapRun>(), "cuMemAlloc(par.runstage)")?,
            cnt: a(KF_MAX_FRONTIER * 4, "cuMemAlloc(par.cnt)")?,
            off: a(KF_MAX_FRONTIER * 4, "cuMemAlloc(par.off)")?,
            start: a(KF_MAX_FRONTIER * 4, "cuMemAlloc(par.start)")?,
            nfr: a(4 * 4, "cuMemAlloc(par.nfr)")?,
            pdbbase: a(KF_MAX_PDB * 4, "cuMemAlloc(par.pdbbase)")?,
            used: a(KF_USED_SLOTS * 4, "cuMemAlloc(par.used)")?,
            sum: a(KF_MAX_FRONTIER * KF_SUM_BYTES, "cuMemAlloc(par.sum)")?,
            head: a(KF_MAX_FRONTIER, "cuMemAlloc(par.head)")?,
        };
        let mut h: KfDev = crate::driver_unsafe::zeroed();
        let h = KfDev {
            runs_per_pdb: cfg.runs_per_pdb,
            entry_budget: cfg.entry_budget,
            run_capacity: cfg.run_capacity,
            pdb_capacity: cfg.pdb_capacity,
            max_pdbs: u32::try_from(KF_MAX_PDB).unwrap_or(64),
            max_slots: cfg.max_slots,
            ..h
        };
        cu.memcpy_h2d(dev.ptr, bytes_of(&h), "cuMemcpyHtoD(KfDev)")?;

        // ★ P4: the asynchronous half, brought up here with everything else lazy (§w724d).
        let stream = cu.stream_create()?;
        let ev_start = cu.event_create()?;
        let ev_copied = cu.event_create()?;
        let ev_done = cu.event_create()?;
        let pin = cu.pinned_alloc(pin_at.total, "cuMemAllocHost(report read-back)")?;
        let pdbs = DevBuf {
            ptr: cu.pinned_device_ptr(&pin, pin_at.pdbs)?,
        };
        let slots = DevBuf { ptr: cu.pinned_device_ptr(&pin, pin_at.slots)? };
        let ack = DevBuf { ptr: cu.pinned_device_ptr(&pin, pin_at.ack)? };
        let ack_code = DevBuf { ptr: cu.pinned_device_ptr(&pin, pin_at.ack_code)? };
        let done_fd = CompletionFd::new()?;

        let mut k = WalkKernel {
            cu,
            ctx,
            f_begin,
            f_walk,
            f_diff_slots,
            f_diff_emit,
            f_commit,
            f_par,
            stream,
            ev_start,
            ev_copied,
            ev_done,
            pin,
            pin_at,
            done_fd,
            inflight: None,
            graph: None,
            par,
            fmt,
            cfg,
            dev,
            walk,
            com,
            slot,
            iscratch,
            pdbs,
            slots,
            ack,
            ack_code,
            pending_ack: None,
            pending_resets: Vec::new(),
            store: None,
            hdr,
            rpdb,
            rrun,
            device_name,
            bring_up_us: 0,
            jit_us,
        };
        // ★ P4b: capture + instantiate the walk graph HERE, with every other lazy path
        // (§w724d) — not on the first submit, after the sandbox may have closed paths.
        k.graph = k.build_graph()?;
        k.warm_up()?;
        k.bring_up_us = u64::try_from(t0.elapsed().as_micros()).unwrap_or(u64::MAX);
        Ok(k)
    }

    /// ★ P4b: pay the two remaining first-use costs HERE rather than on the first walk's
    /// submitting thread. `[measured GA106, w827]` the first walk's `cuGraphLaunch` cost
    /// 66-130 µs (graph upload) and its first `cuLaunchHostFunc` ~260 µs (the driver starts
    /// its callback thread lazily). ⇒ `cuGraphUpload`, then one host signal on the stream,
    /// waited for on the fd and drained so it cannot complete the first real walk.
    /// ⊘ The wait is `poll(2)` on the fd, at bring-up — not a context synchronize, and not on
    /// the walk path.
    fn warm_up(&self) -> Result<(), CudaError> {
        let s = self.stream;
        if let Some(g) = &self.graph {
            self.cu.graph_upload(g.exec, s)?;
        }
        self.cu.launch_host_signal(s, &self.done_fd)?;
        if !self.done_fd.wait_readable(10_000) || self.done_fd.drain() == 0 {
            return Err(CudaError::Refused {
                what: "WalkKernel::bring_up (warm-up host signal)",
                code: 0,
                name: "the warm-up host function did not signal the completion fd within 10 s"
                    .to_string(),
            });
        }
        Ok(())
    }

    /// Capture the whole walk on the walker's stream (placeholder `KfArgs`: no store, no
    /// spaces — the first `submit` rewrites them) and instantiate it. `Ok(None)` when the
    /// driver lacks the graph API; any failure of a capture the API claimed to support is a
    /// refusal, never a silent fallback to per-launch submission.
    fn build_graph(&self) -> Result<Option<WalkGraph>, CudaError> {
        if !self.cu.has_graph_api() {
            return Ok(None);
        }
        let s = self.stream;
        let args = self.args_for(0, 0, 0);
        self.cu.stream_begin_capture(s)?;
        let mut rec = Some(Vec::new());
        let queued = self.enqueue_walk(&args, 0, &mut rec);
        // ⊘ End the capture on EVERY path: a stream left capturing records every later walk.
        let ended = self.cu.stream_end_capture(s);
        let graph = match (queued, ended) {
            (Ok(()), Ok(g)) => g,
            (Err(e), Ok(g)) => {
                self.cu.graph_destroy(g);
                return Err(e);
            }
            (Err(e), Err(_)) | (Ok(()), Err(e)) => return Err(e),
        };
        let exec = match self.cu.graph_instantiate(graph) {
            Ok(e) => e,
            Err(e) => {
                self.cu.graph_destroy(graph);
                return Err(e);
            }
        };
        Ok(Some(WalkGraph {
            graph,
            exec,
            kernels: rec.unwrap_or_default(),
            baked_args: param_bytes(&args),
            baked_npdb: 0,
            updates: 0,
        }))
    }

    fn args_for(&self, gpga: CUdeviceptr, gpga_len: u64, npdb: u32) -> KfArgs {
        // ★★★★★ **w761a — ZEROED FIRST, THEN ASSIGNED. The padding is the point.**
        //
        // ⊘⊘⊘ `KfArgs` carries **8 uninitialised padding bytes** (`npdb`→`scopes` and
        // `nscope`→`hdr`; `abi.rs:312` names the hole) and a struct literal never writes them
        // — Rust only writes fields. This value is then handed to `view_bytes` and on to
        // `cuLaunchKernel` on **every refresh**, so the launch read uninitialised memory every
        // time. That is live UB, not a latent one, and it is the same defect w728 already paid
        // for (`rust_struct_padding_crosses_the_abi_uninitialised`): a fix applied to the
        // instance left the class.
        //
        // ⚠ `zeroed()` exists in this crate for exactly this and had been applied only to
        // `KfFormat` (`abi.rs:425`). ⊘ Sound HERE because `KfArgs` is a `#[repr(C)]` aggregate
        // of integers and pointers with no niche — the generic signature's unsoundness is a
        // separate finding and is not made worse by a correct use.
        let mut a: KfArgs = crate::driver_unsafe::zeroed();
        a.win = crate::abi::KfWin {
            base: gpga,
            len: gpga_len,
                // ★ §39(c): in production these ARE the same number, and saying so here is
                // the point. The single store is the whole of guest vidmem and all of it is
                // mapped, so the bytes we may READ and the addresses a leaf may POINT AT
                // coincide. They are separate fields because that coincidence is a property
                // of THIS deployment, not of the walker -- a corpus image breaks it.
            span: gpga_len,
        };
        a.fmt = self.fmt;
        a.dev = self.dev.ptr;
        a.walk = self.walk.ptr;
        a.com = self.com.ptr;
        a.slot = self.slot.ptr;
        a.pdbs = self.pdbs.ptr;
        a.slots = self.slots.ptr;
        a.npdb = npdb;
        a.ack = self.ack.ptr;
        a.ack_code = self.ack_code.ptr;
        // ⊘ The walk's run stage: done with before the diff runs, free before the commit runs.
        a.scratch = self.par.runstage.ptr;
        a.iscratch = self.iscratch.ptr;
        a.hdr = self.hdr.ptr;
        a.rpdb = self.rpdb.ptr;
        a.rrun = self.rrun.ptr;
        a
    }

    /// ★★★★★ **Queue one walk of `entries` over the imported store, and return.**
    ///
    /// Each entry walks `pdb` and is diffed against slot `slot`'s committed placements. The
    /// graph's first node commits the verdict staged by [`WalkKernel::ack`] on the PREVIOUS
    /// report (and empties the slots [`WalkKernel::reset_slot`] released); then the walk, the
    /// diff, the report read-back into pinned memory, and a host function that signals
    /// [`WalkKernel::completion_fd`]. **Nothing here waits on the GPU.**
    ///
    /// # Errors
    /// [`CudaError::Refused`] naming the call; also refused, by name, when a walk is already in
    /// flight, when no store was imported, when more than [`KF_MAX_PDB`] entries are asked for,
    /// or when a slot is out of range or repeated (two entries may never commit into one slot).
    pub fn submit(&mut self, entries: &[WalkEntry]) -> Result<(), CudaError> {
        let Some((gpga, gpga_len)) = self.store else {
            return Err(refused("WalkKernel::submit", "no store imported (import_store first)".to_string()));
        };
        self.submit_over(gpga, gpga_len, entries)
    }

    /// [`WalkKernel::submit`] over an uploaded image instead of the store (the selftest).
    ///
    /// # Errors
    /// As [`WalkKernel::submit`].
    pub fn submit_image(&mut self, img: &DeviceImage, entries: &[WalkEntry]) -> Result<(), CudaError> {
        self.submit_over(img.ptr, img.len, entries)
    }

    fn submit_over(&mut self, gpga: CUdeviceptr, gpga_len: u64, entries: &[WalkEntry]) -> Result<(), CudaError> {
        if self.inflight.is_some() {
            return Err(refused(
                "WalkKernel::submit",
                "a walk is already in flight; collect it first (one pinned read-back, one walk at a time)".to_string(),
            ));
        }
        if entries.len() > KF_MAX_PDB {
            return Err(refused(
                "WalkKernel::submit",
                format!("the kernel walks {KF_MAX_PDB} entries at once and was handed {}", entries.len()),
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for e in entries {
            if e.slot >= self.cfg.max_slots || !seen.insert(e.slot) {
                return Err(refused(
                    "WalkKernel::submit",
                    format!("slot {} is out of range (max {}) or repeated in one walk", e.slot, self.cfg.max_slots),
                ));
            }
        }
        self.make_current()?;
        let npdb = u32::try_from(entries.len()).unwrap_or(0);
        let mut pdb_bytes = Vec::with_capacity(entries.len() * 8);
        let mut slot_bytes = Vec::with_capacity(entries.len() * 4);
        for e in entries {
            pdb_bytes.extend_from_slice(&e.pdb.to_le_bytes());
            slot_bytes.extend_from_slice(&e.slot.to_le_bytes());
        }
        let s = self.stream;
        let at = self.pin_at;
        // The stages the graph reads in place; the previous walk has been collected (checked
        // above), so nothing is still reading them.
        self.pin.write(at.pdbs, &pdb_bytes);
        self.pin.write(at.slots, &slot_bytes);
        // ★ The verdict on the previous report — or "none" (generation 0), so a verdict is
        // consumed by exactly one walk.
        let mut ack = KfAck::default();
        if let Some((generation, codes)) = self.pending_ack.take() {
            let n = codes.len().min(self.cfg.run_capacity as usize);
            ack.generation = generation;
            ack.nrun = u32::try_from(codes.len()).unwrap_or(u32::MAX);
            self.pin.write(at.ack_code, &codes[..n]);
        }
        let nres = self.pending_resets.len().min(KF_MAX_RESET);
        for (i, r) in self.pending_resets.drain(..nres).enumerate() {
            ack.reset[i] = r;
        }
        ack.nreset = u32::try_from(nres).unwrap_or(0);
        self.pin.write(at.ack, bytes_of(&ack));
        let args = self.args_for(gpga, gpga_len, npdb);
        if let Some(mut g) = self.graph.take() {
            // ★ P4b: ONE driver call for the whole walk, plus a setter per args-bearing node
            // only when the store or the entry count changed since the last walk.
            let r = Self::update_graph(&self.cu, &mut g, &args, npdb)
                .and_then(|()| self.cu.graph_launch(g.exec, s));
            self.graph = Some(g);
            r?;
        } else {
            self.enqueue_walk(&args, npdb, &mut None)?;
        }
        self.inflight = Some(InFlight {
            gpga_len,
            submitted: std::time::Instant::now(),
        });
        Ok(())
    }

    /// ★★★★★ **The host's verdict on the report just collected** — one [`crate::abi::KFWR_ACK_FAILED`]
    /// / `APPLIED` / `HELD` byte per run, in report order. Staged; the next
    /// [`WalkKernel::submit`]'s first node commits it (COMMIT-ON-ACK). A walk submitted without
    /// one commits nothing, which is always safe: the next diff is then computed against the same
    /// placements.
    ///
    /// # Errors
    /// Refused while a walk is in flight (the verdict must name the report that walk will read).
    pub fn ack(&mut self, generation: u64, codes: Vec<u8>) -> Result<(), CudaError> {
        if self.inflight.is_some() {
            return Err(refused("WalkKernel::ack", "a walk is in flight; the verdict answers the COLLECTED report".to_string()));
        }
        self.pending_ack = Some((generation, codes));
        Ok(())
    }

    /// ★ Release `slot` (its VA-space object is gone): the next walk empties it before anything
    /// is diffed against it, and never commits into it in the same walk.
    ///
    /// # Errors
    /// Refused when the slot is out of range or [`KF_MAX_RESET`] releases are already pending.
    pub fn reset_slot(&mut self, slot: u32) -> Result<(), CudaError> {
        if slot >= self.cfg.max_slots || self.pending_resets.len() >= KF_MAX_RESET {
            return Err(refused("WalkKernel::reset_slot", format!("slot {slot} (max {}) or too many pending", self.cfg.max_slots)));
        }
        if !self.pending_resets.contains(&slot) {
            self.pending_resets.push(slot);
        }
        Ok(())
    }

    /// Committed-placement slots this kernel holds.
    #[must_use]
    pub fn max_slots(&self) -> u32 {
        self.cfg.max_slots
    }

    /// Whether the GPU has finished the walk in flight, by `cuEventQuery` — **non-consuming and
    /// non-blocking**. `Ok(false)` with a walk in flight right after [`WalkKernel::submit`]
    /// returned is the measured proof that `submit` did not wait for the GPU (gate 8).
    ///
    /// # Errors
    /// The stream's failure, by name.
    pub fn gpu_done_now(&self) -> Result<bool, CudaError> {
        if self.inflight.is_none() {
            return Ok(true);
        }
        self.cu.event_query(self.ev_done)
    }

    /// Whether a walk is queued and not yet collected.
    #[must_use]
    pub fn in_flight(&self) -> bool {
        self.inflight.is_some()
    }

    /// ★ The fd a walk's completion is signalled on — put it in the worker's `epoll` set.
    #[must_use]
    pub fn completion_fd(&self) -> &CompletionFd {
        &self.done_fd
    }

    /// `cuCtxSynchronize` calls made by this walker's binding since bring-up (see
    /// [`Cuda::ctx_sync_calls`]). A walk adds **zero**.
    #[must_use]
    pub fn ctx_sync_calls(&self) -> u64 {
        self.cu.ctx_sync_calls()
    }

    /// ★ P4b: whether [`WalkKernel::submit`] queues the walk as ONE `cuGraphLaunch` (`true`) or
    /// launch by launch (`false`: this driver has no graph API). Gate 8 prints it beside the
    /// submit cost, so a fallback cannot pass as the fast path.
    #[must_use]
    pub fn submits_as_graph(&self) -> bool {
        self.graph.is_some()
    }

    /// ★ P4b: how many walks had to rewrite the graph's by-value parameters (the store moved
    /// or the address-space count changed). `0` extra per walk is the steady state.
    #[must_use]
    pub fn graph_param_updates(&self) -> u64 {
        self.graph.as_ref().map_or(0, |g| g.updates)
    }

    /// ★★★ **Collect the walk in flight if it has finished — never blocks.**
    ///
    /// `Ok(None)`: nothing in flight, or not done yet. `Ok(Some(_))`: done; the report was
    /// decoded from pinned memory. `Err`: the stream failed (a device fault in the walk surfaces
    /// here, named by `cuEventQuery`), and the walk is dropped.
    ///
    /// # Errors
    /// [`CudaError`].
    pub fn try_collect(&mut self) -> Result<Option<Collected>, CudaError> {
        let Some(f) = self.inflight else {
            return Ok(None);
        };
        if self.done_fd.drain() == 0 {
            // The fd is silent. Ask the event, so a failed stream — whose host function may
            // never run — is named rather than waited on forever.
            match self.cu.event_query(self.ev_done) {
                Ok(false) => return Ok(None),
                Ok(true) => {
                    // `ev_done` follows the host signal in the stream, so the fd was written;
                    // consume it now so it cannot complete the next walk.
                    let _ = self.done_fd.drain();
                }
                Err(e) => {
                    self.inflight = None;
                    return Err(e);
                }
            }
        }
        self.inflight = None;
        let submit_to_collect_us = u64::try_from(f.submitted.elapsed().as_micros()).unwrap_or(u64::MAX);
        let gpu_us = self.cu.event_elapsed_us(self.ev_start, self.ev_copied).unwrap_or(0);
        let at = self.pin_at;
        let hb = self.pin.read(at.hdr, core::mem::size_of::<KfReportHeader>());
        let header = crate::driver_unsafe::read_struct::<KfReportHeader>(&hb);
        // ⊘ Clamped to the CAPACITY: a truncated report legitimately declares more than it
        // carries (invariant I3), and reading `run_count` elements out of a `run_capacity`
        // buffer would turn "loud truncation" into a host-side overrun.
        let npdb = header.pdb_count.min(self.cfg.pdb_capacity) as usize;
        let nrun = header.run_count.min(self.cfg.run_capacity) as usize;
        let pb = self.pin.read(at.rpdb, npdb * core::mem::size_of::<KfPdbEntry>());
        let pdbs = pb
            .chunks_exact(core::mem::size_of::<KfPdbEntry>())
            .map(crate::driver_unsafe::read_struct::<KfPdbEntry>)
            .collect();
        let rb = self.pin.read(at.rrun, nrun * core::mem::size_of::<KfMapRun>());
        let runs = rb
            .chunks_exact(core::mem::size_of::<KfMapRun>())
            .map(crate::driver_unsafe::read_struct::<KfMapRun>)
            .collect();
        Ok(Some(Collected {
            report: Report {
                header,
                pdbs,
                runs,
                gpga_span: f.gpga_len,
            },
            gpu_us,
            submit_to_collect_us,
        }))
    }

    /// ⚠ **HARNESS / SELFTEST ONLY — this blocks the calling thread** (in `poll(2)` on the
    /// completion fd, never in `cuCtxSynchronize`). A worker uses [`WalkKernel::submit`] and
    /// [`WalkKernel::try_collect`] from its `epoll` loop instead.
    ///
    /// # Errors
    /// [`CudaError`], or a refusal naming the timeout.
    pub fn wait(&mut self, timeout_ms: u64) -> Result<Collected, CudaError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            if let Some(c) = self.try_collect()? {
                return Ok(c);
            }
            if !self.in_flight() {
                return Err(CudaError::Refused {
                    what: "WalkKernel::wait",
                    code: 0,
                    name: "no walk in flight".to_string(),
                });
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err(CudaError::Refused {
                    what: "WalkKernel::wait",
                    code: 0,
                    name: format!("the walk did not complete within {timeout_ms} ms"),
                });
            }
            // ⊘ Short slices, so a failed stream (no host function will ever run) is named by
            // the next `try_collect`'s event query rather than waited out.
            let slice = i32::try_from(left.as_millis().min(50)).unwrap_or(50);
            let _ = self.done_fd.wait_readable(slice);
        }
    }

    /// One walk, start to finish: [`WalkKernel::submit`] then [`WalkKernel::wait`] (10 s).
    /// ⚠ **Blocks the caller** — harnesses and the selftest only (see [`WalkKernel::wait`]).
    ///
    /// # Errors
    /// [`CudaError`], naming the call that refused.
    pub fn refresh(&mut self, entries: &[WalkEntry]) -> Result<Report, CudaError> {
        self.submit(entries)?;
        Ok(self.wait(10_000)?.report)
    }

    /// [`WalkKernel::refresh`] over an uploaded image (the selftest). ⚠ Blocks the caller.
    ///
    /// # Errors
    /// [`CudaError`].
    pub fn refresh_image(&mut self, img: &DeviceImage, entries: &[WalkEntry]) -> Result<Report, CudaError> {
        self.submit_image(img, entries)?;
        Ok(self.wait(10_000)?.report)
    }

    /// ★ P4b — bring the instantiated graph's by-value parameters up to `args`/`npdb`. A no-op
    /// (zero driver calls) when they already match, which is every walk in steady state.
    fn update_graph(cu: &Cuda, g: &mut WalkGraph, args: &KfArgs, npdb: u32) -> Result<(), CudaError> {
        let ab = param_bytes(args);
        if ab == g.baked_args && npdb == g.baked_npdb {
            return Ok(());
        }
        // Unknown until every node has taken the new values: a failure part-way leaves some
        // nodes old and some new, and the next walk must rewrite them all.
        g.baked_args.clear();
        for k in g.kernels.iter_mut().filter(|k| k.takes_args) {
            k.params[0].clone_from(&ab);
            if k.seed {
                k.grid = npdb.div_ceil(128).max(1);
            }
            cu.graph_exec_kernel_set(g.exec, k.node, k.f, k.grid, k.block, k.shm, &mut k.params, k.what)?;
        }
        g.baked_args = ab;
        g.baked_npdb = npdb;
        g.updates += 1;
        Ok(())
    }

    /// Queue (or, under capture, record) the whole walk on the walker's stream: pdb upload,
    /// `kf_begin_kernel`, the parallel walk, the diff, the report read-back. `rec` is
    /// `Some` exactly while the stream is capturing; each kernel launch is then recorded with
    /// its graph node so its parameters can be updated later.
    fn enqueue_walk(
        &self,
        args: &KfArgs,
        npdb: u32,
        rec: &mut Option<Vec<GraphKernel>>,
    ) -> Result<(), CudaError> {
        let s = self.stream;
        let at = self.pin_at;
        self.record(self.ev_start, rec.is_some())?;
        // ⊘ No pdb upload: `a.pdbs` IS the pinned stage `submit` wrote (see `pdbs`). Every
        // level's staging cursor is zeroed here, once (see `KF_USED_SLOTS`).
        self.cu.memset_d8_async(s, self.par.used.ptr, 0, KF_USED_SLOTS * 4, "cuMemsetD8Async(used)")?;
        // ★ COMMIT-ON-ACK first: the previous report and its verdict, before anything is walked
        // (the commit uses the run stage as scratch, which the walk then overwrites).
        let grid = u32::try_from(KF_MAX_PDB).unwrap_or(64);
        self.kl(rec, self.f_commit, grid, KF_DIFF_BLOCK, 0, vec![param_bytes(args)], "cuLaunchKernel(kf_commit_kernel)")?;
        self.kl(rec, self.f_begin, 1, 1, 0, vec![param_bytes(args)], "cuLaunchKernel(kf_begin_kernel)")?;
        // ★★★★★ w826 — THE PARALLEL WALK (`kf_run_parallel`, ported to the driver API).
        // `[w726]` ~0.6 ms fixed cost vs the serial walk's one-thread-per-space. The serial
        // kernel stays in the PTX for the scoped path the .cu keeps; this walk is never scoped.
        let _ = self.f_walk;
        self.run_parallel(args, npdb, rec)?;
        // ★ The diff against the committed placements (one block per entry), then the dense report.
        self.kl(rec, self.f_diff_slots, grid, KF_DIFF_BLOCK, 0, vec![param_bytes(args)], "cuLaunchKernel(kf_diff_slots)")?;
        self.kl(rec, self.f_diff_emit, grid, 256, 0, vec![param_bytes(args)], "cuLaunchKernel(kf_diff_emit)")?;
        // One copy of the whole report block (`hdr`, `rpdb`, `rrun` are one allocation laid
        // out as the pinned buffer from `at.hdr` on; see `bring_up`).
        self.cu.memcpy_d2h_async(s, &self.pin, at.hdr, self.hdr.ptr, at.total - at.hdr, "cuMemcpyDtoHAsync(report)")?;
        let capturing = rec.is_some();
        self.record(self.ev_copied, capturing)?;
        // ⊘ ORDER: the host signal BEFORE `ev_done`. Then `ev_done` complete ⇒ the fd was
        // written, so a collect that saw the event can always drain the signal — a signal left
        // behind would complete the NEXT walk before it ran. Under capture the host function
        // becomes a HOST NODE of the graph: it runs on every replay, same fd, same order.
        self.cu.launch_host_signal(s, &self.done_fd)?;
        self.record(self.ev_done, capturing)
    }

    /// `cuEventRecord`, or — under capture — the external record that becomes a graph node.
    fn record(&self, e: EventHandle, capturing: bool) -> Result<(), CudaError> {
        if capturing {
            self.cu.event_record_external(e, self.stream)
        } else {
            self.cu.event_record(e, self.stream)
        }
    }

    /// One kernel launch on the walker's stream; under capture, also records its node.
    #[allow(clippy::too_many_arguments)]
    fn kl(
        &self,
        rec: &mut Option<Vec<GraphKernel>>,
        f: Func,
        grid: u32,
        block: u32,
        shm: u32,
        mut params: Vec<Vec<u8>>,
        what: &'static str,
    ) -> Result<(), CudaError> {
        self.cu.launch_args(self.stream, f, grid, block, shm, &mut params, what)?;
        if let Some(v) = rec {
            let node = self.cu.capture_leaf(self.stream)?;
            v.push(GraphKernel {
                node,
                f,
                grid,
                block,
                shm,
                params,
                what,
                takes_args: f != self.f_par[2],
                seed: f == self.f_par[0],
            });
        }
        Ok(())
    }

    fn run_parallel(
        &self,
        a: &KfArgs,
        npdb: u32,
        rec: &mut Option<Vec<GraphKernel>>,
    ) -> Result<(), CudaError> {
        let p = |v: u64| v.to_le_bytes().to_vec();
        let u = |v: u32| v.to_le_bytes().to_vec();
        let ab = || param_bytes(a);
        let par = &self.par;
        let shm = (KF_PAR_BLOCK / KF_WARP) * KF_SHWORDS * 8;
        let [f_seed, f_expand, f_scan, f_compact, f_leaf, f_heads, f_bases, f_emit, f_join] =
            self.f_par;
        self.kl(
            rec,
            f_seed,
            npdb.div_ceil(128).max(1),
            128,
            0,
            vec![ab(), p(par.fr[0].ptr), p(par.nfr.ptr), p(par.used.ptr)],
            "cuLaunchKernel(kf_par_seed)",
        )?;
        let mut src = 0usize;
        // The last level's output count IS the task count; it is read where it lies (no copy).
        let mut ntask = par.nfr.ptr;
        for k in u32::from(self.fmt.first_dir)..KF_DIRS {
            let used = par.used.ptr + u64::from(k) * 4;
            let nin = par.nfr.ptr + (src as u64) * 4;
            let nout = par.nfr.ptr + ((src ^ 1) as u64) * 4;
            let dst = if k + 1 < KF_DIRS { par.fr[src ^ 1].ptr } else { par.task.ptr };
            self.kl(
                rec,
                f_expand,
                KF_PAR_GRID,
                KF_PAR_BLOCK,
                shm,
                vec![
                    ab(),
                    u(k),
                    p(par.fr[src].ptr),
                    p(nin),
                    p(par.stage.ptr),
                    u(KF_MAX_FRONTIER as u32),
                    p(used),
                    p(par.start.ptr),
                    p(par.cnt.ptr),
                ],
                "cuLaunchKernel(kf_par_expand)",
            )?;
            self.kl(
                rec,
                f_scan,
                1,
                KF_SCAN_BLOCK,
                0,
                vec![p(par.cnt.ptr), p(nin), p(par.off.ptr), p(nout)],
                "cuLaunchKernel(kf_par_scan)",
            )?;
            self.kl(
                rec,
                f_compact,
                KF_PAR_GRID,
                KF_PAR_BLOCK,
                0,
                vec![
                    ab(),
                    p(par.stage.ptr),
                    p(par.start.ptr),
                    p(par.cnt.ptr),
                    p(par.off.ptr),
                    p(nin),
                    p(dst),
                    u(KF_MAX_FRONTIER as u32),
                ],
                "cuLaunchKernel(kf_par_compact)",
            )?;
            if k + 1 < KF_DIRS {
                src ^= 1;
            } else {
                ntask = nout;
            }
        }
        self.kl(
            rec,
            f_leaf,
            KF_PAR_GRID,
            KF_PAR_BLOCK,
            shm,
            vec![
                ab(),
                p(par.task.ptr),
                p(ntask),
                p(par.runstage.ptr),
                u(KF_MAX_SCRATCH as u32),
                p(par.used.ptr + u64::from(KF_DIRS) * 4),
                p(par.sum.ptr),
            ],
            "cuLaunchKernel(kf_par_leaf)",
        )?;
        self.kl(
            rec,
            f_heads,
            KF_PAR_GRID,
            128,
            0,
            vec![
                ab(),
                p(par.task.ptr),
                p(par.sum.ptr),
                p(ntask),
                p(par.cnt.ptr),
                p(par.head.ptr),
            ],
            "cuLaunchKernel(kf_par_heads)",
        )?;
        self.kl(
            rec,
            f_scan,
            1,
            KF_SCAN_BLOCK,
            0,
            vec![p(par.cnt.ptr), p(ntask), p(par.off.ptr), p(par.nfr.ptr + 12)],
            "cuLaunchKernel(kf_par_scan tasks)",
        )?;
        self.kl(
            rec,
            f_bases,
            1,
            1,
            0,
            vec![ab(), p(par.pdbbase.ptr)],
            "cuLaunchKernel(kf_par_bases)",
        )?;
        self.kl(
            rec,
            f_emit,
            KF_PAR_GRID,
            KF_PAR_BLOCK,
            0,
            vec![
                ab(),
                p(par.task.ptr),
                p(par.sum.ptr),
                p(ntask),
                p(par.off.ptr),
                p(par.head.ptr),
                p(par.pdbbase.ptr),
                p(par.runstage.ptr),
            ],
            "cuLaunchKernel(kf_par_emit)",
        )?;
        self.kl(
            rec,
            f_join,
            KF_PAR_GRID,
            128,
            0,
            vec![
                ab(),
                p(par.task.ptr),
                p(par.sum.ptr),
                p(ntask),
                p(par.off.ptr),
                p(par.head.ptr),
                p(par.pdbbase.ptr),
            ],
            "cuLaunchKernel(kf_par_join)",
        )
    }

    /// ★★★★★ **MAKE THIS KERNEL'S CONTEXT CURRENT ON THE CALLING THREAD.**
    ///
    /// ⊘ **Required before any call from a thread that did not bring CUDA up.** The context is
    /// per-thread current; see [`crate::driver_unsafe::Cuda::ctx_set_current`] for the boot
    /// this cost. Idempotent and cheap — call it at the top of every entry point that can be
    /// reached from a worker.
    ///
    /// # Errors
    /// [`CudaError`].
    pub fn make_current(&self) -> Result<(), CudaError> {
        self.cu.ctx_set_current(self.ctx)
    }

    /// Copy a host image into fresh device memory.
    ///
    /// # Errors
    /// [`CudaError`].
    pub fn upload(&self, bytes: &[u8]) -> Result<DeviceImage, CudaError> {
        let p = self.cu.mem_alloc_zeroed(bytes.len(), "cuMemAlloc(gpga)")?;
        self.cu.memcpy_h2d(p, bytes, "cuMemcpyHtoD(gpga)")?;
        Ok(DeviceImage {
            ptr: p,
            len: bytes.len() as u64,
        })
    }

    /// ★★★ **Import an RM-exported object into THIS kernel's context** and map it whole.
    ///
    /// ⊘ `[w825]` It must be this context. `arm_store_device_pointer` imports through a
    /// separately-opened `Cuda` handle; a pointer minted in another context is not one this
    /// kernel can dereference, and nothing would say so until a walk read garbage. ⇒ The import
    /// lives on the kernel, with the kernel's context made current first.
    ///
    /// `fd` is the `/dev/nvidiactl` fd RM exported the object to (`w755i`: RM imports and
    /// exports only through a control fd, never a dma-buf).
    ///
    /// ★ §13: the device pointer stays INSIDE the kernel. Every later access to the store is
    /// [`WalkKernel::write_store`] / [`WalkKernel::read_store`], bounded by its length, and every
    /// walk [`WalkKernel::submit`]s over it.
    ///
    /// # Errors
    /// [`CudaError::Refused`] naming the import step that failed, or a second import.
    pub fn import_store(&mut self, fd: i32, bytes: u64) -> Result<(), CudaError> {
        if self.store.is_some() {
            return Err(refused("import_store", "a store is already imported".to_string()));
        }
        self.make_current()?;
        let n = usize::try_from(bytes).map_err(|_| CudaError::Refused {
            what: "import_store: object length does not fit usize",
            code: 0,
            name: format!("{bytes:#x}"),
        })?;
        // ⊘ `import_and_map` reports which of its four steps refused as a string; carried in
        // `name` verbatim so the step is not lost to a generic code.
        let p = self.cu.import_and_map(0, fd, n).map_err(|name| CudaError::Refused {
            what: "cuMemImportFromShareableHandle + cuMemMap",
            code: 0,
            name,
        })?;
        self.store = Some((p, bytes));
        Ok(())
    }

    /// The imported store's length, if one was imported.
    #[must_use]
    pub fn store_len(&self) -> Option<u64> {
        self.store.map(|(_, l)| l)
    }

    /// The store address of `[off, off+len)`, refused by name unless wholly inside the store.
    fn store_at(&self, off: u64, len: usize, what: &'static str) -> Result<CUdeviceptr, CudaError> {
        let Some((base, bytes)) = self.store else {
            return Err(refused(what, "no store imported".to_string()));
        };
        let end = off.checked_add(len as u64);
        if end.is_none_or(|e| e > bytes) {
            return Err(refused(what, format!("[{off:#x}, +{len:#x}) leaves the {bytes:#x}-byte store")));
        }
        Ok(base + off)
    }

    /// ★ §13: write `bytes` at store offset `off` — bounds-checked against the store's length.
    /// ⊘ Synchronous, on the legacy stream, which the walker's blocking stream is ordered behind:
    /// a walk submitted after this returns reads these bytes.
    ///
    /// # Errors
    /// Refused by name outside the store; the CUDA error otherwise.
    pub fn write_store(&self, off: u64, bytes: &[u8]) -> Result<(), CudaError> {
        let dst = self.store_at(off, bytes.len(), "WalkKernel::write_store")?;
        self.cu.memcpy_h2d(dst, bytes, "cuMemcpyHtoD(write_store)")
    }

    /// ★ §13: read `buf.len()` bytes at store offset `off` (a harness check, never a data path) —
    /// bounds-checked against the store's length.
    ///
    /// # Errors
    /// Refused by name outside the store; the CUDA error otherwise.
    pub fn read_store(&self, off: u64, buf: &mut [u8]) -> Result<(), CudaError> {
        let src = self.store_at(off, buf.len(), "WalkKernel::read_store")?;
        self.cu.memcpy_d2h(buf, src, "cuMemcpyDtoH(read_store)")
    }

    /// Read `buf.len()` bytes of an uploaded image at `off`, bounds-checked against it.
    ///
    /// # Errors
    /// Refused by name outside the image; the CUDA error otherwise.
    pub fn read_image(&self, img: &DeviceImage, off: u64, buf: &mut [u8]) -> Result<(), CudaError> {
        if off.checked_add(buf.len() as u64).is_none_or(|e| e > img.len) {
            return Err(refused("WalkKernel::read_image", format!("[{off:#x}, +{:#x}) leaves the image", buf.len())));
        }
        self.cu.memcpy_d2h(buf, img.ptr + off, "cuMemcpyDtoH(read_image)")
    }

    /// Write `bytes` into an uploaded image at `off`, bounds-checked against it.
    ///
    /// # Errors
    /// Refused by name outside the image; the CUDA error otherwise.
    pub fn write_image(&self, img: &DeviceImage, off: u64, bytes: &[u8]) -> Result<(), CudaError> {
        if off.checked_add(bytes.len() as u64).is_none_or(|e| e > img.len) {
            return Err(refused("WalkKernel::write_image", format!("[{off:#x}, +{:#x}) leaves the image", bytes.len())));
        }
        self.cu.memcpy_h2d(img.ptr + off, bytes, "cuMemcpyHtoD(write_image)")
    }

    /// Release an image returned by [`WalkKernel::upload`].
    pub fn release(&self, img: DeviceImage) {
        self.cu.mem_free(img.ptr);
    }

    /// ★★ **A launch that must FAIL** — probe (b) of `THE_CONSTRAINTS.md` §w724d.
    ///
    /// Asks for a **2048-thread block**, above the architectural maximum on every part that
    /// exists, so the **driver** refuses at launch rather than the device faulting. The point
    /// is not the mechanism: it is that the driver must be able to *report* a refusal after
    /// the sandbox is entered, without reopening anything by path.
    ///
    /// Returns `Some(why)` when it refused as intended, and `None` when the malformed launch
    /// unexpectedly **succeeded** — which is a finding and must not read as a passing probe.
    pub fn probe_failed_launch(&mut self) -> Option<String> {
        let args = self.args_for(0, 0, 0);
        let mut p = param_bytes(&args);
        let r = self.cu.launch_raw(self.f_walk, 1, 2048, &mut p);
        if r == crate::driver_unsafe::CUDA_SUCCESS {
            // ⊘ Drain it, so a surprising success cannot leave work in flight that the next
            // probe would then be blamed for.
            let _ = self.cu.ctx_synchronize();
            return None;
        }
        match self.cu.check("cuLaunchKernel(deliberately malformed)", r) {
            Ok(()) => None,
            Err(e) => Some(e.to_string()),
        }
    }
}

impl Drop for WalkKernel {
    fn drop(&mut self) {
        if self.ctx.is_null() {
            return;
        }
        // ⊘⊘ **THE ORDER HERE WOULD BE A BUG IF LEFT TO RUST.** `Drop::drop` runs BEFORE the
        // fields drop, so destroying the context first would leave every allocation's
        // `cuMemFree` running against a context that no longer exists.
        //
        // ★ And the fix is not "free them first": `cuCtxDestroy` reclaims every allocation
        // made in the context, so freeing them individually as well would be the double-free.
        // ⇒ the buffers are NEUTRALISED and the context is destroyed once.
        self.dev.ptr = 0;
        self.walk.ptr = 0;
        self.com.ptr = 0;
        self.slot.ptr = 0;
        self.iscratch.ptr = 0;
        self.pdbs.ptr = 0;
        self.slots.ptr = 0;
        self.ack.ptr = 0;
        self.ack_code.ptr = 0;
        self.store = None;
        self.hdr.ptr = 0;
        self.rpdb.ptr = 0;
        self.rrun.ptr = 0;
        // ★ P4: the stream and events are destroyed explicitly (the context would reclaim them
        // too); `ctx_destroy` then drains anything still queued — including a host function
        // that names `done_fd` — BEFORE the `CompletionFd` field closes the fd.
        let _ = self.make_current();
        if let Some(g) = self.graph.take() {
            self.cu.graph_exec_destroy(g.exec);
            self.cu.graph_destroy(g.graph);
        }
        self.cu.event_destroy(self.ev_start);
        self.cu.event_destroy(self.ev_copied);
        self.cu.event_destroy(self.ev_done);
        self.cu.stream_destroy(self.stream);
        self.cu.ctx_destroy(self.ctx);
        self.ctx = CtxHandle::null();
    }
}

/// A host image uploaded to device memory.
///
/// ⚠ **Not freed on drop**, and that is deliberate: freeing needs the [`Cuda`] that allocated
/// it, and a guard holding a borrow of the kernel could not coexist with `&mut self` on
/// `refresh`. Hand it to [`WalkKernel::release`], or let `cuCtxDestroy` reclaim it — which is
/// always correct here, because the context outlives every image by construction.
#[derive(Debug, Clone, Copy)]
pub struct DeviceImage {
    ptr: CUdeviceptr,
    len: u64,
}

impl DeviceImage {
    /// Its length.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }
    /// Whether it is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}
