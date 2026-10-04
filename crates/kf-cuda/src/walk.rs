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
    KF_MAX_PDB, KF_MAX_RESET, KF_TBL_VER2, KFWR_HF_DIFF, KFWR_HF_TRUNCATED, KFWR_MAGIC,
    KFWR_OP_UNMAP, KfFormat, KfMapRun, KfPdbEntry, KfReportHeader,
};
pub use crate::driver_unsafe::walk_gpu_unsafe::{DeviceImage, kf_format_check};
use crate::driver_unsafe::walk_gpu_unsafe::{Polled, WalkGpu, WalkStage, Window};
use crate::driver_unsafe::{CompletionFd, CudaError};

/// ★★★ **The committed PTX.** Built from `cuda/walk/kf_walk.cu` by
/// `cuda/walk/make_ptx.py` — NVRTC, no GPU and no nvcc, so it is generated where the rest of
/// this tree is generated rather than on a rented box.
///
/// ⊘ `THE_CONSTRAINTS.md` §20: *"it is not code injection: the PTX is ours, built at build
/// time"*. Embedding it makes that literally true of the shipped artifact — there is no path
/// at run time from which a different program could be read, which also means the sandbox has
/// nothing to grant for it.
pub static WALK_PTX: &[u8] = include_bytes!("../../../cuda/walk/kf_walk.ptx");

/// ★ w829: re-walks one submitted walk may take to fix capacity refusals before the refusal is
/// handed to the caller by name. Two suffice for any single growth (walk region, then slot).
const CAPACITY_RETRIES: u32 = 4;

