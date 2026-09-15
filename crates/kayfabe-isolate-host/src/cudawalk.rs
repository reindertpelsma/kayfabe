//! ★★★★★ **THE SCRATCHPAD ISOLATE'S CUDA HALF** — `THE_CONSTRAINTS.md` §w724d,
//! `SINGLE_STORE_PLAN.md` increment 4.
//!
//! Two entry points and one piece of state, and the ordering between them is the whole design:
//!
//! 1. [`bring_up_before_sandbox`] — called from `build_backends` **before**
//!    `sandbox::enter`, while the dynamic loader and `/dev/nvidia*` are still reachable BY
//!    PATH. It walks every lazy path CUDA has and leaves a live context behind.
//! 2. [`probe_after_sandbox`] — called **after** the namespace, the `pivot_root` and the
//!    privilege drop, on that same context. It is the measurement §w724d asks for, and it is
//!    the only thing that can distinguish *"CUDA survives the drop"* from *"we did not try"*.
//!
//! # ⊘ Why the state is a process global
//!
//! The kernel's context belongs to the **process**, not to a worker, and the two entry points
//! are called from `build_backends` — which has no place to put something that outlives it and
//! is reached again later. ⚠ It is a `Mutex<Option<..>>` and not a `OnceLock`, because the
//! second call mutates what the first produced.
//!
//! # ⊘ The report crosses the wire; it is not shouted
//!
//! The child writes its outcome here and the parent asks for it with
//! [`crate::proto::Request::CudaWalkReport`], so the census line lands beside the other
//! `SCRATCHPAD` lines in the boot's own log rather than interleaved into QEMU's stderr at
//! whatever moment the child happened to reach it.

use kayfabe_cuda::selftest::SelftestOutcome;
use kayfabe_cuda::WalkKernel;
use std::sync::Mutex;

/// The live context and what it has told us so far.
static STATE: Mutex<Option<(SelftestOutcome, Option<WalkKernel>)>> = Mutex::new(None);

/// ★★★ §w724d step 2. Bring CUDA all the way up and prove the kernel answers.
///
/// ⚠ Must be called **before** `sandbox::enter`, on the single-threaded startup path. Calling
/// it twice is a no-op with a complaint: the second call cannot mean anything the first did
/// not, and silently rebuilding a context would leave the first one's allocations orphaned.
pub fn bring_up_before_sandbox() {
    let mut g = STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if g.is_some() {
        eprintln!("kayfabe-isolate: ⊘ CUDA bring-up asked for twice; the first one stands");
        return;
    }
    // ★★★★★ **`/proc` FIRST, AND IT IS NOT HOUSEKEEPING** —
    // `[measured 2026-09-14, RTX 3060, 580.159.04]` `cuInit` refuses with
    // `CUDA_ERROR_OPERATING_SYSTEM` (304) inside this isolate's PID namespace, because the
    // isolate is `clone`d with `CLONE_NEWPID` and `/proc` is still its PARENT'S. Bisected one
    // namespace at a time: user/mount/net/ipc/uts each pass alone; **pid** fails; **pid with a
    // remounted `/proc` passes**. See `kayfabe_linux_raw::sandbox::remount_proc`.
    //
    // ⊘ §w724d did not anticipate this, and the ordering it prescribes does not address it:
    // the isolate is **born namespaced** (the namespaces come from the `clone` that CREATES
    // it, which is the only way `CLONE_NEWPID` can be had at all), so there is no moment in
    // its life when CUDA could have initialised without this.
    //
    // ⚠ A failure here is RECORDED, not fatal: `cuInit` may still succeed on a host whose
    // configuration differs, and refusing the bring-up over a mount would be this code
    // deciding the experiment.
    if let Err(e) = kayfabe_linux_raw::sandbox::remount_proc() {
        eprintln!(
            "kayfabe-isolate: ⚠ could not remount /proc before the CUDA bring-up ({e}); \
             `cuInit` is expected to refuse with CUDA_ERROR_OPERATING_SYSTEM (304) in a PID \
             namespace whose /proc belongs to the parent"
        );
    }
    let (outcome, kernel) = kayfabe_cuda::selftest::bring_up_and_prove();
    // ⊘ One line to the child's own stderr as well as the wire record, because a child that
    // dies between here and the parent's request would otherwise leave no trace at all — and
    // "the isolate went away" is exactly the case where the diagnosis matters most.
    eprintln!(
        "kayfabe-isolate: CUDA-WALK PRE-SANDBOX {} cuda_up={} device={:?} bring_up_ms={:.3} \
         jit_ms={:.3} report_valid={} mapping_matched={} abi_refusal_fired={} why={:?}",
        outcome.token(),
        outcome.cuda_up,
        outcome.device_name,
        outcome.bring_up_us as f64 / 1000.0,
        outcome.jit_us as f64 / 1000.0,
        outcome.report_valid,
        outcome.mapping_matched,
        outcome.abi_refusal_fired,
        outcome.why,
    );
    *g = Some((outcome, kernel));
}

