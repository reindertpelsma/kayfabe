//! The **emission** half of task #111: a core-side [`FaultReport`] becomes the RPC event
//! the guest's own driver already knows how to handle.
//!
//! `docs/design/simulated_gpu_fault.md` §6. This lives in the bridge crate for the same
//! structural reason `translate` does: it is the seam between `kayfabe_core`'s
//! vocabulary and `kayfabe_gsp`'s wire, and this is the only crate permitted to name
//! both (§1.2, CI greps for it). Putting it in the core would give the core an RPC
//! encoder; putting it in the GSP crate would give the faked GSP a view of the RM graph,
//! which `gsp_core_bridge.md` §1.2 forbids on purpose.
//!
//! ★ It is **stateless and total**, like the rest of this crate: one report in, one
//! outcome out, no cursor, no dedup, no memory that a channel has already faulted. A
//! dedup cache here would be the same defect as a handle cache — the *decision* to fault
//! a channel once belongs to whoever owns the channel's state, not to an encoder.

use kayfabe_abi::rc::{EngineRoute, RC_NOTIFIER_SCOPE_TSG, RcTriggered};
use kayfabe_arch::fault::MmuFaultCodes;
use kayfabe_core::fault::FaultReport;
use kayfabe_gsp::{FunctionCodes, OutgoingRpc};

/// Why a [`FaultReport`] could not be turned into an event.
///
/// ★★ The whole reason this is a `Result` and not an `OutgoingRpc`. The receiver
/// `_kgspRpcRCTriggered` resolves `nv2080EngineType` to a channel-id manager and
/// **returns early on failure** (`ogkm-580:
/// src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:578-583`), silently. So a message built
/// with an engine code we guessed is parsed, dropped, and counted by us as delivered —
/// a false green on the emitter's side, which is the failure this project has named
/// four times (`suspect_the_instrument_first`). Refusing to build it is the only
/// outcome that stays honest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultEmitRefusal {
    /// No `nv2080EngineType` names this engine honestly — see [`EngineRoute`].
    ///
    /// Today this is every non-GR engine, and the copy engine is the one that matters:
    /// `EngineKind::Ce` carries no instance, and the copy-engine ids are a *range*. The
    /// caller must escalate to the operator; it must not fall back to GR, which would
    /// attribute a copy-engine fault to the graphics engine.
    NoEngineRoute {
        /// The engine that has no route.
        engine: kayfabe_arch::ids::EngineKind,
    },
    /// The channel's runlist index does not fit the `chid` field.
    ///
    /// `VChid` is a `u16` and `chid` is a `u32`, so this is unreachable today and is
    /// written as a refusal rather than a cast so that it stays unreachable if `VChid`
    /// ever widens. A truncating cast here would attribute the fault to a *different,
    /// live* channel, which is worse than not sending.
    ChidOutOfRange {
        /// The value that did not fit.
        vchid: u32,
    },
}

/// Build the `RC_TRIGGERED` event for `report`.
///
/// `codes` is the Axis-B encoding table for the chip being emulated; `except_type` is the
/// `ROBUST_CHANNEL_*` code (`kayfabe_abi::generated::rpc::ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT`
/// for an MMU fault — the number a kernel log prints as `Xid 31`); `functions` supplies
/// the event id from the same Axis-A table the rest of the GSP transport uses.
///
/// ★ The scope is [`RC_NOTIFIER_SCOPE_TSG`], matching the driver's own MMU-fault handler
/// (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/volta/kern_gmmu_gv100.c:2127-2131`).
///
/// # Errors
///
/// [`FaultEmitRefusal`] — and a refusal is a **fault the guest is not told about**, so a
/// caller that discards it has reintroduced the hang this whole path exists to remove.
pub fn rc_triggered_for(
    report: &FaultReport,
    codes: &dyn MmuFaultCodes,
    functions: &FunctionCodes,
    except_type: u32,
) -> Result<OutgoingRpc, FaultEmitRefusal> {
    let engine = EngineRoute::for_engine(report.engine).ok_or(FaultEmitRefusal::NoEngineRoute {
        engine: report.engine,
    })?;
    let chid = u32::from(report.vchid.0);
    let ev = RcTriggered {
        engine,
        chid,
        except_type,
        scope: RC_NOTIFIER_SCOPE_TSG,
        mmu_fault_addr: report.va.0,
        mmu_fault_type: codes.fault_type(report.cause),
    };
    // The access type is deliberately NOT in this message: `rpc_rc_triggered_v17_02` has
    // no access-type field (`ogkm-580: src/nvidia/generated/g_rpc-structures.h:1481-1496`).
    // It exists in the 32-byte hardware fault-buffer entry, which this path does not
    // write — see the design note §5 for why, and for what that costs.
    let _ = codes.access_type(report.access);
    Ok(OutgoingRpc {
        function: functions.rc_triggered,
        // An unsolicited event answers no request, so it carries no request's sequence.
        // The transport stamps the queue's own `seqNum`; this field is the RPC
        // transaction id and zero is what an event with no transaction has.
        sequence: 0,
        rpc_result: 0,
        rpc_result_private: 0,
        payload: ev.encode(),
    })
}