/// How large a report this driver asks the kernel for.
#[derive(Debug, Clone, Copy)]
pub struct WalkCfg {
    /// ★ w829: the most runs ONE address space may hold (its walk region, its slot) — the named
    /// ceiling. ⊘ Was the uniform per-space slice (16 384); capacity is now carved per space from
    /// the two pools below ([`crate::capacity`]).
    pub runs_per_pdb: u32,
    /// ★ w829: the walk pool, in runs — every walk's entries share it (and 4× it of scratch).
    pub walk_pool: u32,
    /// ★ w829: the committed-placement pool, in runs — every slot's region comes from it.
    pub slot_pool: u32,
    /// ★ w829: a new slot's first region, in runs (grown on demand).
    pub slot_default: u32,
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
    /// ★★★ v3-roperm: the permission bits that join the diff key (`KfArgs::key_perm`)
    /// — a subset of [`crate::abi::KFWR_RF_KEY_PERM_ALL`], the host's policy
    /// (`kf_mem::apply::PermPolicy::key_perm`), refused by name otherwise.
    pub key_perm: u32,
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
        // ★★★★★ w829 — POOLED. `[measured uw4]` one CUDA process's space held more than 16 384
        // scattered 4 KiB sysmem runs; the uniform slice killed UVM's channel. Same device memory
        // as the uniform tables it replaces (walk 1 Mi runs = 32 MiB, slots 2 Mi runs = 64 MiB,
        // the scratch is the existing 4 Mi-run stage), but ONE space may now hold 1 Mi runs
        // (4 GiB of scattered 4 KiB pages) instead of 16 384. The report carries 256 Ki runs
        // (8 MiB pinned) — a first diff of a large space is one report, not a truncation.
        WalkCfg {
            runs_per_pdb: 1 << 20,
            walk_pool: 1 << 20,
            slot_pool: 2 << 20,
            slot_default: 1024,
            run_capacity: 1 << 18,
            pdb_capacity: 64,
            entry_budget: 1 << 22,
            table_version: KF_TBL_VER2,
            max_slots: 128,
            key_perm: crate::abi::KFWR_RF_KEY_PERM_DEFAULT,
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
    CudaError::Refused {
        what,
        code: 0,
        name,
    }
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
            //
            // ⊘⊘ w828 — **the span is the STORE's (vidmem), so it bounds VIDMEM leaves only.** A
            // sysmem leaf's `gpga` is a GUEST-PHYSICAL address, which the walker deliberately
            // leaves unbounded (w825: it cannot know the VMM's layout) and `kf_mem::ledger::
            // desired_from_leaves` bounds against the guest-RAM map (`ram_offset`, PCI hole
            // included) before anything is mapped. Bounding it here by the STORE size refused every
            // guest page above `fb-mb`: `[measured vh llm vhA_gpm]` an 8 GiB q35 guest (high RAM up
            // to 0x2_8000_0000) had UVM's CE channel killed by `run[69] leaves the guest's GPGA:
            // gpga=0x2028f0000 … span is 0x200000000` — valid guest RAM — and every later CUDA
            // process spun forever. With the default 2 GiB guest no GPA ever reached the span,
            // which is why it never fired. Peer and unknown apertures stay bounded (and are
            // refused downstream by aperture anyway).
            let sysmem = matches!(
                r.aperture(),
                crate::abi::KFWR_AP_SYS_COHERENT | crate::abi::KFWR_AP_SYS_NONCOHERENT
            );
            if r.op != KFWR_OP_UNMAP
                && !sysmem
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
            return Err(ReportError::NotDiff {
                flags: self.header.flags,
            });
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

/// The walk in flight (at most one).
#[derive(Debug, Clone, Copy)]
struct InFlight {
    gpga_len: u64,
    submitted: std::time::Instant,
}

/// ★★★★★ **CUDA, up and holding the walk kernel.** One per VM, in the VMM (v3, §20).
///
/// ★ `v3-sec-rawaddr` (2026-10-04): this type holds the walker's LOGIC — capacity planning, the
/// capacity re-walk decision, ack and reset staging, report decoding — and reaches the GPU only
/// through the perimeter's `WalkGpu` (`driver_unsafe/walk_gpu_unsafe.rs`), which validates every
/// input itself (V5–V8). It holds no device address: a planner bug here becomes a named refusal at
/// V6, never a device write.
///
/// ⚠ **Everything lazy is walked during [`WalkKernel::bring_up`]**: `cuInit` → the device →
/// `cuCtxCreate` → `cuModuleLoadData` (**the PTX JIT runs here**) → every allocation → the graph →
/// a warm host signal (`THE_CONSTRAINTS.md` §w724d).
pub struct WalkKernel {
    gpu: WalkGpu,
    cfg: WalkCfg,
    inflight: Option<InFlight>,
    /// ★ The verdict the next [`WalkKernel::submit`] hands the commit node, and the slots it
    /// empties ([`WalkKernel::ack`], [`WalkKernel::reset_slot`]).
    pending_ack: Option<(u64, Vec<u8>)>,
    pending_resets: Vec<u32>,
    /// ★ w829: the capacity layout (pools, per-slot regions, per-space needs).
    cap: crate::capacity::Capacity,
    /// ★ w829: the entries of the walk in flight (a fixable capacity refusal re-submits them).
    last_entries: Vec<WalkEntry>,
    /// Re-submissions left for the walk in flight (bounded: a need that keeps moving is refused).
    retries_left: u32,
    /// ★ w829: capacity events (growth, re-walks, ceilings) for the caller's log.
    events: Vec<String>,
    /// Device name, for the census.
    pub device_name: String,
    /// How long the whole bring-up took, in microseconds.
    pub bring_up_us: u64,
    /// How long `cuModuleLoadData` alone took — the PTX JIT.
    pub jit_us: u64,
}

impl core::fmt::Debug for WalkKernel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WalkKernel")
            .field("gpu", &self.gpu)
            .field("in_flight", &self.inflight.is_some())
            .finish_non_exhaustive()
    }
}

/// ★ Which CUDA device a [`WalkKernel`] is brought up on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkDevice<'a> {
    /// CUDA ordinal 0 — for single-GPU harnesses (the gates, the self-test) only. ⊘ Never for
    /// a kf3 device: ordinal 0 is the *fastest* GPU, not the one the store was exported from.
    FirstOrdinal,
    /// The GPU at this PCI address (`dddd:bb:ss.f`) — what `kf_host::HostRm::card` states.
    PciBusId(&'a str),
}

impl WalkKernel {
    /// ★★★ Bring CUDA all the way up and load the kernel.
    ///
    /// # Errors
    /// [`CudaError`], naming the call that refused. ⊘ In particular a [`KfFormat`] that fails
    /// [`kf_format_check`] (the `.cu`'s own descriptor check, abi_version first) is refused
    /// **before the library is even opened** — §21's *"a Rust/PTX skew must fail loudly at launch,
    /// not decode garbage field offsets and look like a page-table bug"*.
    pub fn bring_up(cfg: WalkCfg, fmt: KfFormat) -> Result<WalkKernel, CudaError> {
        Self::bring_up_on(cfg, fmt, WalkDevice::FirstOrdinal)
    }

