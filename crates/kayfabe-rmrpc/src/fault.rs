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

use kayfabe_abi::notifier::ErrorNotification;
use kayfabe_abi::rc::{EngineRoute, RC_NOTIFIER_SCOPE_TSG, RcTriggered};
use kayfabe_arch::fault::MmuFaultCodes;
use kayfabe_core::fault::FaultReport;
use kayfabe_gsp::boot::GspFsm;
use kayfabe_gsp::ram::GuestRam;
use kayfabe_gsp::{FunctionCodes, GspFault, OutgoingRpc};

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
    /// The engine route does not fit the notifier's `info16`.
    ///
    /// ★ `nv2080EngineType` is a `NvU32` in the RC event and a `NvV16` in the notifier —
    /// RM narrows it with a cast, `(NvU16)gpuGetNv2080EngineType(localRmEngineType)`
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/rc/kernel_rc_notification.c:74-97`).
    /// Unreachable for every route this port has (`NV2080_ENGINE_TYPE_GRAPHICS` = 1), and
    /// written as a refusal rather than a truncating cast so it stays unreachable: a
    /// truncated engine in the notifier would disagree with the one in the event, and the
    /// two are read by different consumers.
    EngineOutOfRange {
        /// The `nv2080EngineType` that did not fit.
        engine: u32,
    },
}

/// ★★★ **One fault, delivered in two places, and the order between them is the contract.**
///
/// The GSP's job under this split is *both* halves, and NVIDIA states the split on the
/// receiver we are impersonating:
///
/// > *"RC error handling ("Channel Teardown sequence") is executed in GSP-RM. Client
/// > notifications, OS interaction etc happen in CPU-RM (Kernel RM)."*
/// > (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:541-545`)
///
/// and says explicitly *why* the notifier is written first: *"GSP writes to notifiers to
/// avoid a race condition between the time when an error is handled and when the client
/// receives notifications"* (`ogkm-580:
/// src/nvidia/src/kernel/gpu/rc/kernel_rc_notification.c:352-372`). With Confidential
/// Compute off, CPU-RM does not write it for us — the conditional is quoted in
/// `kayfabe_core::fault::NotAttributable::NoErrorNotifier`.
///
/// ⇒ Posting the event without the write leaves every consumer polling a zero, which is
/// the hang task #111 exists to remove. This type exists so that fact is carried by the
/// *type* and not by a comment: there is no way to obtain the event without also
/// obtaining the bytes and the address, and [`Self::deliver`] is the only ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "an undelivered emission is a fault the guest is never told about"]
pub struct FaultEmission {
    /// Guest-physical address of the channel's 16-byte error-notification record.
    pub notifier_gpa: u64,
    /// The record itself.
    pub notification: ErrorNotification,
    /// The `RC_TRIGGERED` event.
    pub rpc: OutgoingRpc,
}

impl FaultEmission {
    /// Write the notifier, then post the event.
    ///
    /// ★ The notification is published in **two** writes — everything, then the status
    /// half-word — because `status` is the only field a waiter polls and a record
    /// published status-first is a window in which it reads "there is an error" beside a
    /// stale exception type. That is RM's own order
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/method_notification.c:181-185`) and
    /// [`ErrorNotification::PUBLISH_SPLIT`] is where it splits.
    ///
    /// # Errors
    ///
    /// [`FaultDeliveryError::Notifier`] if guest RAM refused the write — in which case
    /// **the event is not posted**, because an RC the guest cannot see the reason for is
    /// worse than the refusal we still have in hand. [`FaultDeliveryError::Transport`] if
    /// the GSP transport refused the event, after the notifier was already written; that
    /// residue is deliberate and named in the variant.
    pub fn deliver(
        &self,
        ram: &mut dyn GuestRam,
        fsm: &mut GspFsm,
    ) -> Result<(), FaultDeliveryError> {
        let bytes = self.notification.encode();
        let split = ErrorNotification::PUBLISH_SPLIT;
        ram.write(self.notifier_gpa, &bytes[..split])
            .map_err(FaultDeliveryError::Notifier)?;
        ram.write(self.notifier_gpa + split as u64, &bytes[split..])
            .map_err(FaultDeliveryError::Notifier)?;
        fsm.post_event(ram, &self.rpc)
            .map_err(FaultDeliveryError::Transport)
    }
}

/// Why a [`FaultEmission`] did not reach the guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultDeliveryError {
    /// Guest RAM refused the notifier write. Nothing was posted.
    Notifier(kayfabe_gsp::RamRefused),
    /// The notifier was written and the transport refused the event.
    ///
    /// ⚠ The residue is real and is not cleaned up: the guest now has a non-zero
    /// notifier and no event. That is the *safe* half — a poller unsticks and reports an
    /// RC error, which is true — but no `NV2080_NOTIFIERS_RC_ERROR` callback fires. It is
    /// named rather than swallowed because a caller that retries would write the same
    /// bytes again, which is idempotent, and a caller that gives up must know the guest
    /// is in a half-told state.
    Transport(GspFault),
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
    timestamp: u64,
) -> Result<FaultEmission, FaultEmitRefusal> {
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
    // ★ The notifier carries the SAME two numbers the event routes on — `exceptType` in
    // `info32` and the `nv2080EngineType` in `info16` — because that is what
    // `krcErrorSetNotifier_IMPL` passes down (`ogkm-580:
    // src/nvidia/src/kernel/gpu/rc/kernel_rc_notification.c:70-97`, `:326-338`). Building
    // them once, here, is what stops the two halves of one fault disagreeing.
    let engine_code =
        u16::try_from(engine.code()).map_err(|_| FaultEmitRefusal::EngineOutOfRange {
            engine: engine.code(),
        })?;
    let notification = ErrorNotification::rc(except_type, engine_code, timestamp);
    // The access type is deliberately NOT in this message: `rpc_rc_triggered_v17_02` has
    // no access-type field (`ogkm-580: src/nvidia/generated/g_rpc-structures.h:1481-1496`).
    // It exists in the 32-byte hardware fault-buffer entry, which this path does not
    // write — see the design note §5 for why, and for what that costs.
    let _ = codes.access_type(report.access);
    Ok(FaultEmission {
        notifier_gpa: report.notifier_gpa,
        notification,
        rpc: OutgoingRpc {
            function: functions.rc_triggered,
            // An unsolicited event answers no request, so it carries no request's
            // sequence. The transport stamps the queue's own `seqNum`; this field is the
            // RPC transaction id and zero is what an event with no transaction has.
            sequence: 0,
            rpc_result: 0,
            rpc_result_private: 0,
            payload: ev.encode(),
        },
    })
}
