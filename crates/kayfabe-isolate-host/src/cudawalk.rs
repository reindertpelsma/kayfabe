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
        return;
    };
    kayfabe_cuda::selftest::probe_after_sandbox(k, outcome);
    eprintln!(
        "kayfabe-isolate: CUDA-WALK POST-SANDBOX relaunch={:?} failed_launch={:?}",
        outcome.probe_relaunch, outcome.probe_failed_launch
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
         probe_relaunch={:?} probe_failed_launch={:?} why={:?}",
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
        o.why,
    )
}