    /// ★★ As [`WalkKernel::bring_up`], on the CUDA device `on` names. A kf3 device passes
    /// [`WalkDevice::PciBusId`] with its host GPU's PCI address (V3_MULTI_GPU_AUDIT §2 blocker 1).
    ///
    /// # Errors
    /// As [`WalkKernel::bring_up`]; a PCI address this process's CUDA cannot see is refused by
    /// name, never replaced by ordinal 0.
    pub fn bring_up_on(
        cfg: WalkCfg,
        fmt: KfFormat,
        on: WalkDevice<'_>,
    ) -> Result<WalkKernel, CudaError> {
        let t0 = std::time::Instant::now();
        // ★ VER3 (Hopper, Blackwell) is ACCEPTED (w826, owner: every family first-class).
        // ⚠ KNOWN GAP (w826): the .cu's big-PTE veto is VER2-only; VER3 spells "no valid 4 KiB
        // page under this big PTE" as `PCF = 0x3`, which the kernel does not yet test.
        let (gpu, ident) = WalkGpu::bring_up(&cfg, &fmt, on)?;
        let cap = crate::capacity::Capacity::new(
            cfg.max_slots,
            cfg.walk_pool,
            cfg.slot_pool,
            cfg.runs_per_pdb,
            cfg.slot_default,
        )
        .map_err(|name| CudaError::Refused {
            what: "WalkKernel::bring_up (WalkCfg capacity)",
            code: 0,
            name,
        })?;
        Ok(WalkKernel {
            gpu,
            cfg,
            inflight: None,
            pending_ack: None,
            pending_resets: Vec::new(),
            cap,
            last_entries: Vec::new(),
            retries_left: 0,
            events: Vec::new(),
            device_name: ident.device_name,
            bring_up_us: u64::try_from(t0.elapsed().as_micros()).unwrap_or(u64::MAX),
            jit_us: ident.jit_us,
        })
    }

    /// ★★★★★ **Queue one walk of `entries` over the imported store, and return.**
    ///
    /// Each entry walks `pdb` and is diffed against slot `slot`'s committed placements. The
    /// graph's first node commits the verdict staged by [`WalkKernel::ack`] on the PREVIOUS
    /// report (and empties the slots [`WalkKernel::reset_slot`] released); then the walk, the
    /// diff, the report into pinned memory, and a host function that signals
    /// [`WalkKernel::completion_fd`]. **Nothing here waits on the GPU.**
    ///
    /// # Errors
    /// [`CudaError::Refused`] naming the call; also refused, by name, when a walk is already in
    /// flight, when no store was imported, when more than [`KF_MAX_PDB`] entries are asked for,
    /// or when a slot is out of range or repeated (two entries may never commit into one slot).
    pub fn submit(&mut self, entries: &[WalkEntry]) -> Result<(), CudaError> {
        let Some(len) = self.gpu.store_len() else {
            return Err(refused(
                "WalkKernel::submit",
                "no store imported (import_store first)".to_string(),
            ));
        };
        self.retries_left = CAPACITY_RETRIES;
        self.submit_over(Window::Store, len, entries)
    }

    /// [`WalkKernel::submit`] over an uploaded image instead of the store (the selftest).
    ///
    /// # Errors
    /// As [`WalkKernel::submit`]; an image of another walker's context is refused by name.
    pub fn submit_image(
        &mut self,
        img: &DeviceImage,
        entries: &[WalkEntry],
    ) -> Result<(), CudaError> {
        self.retries_left = CAPACITY_RETRIES;
        self.submit_over(Window::Image(img), img.len(), entries)
    }