/// ★★★ §w724d's *"empirical risk"*, measured. Run **after** the sandbox and the privilege drop.
///
/// ⊘ If the bring-up never happened this records nothing and says so — an absent probe result
/// must not read as a passing one.
pub fn probe_after_sandbox() {
    let mut g = STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((outcome, kernel)) = g.as_mut() else {
        eprintln!(
            "kayfabe-isolate: ⊘ the post-sandbox probes were asked for with no CUDA bring-up \
             behind them; nothing was measured"
        );
        return;
    };
    let Some(k) = kernel.as_mut() else {
        outcome.probe_relaunch =
            "SKIPPED — CUDA never came up, so there is no context to probe".to_string();
        outcome.probe_failed_launch.clone_from(&outcome.probe_relaunch);
        outcome.probe_other_thread.clone_from(&outcome.probe_relaunch);
        return;
    };
    kayfabe_cuda::selftest::probe_after_sandbox(k, outcome);
    eprintln!(
        "kayfabe-isolate: CUDA-WALK POST-SANDBOX relaunch={:?} failed_launch={:?} \
         other_thread={:?}",
        outcome.probe_relaunch, outcome.probe_failed_launch, outcome.probe_other_thread
    );
}

/// The outcome, rendered for the wire. ⊘ A flat `key=value` line rather than a struct on the
/// protocol: every field is for a human reading a boot log, the shape is not something any
/// code branches on, and a new field must not be a wire-format change.
#[must_use]
pub fn report_line() -> String {
    let g = STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((o, _)) = g.as_ref() else {
        return "CUDA_WALK=ABSENT reason=\"this isolate never ran a CUDA bring-up\"".to_string();
    };
    format!(
        "CUDA_WALK={} cuda_up={} device={:?} ptx_bytes={} bring_up_ms={:.3} jit_ms={:.3} \
         report_valid={} mapping_matched={} abi_refusal_fired={} report={:?} \
         probe_relaunch={:?} probe_failed_launch={:?} probe_other_thread={:?} why={:?}",
        o.token(),
        o.cuda_up,
        o.device_name,
        o.ptx_bytes,
        o.bring_up_us as f64 / 1000.0,
        o.jit_us as f64 / 1000.0,
        o.report_valid,
        o.mapping_matched,
        o.abi_refusal_fired,
        o.report,
        o.probe_relaunch,
        o.probe_failed_launch,
        o.probe_other_thread,
        o.why,
    )
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// ★★★★★ THE LIVE SHADOW — `SINGLE_STORE_PLAN.md` §6 step 1's live half.
// ─────────────────────────────────────────────────────────────────────────────────────────

/// The staged image: the guest's page-table pages, **relocated into a compact arena** by
/// `kayfabe_mmu::walkshadow::build_image` and shipped here a frame at a time.
///
/// ⊘ Its own lock, not `STATE`'s. Staging is a stream of chunks and a run is one launch; a
/// single lock would make every chunk contend with nothing, and would make a poisoned
/// bring-up take the staging buffer with it.
static IMAGE: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// A chunk that would leave the declared image.
const WS_CHUNK_OUT_OF_RANGE: u32 = 0x5748_0010;
/// A chunk arrived for an image no one declared, or with a different `span` than the one
/// that was. ⊘ Refused rather than re-declared: a `span` that changes mid-image means the
/// parent and the child disagree about what is being assembled.
const WS_SPAN_MISMATCH: u32 = 0x5748_0011;
/// The image is empty — no `off == 0` chunk ever arrived.
const WS_NO_IMAGE: u32 = 0x5748_0012;
/// CUDA never came up in this isolate, so there is no kernel to run.
const WS_NO_KERNEL: u32 = 0x5748_0013;
/// The declared image is larger than [`IMAGE_MAX`].
const WS_IMAGE_TOO_LARGE: u32 = 0x5748_0014;
/// `cuMemAlloc` / `cuMemcpyHtoD` / `cuLaunchKernel` refused. ⚠ One code for the whole
/// launch: the *reason* is printed to this child's stderr in full, and collapsing three
/// CUDA errors into three wire codes would put the parsing of CUDA's vocabulary on the
/// protocol.
const WS_LAUNCH_FAILED: u32 = 0x5748_0015;
/// More address spaces than the kernel's table holds.
const WS_TOO_MANY_PDBS: u32 = 0x5748_0016;

/// The largest image this isolate will hold. 64 MiB — comfortably above `[measured w724c]`
/// **7.3 MiB** of resident page tables, and small enough that a hostile `span` cannot make
/// the isolate the thing that runs the host out of memory.
const IMAGE_MAX: u64 = 64 << 20;

/// ★★★ **Stage one chunk.** `off == 0` declares a fresh image of `span` bytes.
///
/// # Errors
/// A named status. ⚠ Every refusal also prints its reason to this child's stderr, because a
/// `u32` on the wire is what the parent's census can count and the sentence is what tells
/// somebody what to do about it.
pub fn stage(span: u64, off: u64, bytes: &[u8]) -> Result<(), u32> {
    if span == 0 || span > IMAGE_MAX {
        eprintln!("kayfabe-isolate: ⊘ WALK-SHADOW stage refused: span={span} (max {IMAGE_MAX})");
        return Err(WS_IMAGE_TOO_LARGE);
    }
    let mut g = IMAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if off == 0 {
        // ⊘ `resize` then `fill`, not `clear` + `resize`: an image re-declared at the same
        // span must not inherit the previous refresh's bytes in the region this refresh does
        // not write. A stale page decodes perfectly well and would be a mapping from the
        // last refresh reported as if it were this one's.
        g.clear();
        g.resize(span as usize, 0);
    } else if g.len() as u64 != span {
        eprintln!(
            "kayfabe-isolate: ⊘ WALK-SHADOW stage refused: chunk at {off:#x} declares \
             span={span} but the image in hand is {} bytes",
            g.len()
        );
        return Err(WS_SPAN_MISMATCH);
    }
    let Some(end) = off.checked_add(bytes.len() as u64) else {
        return Err(WS_CHUNK_OUT_OF_RANGE);
    };
    if end > span {
        eprintln!(
            "kayfabe-isolate: ⊘ WALK-SHADOW stage refused: chunk [{off:#x},{end:#x}) leaves \
             an image of {span} bytes"
        );
        return Err(WS_CHUNK_OUT_OF_RANGE);
    }
    g[off as usize..end as usize].copy_from_slice(bytes);
    Ok(())
}

/// ★★★★★ **RUN THE WALK KERNEL** over the staged image and return the report **as bytes**.
///
/// The layout is `kf_walk.h`'s own: `KfReportHeader`, then `pdb_count` `KfPdbEntry`s, then
/// `run_count` `KfMapRun`s, each `#[repr(C)]` and copied verbatim.
///
/// ⊘ **The child does not interpret the report.** `kayfabe_mmu::walkreport` is a *second,
/// independent* parser of this format, written against `kf_walk.h` rather than against the
/// Rust mirror — and the whole value of having two is lost the moment one of them is used to
/// pre-digest the bytes for the other.
///
/// # Errors
/// A named status; the reason is printed to this child's stderr.
pub fn run(pdbs: &[u64]) -> Result<Vec<u8>, u32> {
    if pdbs.is_empty() || pdbs.len() > kayfabe_cuda::abi::KF_MAX_PDB {
        eprintln!(
            "kayfabe-isolate: ⊘ WALK-SHADOW run refused: {} address spaces (the kernel's \
             table holds {})",
            pdbs.len(),
            kayfabe_cuda::abi::KF_MAX_PDB
        );
        return Err(WS_TOO_MANY_PDBS);
    }
    let image = {
        let g = IMAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if g.is_empty() {
            eprintln!("kayfabe-isolate: ⊘ WALK-SHADOW run refused: no image was staged");
            return Err(WS_NO_IMAGE);
        }
        g.clone()
    };
    let mut st = STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((_, Some(k))) = st.as_mut() else {
        eprintln!(
            "kayfabe-isolate: ⊘ WALK-SHADOW run refused: CUDA never came up in this isolate"
        );
        return Err(WS_NO_KERNEL);
    };
    // ★★★★★ **THE CONTEXT IS PER-THREAD CURRENT, AND THIS IS NOT THE BRING-UP THREAD.**
    // `[measured w731]` without this every `cuMemAlloc` here returns
    // `CUDA_ERROR_INVALID_CONTEXT` (201) — 2 115 times in one boot — while the selftest, which
    // runs on the bring-up thread, reports `CUDA_WALK=OK`.
    if let Err(e) = k.make_current() {
        eprintln!("kayfabe-isolate: ⊘ WALK-SHADOW could not make the CUDA context current: {e}");
        return Err(WS_LAUNCH_FAILED);
    }
    let img = match k.upload(&image) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("kayfabe-isolate: ⊘ WALK-SHADOW upload refused: {e}");
            return Err(WS_LAUNCH_FAILED);
        }
    };
    let report = k.refresh(img.ptr(), img.len(), pdbs);
    // ⊘ Released on BOTH paths and before the `?`: a refused launch that leaked its image
    // would run the device out of memory over a boot's worth of refreshes, and the symptom
    // would arrive as a CUDA failure hundreds of refreshes after the one that caused it.
    k.release(img);
    let report = match report {
        Ok(r) => r,
        Err(e) => {
            eprintln!("kayfabe-isolate: ⊘ WALK-SHADOW launch refused: {e}");
            return Err(WS_LAUNCH_FAILED);
        }
    };
    let mut out = Vec::with_capacity(
        core::mem::size_of::<kayfabe_cuda::abi::KfReportHeader>()
            + report.pdbs.len() * core::mem::size_of::<kayfabe_cuda::abi::KfPdbEntry>()
            + report.runs.len() * core::mem::size_of::<kayfabe_cuda::abi::KfMapRun>(),
    );
    out.extend_from_slice(kayfabe_cuda::driver_unsafe::view_bytes(&report.header));
    for p in &report.pdbs {
        out.extend_from_slice(kayfabe_cuda::driver_unsafe::view_bytes(p));
    }
    for r in &report.runs {
        out.extend_from_slice(kayfabe_cuda::driver_unsafe::view_bytes(r));
    }
    Ok(out)
}
