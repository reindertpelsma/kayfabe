//! ★★★★★ **THE WALK KERNEL, DRIVEN FROM RUST** — load the committed PTX, allocate, launch,
//! read the report back, validate it.
//!
//! This is the half of `cuda/walk/kf_walk.cu` that the `.cu` writes against the CUDA
//! **runtime** API (`kf_create`/`kf_refresh`) and that kayfabe cannot use: the runtime API
//! lives in `libcudart`, which a driver-only box does not have, and it owns a context
//! lifecycle this process wants to own itself. ⊘ The device half is untouched — it is the
//! committed PTX, built from that same file.

use crate::abi::{
    KF_ABI_VERSION, KF_MAX_PDB, KF_TBL_VER2, KFWR_HF_TRUNCATED, KFWR_MAGIC, KFWR_OP_UNMAP, KfArgs,
    KfDev, KfFormat, KfMapRun, KfPdbEntry, KfReportHeader, KfScope,
};
use crate::driver_unsafe::{CUdeviceptr, CtxHandle, Cuda, CudaError, Func};

/// Byte offset of `KfDev::acked`. ⊘ Derived with `offset_of!` rather than written as `8`, so a
/// field inserted before it is a compile-time relocation and not a silent write to the wrong
/// word — `generation` sits immediately before it and the two are the same type.
const ACKED_BYTE_OFFSET: u64 = core::mem::offset_of!(KfDev, acked) as u64;

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
const SYM_BEGIN: &str = "_Z15kf_begin_kernelP5KfDev";
const SYM_WALK: &str = "_Z14kf_walk_kernel6KfArgs";
const SYM_DIFF: &str = "_Z14kf_diff_kernel6KfArgsPKj";
/// w826 — the parallel prefix/suffix trim that bounds the serial diff to what changed.
const SYM_TRIM: &str = "_Z19kf_diff_trim_kernel6KfArgsPj";

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
    ntask: DevBuf,
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
        WalkCfg {
            runs_per_pdb: 16384,
            run_capacity: 16384,
            pdb_capacity: 64,
            entry_budget: 1 << 22,
            table_version: KF_TBL_VER2,
        }
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

    /// Whether the walk was truncated — in which case it is **not** a delta and the walker
    /// must not be acked.
    #[must_use]
    pub fn truncated(&self) -> bool {
        (self.header.flags & KFWR_HF_TRUNCATED) != 0
    }
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
    f_diff: Func,
    f_trim: Func,
    f_par: [Func; 9],
    par: ParBufs,
    fmt: KfFormat,
    cfg: WalkCfg,
    dev: DevBuf,
    tbl: [DevBuf; 2],
    pdbs: DevBuf,
    scopes: DevBuf,
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
        let f_diff = cu.module_function(module, SYM_DIFF)?;
        let f_trim = cu.module_function(module, SYM_TRIM)?;
        let mut f_par = [f_begin; 9];
        for (i, sym) in SYM_PAR.iter().enumerate() {
            f_par[i] = cu.module_function(module, sym)?;
        }

        let tbl_runs = cfg.runs_per_pdb as usize * KF_MAX_PDB;
        let a = |bytes: usize, what: &'static str| -> Result<DevBuf, CudaError> {
            Ok(DevBuf {
                ptr: cu.mem_alloc_zeroed(bytes, what)?,
            })
        };
        let dev = a(core::mem::size_of::<KfDev>(), "cuMemAlloc(KfDev)")?;
        let tbl = [
            a(
                tbl_runs * core::mem::size_of::<KfMapRun>(),
                "cuMemAlloc(tbl0)",
            )?,
            a(
                tbl_runs * core::mem::size_of::<KfMapRun>(),
                "cuMemAlloc(tbl1)",
            )?,
        ];
        let pdbs = a(KF_MAX_PDB * 8, "cuMemAlloc(pdbs)")?;
        let scopes = a(
            crate::abi::KF_MAX_SCOPE * core::mem::size_of::<KfScope>(),
            "cuMemAlloc(scopes)",
        )?;
        let hdr = a(core::mem::size_of::<KfReportHeader>(), "cuMemAlloc(hdr)")?;
        let rpdb = a(
            cfg.pdb_capacity as usize * core::mem::size_of::<KfPdbEntry>(),
            "cuMemAlloc(rpdb)",
        )?;
        let rrun = a(
            cfg.run_capacity as usize * core::mem::size_of::<KfMapRun>(),
            "cuMemAlloc(rrun)",
        )?;

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
            ntask: a(4, "cuMemAlloc(par.ntask)")?,
            pdbbase: a(KF_MAX_PDB * 4, "cuMemAlloc(par.pdbbase)")?,
            used: a(4 * 4, "cuMemAlloc(par.used)")?,
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
            ..h
        };
        cu.memcpy_h2d(dev.ptr, bytes_of(&h), "cuMemcpyHtoD(KfDev)")?;

        Ok(WalkKernel {
            cu,
            ctx,
            f_begin,
            f_walk,
            f_diff,
            f_trim,
            f_par,
            par,
            fmt,
            cfg,
            dev,
            tbl,
            pdbs,
            scopes,
            hdr,
            rpdb,
            rrun,
            device_name,
            bring_up_us: u64::try_from(t0.elapsed().as_micros()).unwrap_or(u64::MAX),
            jit_us,
        })
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
        a.tbl = [self.tbl[0].ptr, self.tbl[1].ptr];
        a.pdbs = self.pdbs.ptr;
        a.npdb = npdb;
        a.scopes = self.scopes.ptr;
        a.nscope = 0;
        a.hdr = self.hdr.ptr;
        a.rpdb = self.rpdb.ptr;
        a.rrun = self.rrun.ptr;
        a
    }

    /// One refresh over `gpga` (a device pointer and a length) for the ascending `pdbs`.
    ///
    /// # Errors
    /// [`CudaError`], naming the call that refused.
    ///
    /// # Panics
    /// If more than [`KF_MAX_PDB`] address spaces are asked for — a cap the caller can see and
    /// must not exceed silently.
    pub fn refresh(
        &mut self,
        gpga: CUdeviceptr,
        gpga_len: u64,
        pdbs: &[u64],
    ) -> Result<Report, CudaError> {
        assert!(
            pdbs.len() <= KF_MAX_PDB,
            "the kernel's table holds {KF_MAX_PDB} address spaces and was handed {}",
            pdbs.len()
        );
        let mut pdb_bytes = Vec::with_capacity(pdbs.len() * 8);
        for p in pdbs {
            pdb_bytes.extend_from_slice(&p.to_le_bytes());
        }
        self.cu
            .memcpy_h2d(self.pdbs.ptr, &pdb_bytes, "cuMemcpyHtoD(pdbs)")?;

        let args = self.args_for(gpga, gpga_len, u32::try_from(pdbs.len()).unwrap_or(0));
        let mut dev_param = param_bytes(&self.dev.ptr);
        let mut args_param = param_bytes(&args);

        self.cu.launch(
            self.f_begin,
            1,
            1,
            &mut dev_param,
            "cuLaunchKernel(kf_begin_kernel)",
        )?;
        // ★★★★★ w826 — THE PARALLEL WALK (`kf_run_parallel`, ported to the driver API).
        // `[w726]` ~0.6 ms fixed cost vs the serial walk's one-thread-per-space (2.4 ms on the
        // live guest, 450 ms on the measured working set). The serial kernel stays in the PTX
        // for the scoped path the .cu keeps; this refresh is never scoped.
        let _ = self.f_walk;
        let t0 = std::time::Instant::now();
        self.run_parallel(&args, u32::try_from(pdbs.len()).unwrap_or(0))?;
        // w826 — phase census: the walk vs the (single-thread) diff, every 512 refreshes.
        self.cu.ctx_synchronize()?;
        let t_walk = t0.elapsed();
        let _ = &mut args_param;
        let trim = self.par.cnt.ptr;
        self.cu.launch_args(
            self.f_trim,
            u32::try_from(pdbs.len()).unwrap_or(0).max(1),
            256,
            0,
            &mut [param_bytes(&args), trim.to_le_bytes().to_vec()],
            "cuLaunchKernel(kf_diff_trim_kernel)",
        )?;
        self.cu.launch_args(
            self.f_diff,
            1,
            1,
            0,
            &mut [param_bytes(&args), trim.to_le_bytes().to_vec()],
            "cuLaunchKernel(kf_diff_kernel)",
        )?;
        self.cu.ctx_synchronize()?;
        phase_census(t_walk, t0.elapsed() - t_walk);

        let mut hb = vec![0u8; core::mem::size_of::<KfReportHeader>()];
        self.cu
            .memcpy_d2h(&mut hb, self.hdr.ptr, "cuMemcpyDtoH(hdr)")?;
        let header = crate::driver_unsafe::read_struct::<KfReportHeader>(&hb);

        // ⊘ Clamped to the CAPACITY before the copy: a truncated report legitimately declares
        // more than it carries (invariant I3), and reading `run_count` elements out of a
        // `run_capacity` buffer would turn "loud truncation" into a host-side overrun.
        let npdb = header.pdb_count.min(self.cfg.pdb_capacity) as usize;
        let nrun = header.run_count.min(self.cfg.run_capacity) as usize;
        let mut pdbs_out = Vec::with_capacity(npdb);
        let mut runs_out = Vec::with_capacity(nrun);
        if npdb > 0 {
            let mut b = vec![0u8; npdb * core::mem::size_of::<KfPdbEntry>()];
            self.cu
                .memcpy_d2h(&mut b, self.rpdb.ptr, "cuMemcpyDtoH(rpdb)")?;
            for c in b.chunks_exact(core::mem::size_of::<KfPdbEntry>()) {
                pdbs_out.push(crate::driver_unsafe::read_struct::<KfPdbEntry>(c));
            }
        }
        if nrun > 0 {
            let mut b = vec![0u8; nrun * core::mem::size_of::<KfMapRun>()];
            self.cu
                .memcpy_d2h(&mut b, self.rrun.ptr, "cuMemcpyDtoH(rrun)")?;
            for c in b.chunks_exact(core::mem::size_of::<KfMapRun>()) {
                runs_out.push(crate::driver_unsafe::read_struct::<KfMapRun>(c));
            }
        }
        Ok(Report {
            header,
            pdbs: pdbs_out,
            runs: runs_out,
            gpga_span: gpga_len,
        })
    }

    /// ★★★★★ **ACK A GENERATION — the host half of the snapshot handshake.**
    ///
    /// ⊘⊘⊘ **This had NO CALLER.** `KfDev` has carried `generation`/`acked` and the `.cu` has
    /// carried `kf_ack` since the walker was written, and nothing in the Rust tree ever acked:
    /// `acked` sat at 0 for the life of every VM. The handshake was built and orphaned.
    ///
    /// # The race it closes
    ///
    /// Owner, 2026-09-18: *"worker 1 is applying mmio refresh but worker 2 started refresh in
    /// scratchpad."* Without an ack the kernel has no way to know a report was consumed, so a
    /// second refresh may install a new snapshot while the first one's delta is still being
    /// applied. The second delta is then computed against a snapshot that assumes the first
    /// one's mappings are live when they are not — and the difference between those two
    /// worlds is silent: both are well-formed reports.
    ///
    /// ⚠ **ACK AFTER APPLYING, NEVER ON RECEIPT.** Acking when the bytes arrive leaves exactly
    /// the window open that this exists to close. The caller's contract is: apply the delta,
    /// then ack the generation it came from.
    ///
    /// ⊘ A **truncated** report is not a delta and must NOT be acked — see
    /// [`Report::truncated`], whose doc has said so since before anything could ack.
    ///
    /// # Why a memcpy and not `kf_ack_kernel`
    ///
    /// The `.cu` provides `kf_ack_kernel(KfDev*, u64)`, and it is in the shipped PTX. But it
    /// takes **two** by-value parameters while [`Cuda::launch_raw`] passes exactly one, so
    /// using it would mean widening the unsafe launch surface for a single `u64` store. A
    /// device-to-device write of one field through the existing bounded `memcpy_h2d` adds no
    /// `unsafe` at all. ⊘ `self.dev.ptr` is a **device** address, not a host-process VA, so it
    /// is legal to compute on in safe Rust (the constraint is about VMM virtual addresses).
    ///
    /// # Errors
    /// The CUDA error, if the copy fails.
    fn run_parallel(&self, a: &KfArgs, npdb: u32) -> Result<(), CudaError> {
        let p = |v: u64| v.to_le_bytes().to_vec();
        let u = |v: u32| v.to_le_bytes().to_vec();
        let ab = || param_bytes(a);
        let par = &self.par;
        let shm = (KF_PAR_BLOCK / KF_WARP) * KF_SHWORDS * 8;
        let [f_seed, f_expand, f_scan, f_compact, f_leaf, f_heads, f_bases, f_emit, f_join] =
            self.f_par;
        self.cu.launch_args(
            f_seed,
            npdb.div_ceil(128).max(1),
            128,
            0,
            &mut [ab(), p(par.fr[0].ptr), p(par.nfr.ptr), p(par.used.ptr)],
            "cuLaunchKernel(kf_par_seed)",
        )?;
        let mut src = 0usize;
        for k in u32::from(self.fmt.first_dir)..KF_DIRS {
            let nin = par.nfr.ptr + (src as u64) * 4;
            let nout = par.nfr.ptr + ((src ^ 1) as u64) * 4;
            let dst = if k + 1 < KF_DIRS { par.fr[src ^ 1].ptr } else { par.task.ptr };
            self.cu.launch_args(
                f_expand,
                KF_PAR_GRID,
                KF_PAR_BLOCK,
                shm,
                &mut [
                    ab(),
                    u(k),
                    p(par.fr[src].ptr),
                    p(nin),
                    p(par.stage.ptr),
                    u(KF_MAX_FRONTIER as u32),
                    p(par.used.ptr),
                    p(par.start.ptr),
                    p(par.cnt.ptr),
                ],
                "cuLaunchKernel(kf_par_expand)",
            )?;
            self.cu.launch_args(
                f_scan,
                1,
                KF_SCAN_BLOCK,
                0,
                &mut [p(par.cnt.ptr), p(nin), p(par.off.ptr), p(nout)],
                "cuLaunchKernel(kf_par_scan)",
            )?;
            self.cu.launch_args(
                f_compact,
                KF_PAR_GRID,
                KF_PAR_BLOCK,
                0,
                &mut [
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
                self.cu.memcpy_d2d(par.ntask.ptr, nout, 4, "cuMemcpyDtoD(ntask)")?;
            }
            self.cu.memset_d8(par.used.ptr, 0, 4, "cuMemsetD8(used)")?;
        }
        self.cu.launch_args(
            f_leaf,
            KF_PAR_GRID,
            KF_PAR_BLOCK,
            shm,
            &mut [
                ab(),
                p(par.task.ptr),
                p(par.ntask.ptr),
                p(par.runstage.ptr),
                u(KF_MAX_SCRATCH as u32),
                p(par.used.ptr),
                p(par.sum.ptr),
            ],
            "cuLaunchKernel(kf_par_leaf)",
        )?;
        self.cu.launch_args(
            f_heads,
            KF_PAR_GRID,
            128,
            0,
            &mut [
                ab(),
                p(par.task.ptr),
                p(par.sum.ptr),
                p(par.ntask.ptr),
                p(par.cnt.ptr),
                p(par.head.ptr),
            ],
            "cuLaunchKernel(kf_par_heads)",
        )?;
        self.cu.launch_args(
            f_scan,
            1,
            KF_SCAN_BLOCK,
            0,
            &mut [p(par.cnt.ptr), p(par.ntask.ptr), p(par.off.ptr), p(par.nfr.ptr + 12)],
            "cuLaunchKernel(kf_par_scan tasks)",
        )?;
        self.cu.launch_args(
            f_bases,
            1,
            1,
            0,
            &mut [ab(), p(par.pdbbase.ptr)],
            "cuLaunchKernel(kf_par_bases)",
        )?;
        self.cu.launch_args(
            f_emit,
            KF_PAR_GRID,
            KF_PAR_BLOCK,
            0,
            &mut [
                ab(),
                p(par.task.ptr),
                p(par.sum.ptr),
                p(par.ntask.ptr),
                p(par.off.ptr),
                p(par.head.ptr),
                p(par.pdbbase.ptr),
                p(par.runstage.ptr),
            ],
            "cuLaunchKernel(kf_par_emit)",
        )?;
        self.cu.launch_args(
            f_join,
            KF_PAR_GRID,
            128,
            0,
            &mut [
                ab(),
                p(par.task.ptr),
                p(par.sum.ptr),
                p(par.ntask.ptr),
                p(par.off.ptr),
                p(par.head.ptr),
                p(par.pdbbase.ptr),
            ],
            "cuLaunchKernel(kf_par_join)",
        )
    }

    pub fn ack(&mut self, generation: u64) -> Result<(), CudaError> {
        let at = self.dev.ptr + ACKED_BYTE_OFFSET;
        self.cu
            .memcpy_h2d(at, &generation.to_le_bytes(), "cuMemcpyHtoD(KfDev::acked)")
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
    /// # Errors
    /// [`CudaError::Refused`] naming the import step that failed.
    pub fn import_store(&self, fd: i32, bytes: u64) -> Result<CUdeviceptr, CudaError> {
        self.make_current()?;
        let n = usize::try_from(bytes).map_err(|_| CudaError::Refused {
            what: "import_store: object length does not fit usize",
            code: 0,
            name: format!("{bytes:#x}"),
        })?;
        // ⊘ `import_and_map` reports which of its four steps refused as a string; carried in
        // `name` verbatim so the step is not lost to a generic code.
        self.cu.import_and_map(0, fd, n).map_err(|name| CudaError::Refused {
            what: "cuMemImportFromShareableHandle + cuMemMap",
            code: 0,
            name,
        })
    }

    /// Copy host bytes to an **arbitrary** device address — used to place tables at the
    /// offsets the guest actually uses, inside an imported object, without relocating them.
    ///
    /// # Errors
    /// [`CudaError`].
    /// Read `buf.len()` bytes of device memory at `src` (a harness check, never a data path).
    ///
    /// # Errors
    /// The CUDA error.
    pub fn read_at(&self, src: CUdeviceptr, buf: &mut [u8]) -> Result<(), CudaError> {
        self.cu.memcpy_d2h(buf, src, "cuMemcpyDtoH(read_at)")
    }

    pub fn write_at(&self, dst: CUdeviceptr, bytes: &[u8]) -> Result<(), CudaError> {
        self.cu.memcpy_h2d(dst, bytes, "cuMemcpyHtoD(write_at)")
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
        self.tbl[0].ptr = 0;
        self.tbl[1].ptr = 0;
        self.pdbs.ptr = 0;
        self.scopes.ptr = 0;
        self.hdr.ptr = 0;
        self.rpdb.ptr = 0;
        self.rrun.ptr = 0;
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
    /// Its device pointer.
    #[must_use]
    pub fn ptr(&self) -> CUdeviceptr {
        self.ptr
    }
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

/// ★ w826 — where a refresh's time goes: the parallel walk vs the diff kernel (`<<<1,1>>>`).
fn phase_census(walk: std::time::Duration, diff: std::time::Duration) {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    static N: AtomicU64 = AtomicU64::new(0);
    static W: AtomicU64 = AtomicU64::new(0);
    static D: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Relaxed) + 1;
    let w = W.fetch_add(walk.as_micros() as u64, Relaxed) + walk.as_micros() as u64;
    let d = D.fetch_add(diff.as_micros() as u64, Relaxed) + diff.as_micros() as u64;
    if n % 512 == 0 {
        eprintln!(
            "kayfabe-isolate: WALK-PHASES n={n} avg_us[walk={} diff={}] last_us[walk={} diff={}]",
            w / n,
            d / n,
            walk.as_micros(),
            diff.as_micros()
        );
    }
}