    fn submit_over(
        &mut self,
        win: Window<'_>,
        gpga_len: u64,
        entries: &[WalkEntry],
    ) -> Result<(), CudaError> {
        if self.inflight.is_some() {
            return Err(refused(
                "WalkKernel::submit",
                "a walk is already in flight; collect it first (one pinned read-back, one walk at a time)".to_string(),
            ));
        }
        if entries.len() > KF_MAX_PDB {
            return Err(refused(
                "WalkKernel::submit",
                format!(
                    "the kernel walks {KF_MAX_PDB} entries at once and was handed {}",
                    entries.len()
                ),
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for e in entries {
            if e.slot >= self.cfg.max_slots || !seen.insert(e.slot) {
                return Err(refused(
                    "WalkKernel::submit",
                    format!(
                        "slot {} is out of range (max {}) or repeated in one walk",
                        e.slot, self.cfg.max_slots
                    ),
                ));
            }
        }
        self.gpu.make_current()?;
        // ★ w829: carve this walk's capacity (a new slot's first region, each entry's walk
        // region from its last need). A pool ceiling is refused here, by name.
        let slot_list: Vec<u32> = entries.iter().map(|e| e.slot).collect();
        let lay = self.cap.plan(&slot_list).map_err(|name| {
            self.events.push(format!("capacity ceiling: {name}"));
            refused("WalkKernel::submit (capacity)", name)
        })?;
        let pdbs: Vec<u64> = entries.iter().map(|e| e.pdb).collect();
        let nres = self.pending_resets.len().min(KF_MAX_RESET);
        let resets: Vec<u32> = self.pending_resets[..nres].to_vec();
        // ★ The verdict on the previous report is consumed by exactly this walk.
        let ack = self.pending_ack.take();
        let stage = WalkStage {
            pdbs: &pdbs,
            slots: &slot_list,
            layout: &lay,
            ack: ack.as_ref().map(|(g, c)| (*g, c.as_slice())),
            resets: &resets,
        };
        self.gpu.launch(win, &stage)?;
        self.pending_resets.drain(..nres);
        self.inflight = Some(InFlight {
            gpga_len,
            submitted: std::time::Instant::now(),
        });
        self.last_entries = entries.to_vec();
        Ok(())
    }

    /// ★★★★★ **The host's verdict on the report just collected** — one
    /// [`crate::abi::KFWR_ACK_FAILED`] / `APPLIED` / `HELD` byte per run, in report order. Staged;
    /// the next [`WalkKernel::submit`]'s first node commits it (COMMIT-ON-ACK). A walk submitted
    /// without one commits nothing, which is always safe: the next diff is then computed against
    /// the same placements.
    ///
    /// # Errors
    /// Refused while a walk is in flight (the verdict must name the report that walk will read).
    pub fn ack(&mut self, generation: u64, codes: Vec<u8>) -> Result<(), CudaError> {
        if self.inflight.is_some() {
            return Err(refused(
                "WalkKernel::ack",
                "a walk is in flight; the verdict answers the COLLECTED report".to_string(),
            ));
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
            return Err(refused(
                "WalkKernel::reset_slot",
                format!(
                    "slot {slot} (max {}) or too many pending",
                    self.cfg.max_slots
                ),
            ));
        }
        if !self.pending_resets.contains(&slot) {
            self.pending_resets.push(slot);
        }
        // ★ w829: teardown releases exactly what the object owned — its region returns to the
        // pool now. ⊘ Safe before the reset lands: the next walk's commit skips a reset slot, and
        // a region handed to another slot is read only through THAT slot's counts (zero when new).
        self.cap.release(slot);
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
        self.gpu.gpu_done_now()
    }

    /// Whether a walk is queued and not yet collected.
    #[must_use]
    pub fn in_flight(&self) -> bool {
        self.inflight.is_some()
    }

    /// ★ The fd a walk's completion is signalled on — put it in the worker's `epoll` set.
    #[must_use]
    pub fn completion_fd(&self) -> &CompletionFd {
        self.gpu.completion_fd()
    }

    /// `cuCtxSynchronize` calls made by this walker's context since bring-up. A walk adds
    /// **zero**.
    #[must_use]
    pub fn ctx_sync_calls(&self) -> u64 {
        self.gpu.ctx_sync_calls()
    }

    /// ★ P4b: whether [`WalkKernel::submit`] queues the walk as ONE `cuGraphLaunch` (`true`) or
    /// launch by launch (`false`: this driver has no graph API).
    #[must_use]
    pub fn submits_as_graph(&self) -> bool {
        self.gpu.submits_as_graph()
    }

    /// ★ P4b: how many walks had to rewrite the graph's by-value parameters.
    #[must_use]
    pub fn graph_param_updates(&self) -> u64 {
        self.gpu.graph_param_updates()
    }

    /// ★★★ **Collect the walk in flight if it has finished — never blocks.**
    ///
    /// `Ok(None)`: nothing in flight, or not done yet. `Ok(Some(_))`: done; the report was
    /// decoded from pinned memory. `Err`: the stream failed (a device fault in the walk surfaces
    /// here, named by `cuEventQuery`) and the walker is POISONED (failure policy F2): every later
    /// submit, verdict-commit and read is refused by name; recovery is a new `WalkKernel`.
    ///
    /// # Errors
    /// [`CudaError`].
    pub fn try_collect(&mut self) -> Result<Option<Collected>, CudaError> {
        let Some(f) = self.inflight else {
            return Ok(None);
        };
        let gpu_us = match self.gpu.poll() {
            Polled::Pending => return Ok(None),
            Polled::Done { gpu_us } => gpu_us,
            Polled::Idle => {
                self.inflight = None;
                return Err(refused(
                    "WalkKernel::try_collect",
                    "the walker lost track of its walk (no flight)".to_string(),
                ));
            }
            Polled::Poisoned(why) => {
                self.inflight = None;
                return Err(refused(
                    "WalkKernel::try_collect (poisoned)",
                    format!(
                        "a completion query failed; the walker refuses all further work: {why}"
                    ),
                ));
            }
        };
        self.inflight = None;
        let submit_to_collect_us =
            u64::try_from(f.submitted.elapsed().as_micros()).unwrap_or(u64::MAX);
        // ★ One decoder per report struct (`abi.rs`), the counts clamped inside the perimeter.
        let (header, pdbs, runs) = self.gpu.read_report()?;
        let report = Report {
            header,
            pdbs,
            runs,
            gpga_span: f.gpga_len,
        };
        // ★ w829: a capacity refusal the pools can fix is fixed HERE and the same walk re-queued
        // — the caller never sees that report (it was never acked, so nothing of it is committed
        // and nothing committed is lost: the next diff is taken against the same placements).
        if self.capacity_retry(&report, &f)? {
            return Ok(None);
        }
        Ok(Some(Collected {
            report,
            gpu_us,
            submit_to_collect_us,
        }))
    }

    /// ★★★★★ w829 — **is this report's capacity refusal fixable, and if so, fix it and re-walk.**
    ///
    /// - an entry whose WALK was cut at its region (`KFWR_R_RUN_CAP` in its own bits): its need
    ///   (uncapped, `KfPdbEntry::need`) sizes the next walk's region;
    /// - an entry whose DIFF could not fit its slot (`PARTIAL`/`OVERFLOW`, need > the slot):
    ///   the slot GROWS — a new region, the committed placements copied device-to-device on the
    ///   walk stream (ordered before the re-walk's commit and diff), the old region freed.
    ///
    /// ⊘ Not fixable, and returned to the caller to refuse by name: a walk abort (budget,
    /// frontier), a report-level truncation, a need past the ceiling, or a need that keeps moving
    /// ([`CAPACITY_RETRIES`] re-walks).
    fn capacity_retry(&mut self, r: &Report, f: &InFlight) -> Result<bool, CudaError> {
        use crate::abi::{
            KFWR_R_BUDGET, KFWR_R_FRONTIER_CAP, KFWR_R_PDB_CAP, KFWR_R_RUN_CAP, KFWR_V_OVERFLOW,
            KFWR_V_PARTIAL,
        };
        let h = &r.header;
        let aborted = h.refuse_mask & (KFWR_R_BUDGET | KFWR_R_FRONTIER_CAP | KFWR_R_PDB_CAP) != 0;
        let mut fix = false;
        let mut walk_cut = false;
        let mut moves = Vec::new();
        for p in &r.pdbs {
            let s = p.slot();
            let need = p.need();
            if p.refused_bits() & KFWR_R_RUN_CAP != 0 {
                walk_cut = true;
                match self.cap.walk_needed(s, need) {
                    Ok(_) => fix = true,
                    Err(e) => {
                        self.events.push(format!("capacity ceiling: {e}"));
                        return Ok(false);
                    }
                }
            } else if p.vas_flags & (KFWR_V_OVERFLOW | KFWR_V_PARTIAL) != 0 && need > 0 {
                match self.cap.slot_needed(s, need) {
                    Ok(Some(m)) => {
                        moves.push(m);
                        fix = true;
                    }
                    // Fits the slot already: the diff's guard fired (slot > walk region) — the
                    // next plan sizes the walk region to the slot.
                    Ok(None) => {
                        let _ = self.cap.walk_needed(s, need);
                        fix = true;
                    }
                    Err(e) => {
                        self.events.push(format!("capacity ceiling: {e}"));
                        return Ok(false);
                    }
                }
            }
        }
        let truncated = h.flags & KFWR_HF_TRUNCATED != 0;
        // A truncation we did not cause at an entry's walk region (the report's own capacity) is
        // not ours to fix by re-walking.
        if !fix || aborted || (truncated && !walk_cut) {
            return Ok(false);
        }
        if self.retries_left == 0 {
            self.events.push(format!(
                "capacity: the need kept moving after {CAPACITY_RETRIES} re-walks — refusing this walk by name"
            ));
            return Ok(false);
        }
        self.retries_left -= 1;
        self.cap.stats.retries += 1;
        for m in &moves {
            // Copy the whole old region: the counts (in `KfSlot`) say how much of it is live.
            self.gpu.move_slot(m.from, m.to)?;
            self.events.push(format!(
                "capacity: slot {} grown {} → {} runs",
                m.slot, m.from.cap, m.to.cap
            ));
        }
        let needs: Vec<String> = r
            .pdbs
            .iter()
            .filter(|p| p.need() > 0)
            .map(|p| format!("s{}:{}", p.slot(), p.need()))
            .collect();
        self.events.push(format!(
            "capacity: re-walk (needs {}); {}",
            needs.join(" "),
            self.cap.census()
        ));
        let entries = core::mem::take(&mut self.last_entries);
        self.submit_over(Window::Same, f.gpga_len, &entries)?;
        Ok(true)
    }

    /// ★ w829: capacity events since the last call (growth, re-walks, ceilings), for the log.
    pub fn take_capacity_events(&mut self) -> Vec<String> {
        core::mem::take(&mut self.events)
    }

    /// ★ w829: the capacity counters.
    #[must_use]
    pub fn capacity_stats(&self) -> crate::capacity::CapacityStats {
        self.cap.stats
    }

    /// ★ w829: the capacity census — pools, use, growth. For the device's budget line.
    #[must_use]
    pub fn capacity_census(&self) -> String {
        self.cap.census()
    }

    /// ★ w829: the device memory the walker's pools hold, in bytes (walk table + committed
    /// placements + integer scratch + the run stage the scratch reuses).
    #[must_use]
    pub fn pool_bytes(&self) -> u64 {
        self.gpu.pool_bytes()
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
            let _ = self.gpu.completion_fd().wait_readable(slice);
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
    pub fn refresh_image(
        &mut self,
        img: &DeviceImage,
        entries: &[WalkEntry],
    ) -> Result<Report, CudaError> {
        self.submit_image(img, entries)?;
        Ok(self.wait(10_000)?.report)
    }

    /// ★★★★★ **MAKE THIS KERNEL'S CONTEXT CURRENT ON THE CALLING THREAD.**
    ///
    /// ⊘ **Required before any call from a thread that did not bring CUDA up.** The context is
    /// per-thread current (`[measured w731, RTX 3060, 580.159.04]`). Idempotent and cheap — call
    /// it at the top of every entry point that can be reached from a worker.
    ///
    /// # Errors
    /// [`CudaError`].
    pub fn make_current(&self) -> Result<(), CudaError> {
        self.gpu.make_current()
    }

    /// Copy a host image (non-empty) into fresh device memory of this walker's context.
    ///
    /// # Errors
    /// [`CudaError`]; an empty image is refused.
    pub fn upload(&self, bytes: &[u8]) -> Result<DeviceImage, CudaError> {
        self.gpu.upload(bytes)
    }

    /// ★★★ **Import an RM-exported store into THIS kernel's context** and map it whole.
    ///
    /// `store` is the one token `kf_host::HostRm::export_store` mints: the `/dev/nvidiactl`
    /// descriptor RM exported the object to (`w755i`) and the length the session allocated for it
    /// (§2.6 Res-1). Neither is a caller's value — no caller can pass a length of its own.
    ///
    /// ★ §13: the device address stays INSIDE the perimeter. Every later access to the store is
    /// [`WalkKernel::write_store`] / [`WalkKernel::read_store`], bounded by its length, and every
    /// walk [`WalkKernel::submit`]s over it.
    ///
    /// # Errors
    /// [`CudaError::Refused`] naming the import step that failed, or a second import.
    pub fn import_store(&mut self, store: &kf_host::RmExport) -> Result<(), CudaError> {
        self.gpu.import_store(store)
    }

    /// The imported store's length, if one was imported.
    #[must_use]
    pub fn store_len(&self) -> Option<u64> {
        self.gpu.store_len()
    }

    /// ★ §13: write `bytes` at store offset `off` — bounds-checked against the store inside the
    /// perimeter. ⊘ Synchronous, on the legacy stream, which the walker's blocking stream is
    /// ordered behind: a walk submitted after this returns reads these bytes.
    ///
    /// # Errors
    /// Refused by name outside the store; the CUDA error otherwise.
    pub fn write_store(&self, off: u64, bytes: &[u8]) -> Result<(), CudaError> {
        self.gpu.store_write(off, bytes)
    }

    /// ★ §13: read `buf.len()` bytes at store offset `off` (a harness check, never a data path).
    ///
    /// # Errors
    /// Refused by name outside the store; the CUDA error otherwise.
    pub fn read_store(&self, off: u64, buf: &mut [u8]) -> Result<(), CudaError> {
        self.gpu.store_read(off, buf)
    }

    /// Diagnostics: the LAST walk's runs for walk entry `entry` (its previous walk region). ⊘ A
    /// read-back of OUR table, never of a guest one; only a refusal's census calls it.
    ///
    /// # Errors
    /// Refused past [`KF_MAX_PDB`], with no region, or while a walk is in flight; the CUDA error
    /// otherwise.
    pub fn debug_walk_runs(&self, entry: u32) -> Result<Vec<KfMapRun>, CudaError> {
        if entry as usize >= KF_MAX_PDB {
            return Err(refused(
                "WalkKernel::debug_walk_runs",
                format!("entry {entry}"),
            ));
        }
        let Some(reg) = self.cap.prev_walk(entry as usize) else {
            return Err(refused(
                "WalkKernel::debug_walk_runs",
                format!("entry {entry}: no walk region"),
            ));
        };
        self.gpu.read_walk_region(reg.off, reg.cap)
    }

    /// Read `buf.len()` bytes of an uploaded image at `off`, bounds-checked against it.
    ///
    /// # Errors
    /// Refused by name outside the image or for another walker's image; the CUDA error otherwise.
    pub fn read_image(&self, img: &DeviceImage, off: u64, buf: &mut [u8]) -> Result<(), CudaError> {
        self.gpu.image_read(img, off, buf)
    }

    /// Write `bytes` into an uploaded image at `off`, bounds-checked against it.
    ///
    /// # Errors
    /// As [`WalkKernel::read_image`].
    pub fn write_image(&self, img: &DeviceImage, off: u64, bytes: &[u8]) -> Result<(), CudaError> {
        self.gpu.image_write(img, off, bytes)
    }

    /// ★★ **A launch that must FAIL** — probe (b) of `THE_CONSTRAINTS.md` §w724d.
    ///
    /// Asks for a **2048-thread block**, above the architectural maximum on every part that
    /// exists, so the **driver** refuses at launch rather than the device faulting. The point
    /// is that the driver must be able to *report* a refusal after the sandbox is entered,
    /// without reopening anything by path.
    ///
    /// Returns `Some(why)` when the driver refused it as intended, and `None` when it did not —
    /// it launched (a finding), or a walk was in flight so it could not be attempted. `None` must
    /// never read as a passing probe.
    pub fn probe_failed_launch(&mut self) -> Option<String> {
        if self.inflight.is_some() {
            return None;
        }
        self.gpu.probe_oversized_block()
    }
}
