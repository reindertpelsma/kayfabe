//! ★ **Where the guest's replayable fault buffer is — written down, and nothing else.**
//!
//! `docs/design/resume_from_fault.md` §7 step **5a**, which is the only part of step 5
//! this port does: *"One RPC; turns "where is the buffer" from an unknown into a fact. Do
//! this now — it costs nothing and it de-risks everything after it."*
//!
//! ⊘ **There is no fault buffer.** Nothing here writes a 32-byte packet, moves `PUT`, or
//! pulses an interrupt leaf, and none of that is half-built. Steps 5b–5d are gated on a
//! decision this does not pre-empt (`resume_from_fault.md` §7 step 0) — and §14.41 does not
//! pre-empt it either: serving the *registration* is 5a's own sentence, not 5b.
//!
//! # ★★ Recording is not answering — and §14.41 changed WHO answers, not that rule
//!
//! ⊘⊘ **REFUTED, and the refutation is this module's own** —
//! `[measured 2026-08-09, boot `pu1448` at `ef20ccc`]`. These docs used
//! to argue that declining was the *safe* half of the trade: *"answering would change what
//! the guest's UVM bring-up does, on a path this port cannot yet service."* That boot showed
//! what declining actually does — `kgmmuFaultBufferReplayableAllocate_IMPL`
//! propagates the `0x56` verbatim (`ogkm-580: kern_gmmu.c:1261-1272`),
//! `faultbufConstruct_IMPL` re-returns it, `UVM_REGISTER_GPU` fails and **`cuInit` dies**.
//! ⇒ Declining is not the conservative choice; it is a wall. The sentence was true about the
//! mechanism (*answering does change bring-up*) and wrong about its sign.
//!
//! ★ What survives intact is the **separation**: recording and answering are still different
//! jobs done by different links. [`crate::inittables`]' `RegisterFaultBuffer` arm answers;
//! this type records. And it now records from a seat where *"it declines"* is not a promise
//! at all — [`FaultBufferRecorder`] implements [`CommandObserver`], whose `observe` returns
//! **nothing**, so `rustc` rather than a reviewer guarantees it changes no reply byte. That
//! is strictly stronger than the always-`None` [`kayfabe_gsp::CommandPolicy`] it replaced,
//! and it is why the seat had to move: `InitTablePolicy` *terminates* the chain for this id,
//! so a recorder at the tail would never see the command again (`crate::gvaspub`'s lesson,
//! `execution_plane_increments.md` §14.8).
//!
//! ★ **The sticky-answer question moves with the answer, and is discharged where it lands.**
//! The guest's control cache is populated only from a reply the RPC layer treats as accepted
//! (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11093-11104`, `ogkm-610: :10898-10909` — the
//! `else` arm of `if (rpc_params->status != NV_OK)`). This type authors no reply, so it has
//! nothing to persist and no row in [`crate::sticky::POLICY_DISPOSITIONS`] — it is not a
//! `CommandPolicy`. The answerer is `InitTablePolicy`, whose row is `Guarded`, and
//! `0x20800a9b & 0x8000 == 0` so branch (b) cannot reach it either way.
//!
//! ⚠ **What this still does NOT establish.** Seeing this control means the guest asked and
//! we said yes. It does not mean a fault will ever be delivered — nothing here writes a
//! 32-byte packet, moves `PUT`, or pulses an interrupt leaf, and none of that is half-built.
//! [`kayfabe_abi::faultbuffer::DELIVERY_UNBUILT`] is that gap said out loud, and
//! [`FaultBufferLog::total`] is what makes the report print it.

use std::sync::{Arc, Mutex};

use core::sync::atomic::{AtomicU64, Ordering};

use kayfabe_abi::faultbuffer::{
    FaultBufferRegistration, NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER,
    decode_register_fault_buffer,
};
use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_gsp::{CommandObserver, RpcCommand, RpcFunction};

/// One registration attempt, as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultBufferNote {
    /// The params decoded.
    Registered(FaultBufferRegistration),
    /// The command arrived and its params did **not** decode.
    ///
    /// ★ Its own variant rather than a dropped record. *"The guest never asked"* and *"the
    /// guest asked in a shape we could not read"* are different findings, and the second
    /// is the one that means this port's layout is wrong — which is precisely the thing a
    /// step-5 build would need to know before trusting an address.
    Malformed {
        /// Bytes the params carried.
        len: usize,
    },
}

/// The shared record. Cloneable so the plane and the chain link hold the same one.
#[derive(Debug, Clone, Default)]
pub struct FaultBufferLog {
    seen: Arc<Mutex<Vec<FaultBufferNote>>>,
    total: Arc<AtomicU64>,
}

/// How many registrations are remembered.
///
/// ⊘ Bounded for the same reason [`crate::unserviced::UNSERVICED_SAMPLE_MAX`] is: the
/// guest drives how often the control arrives, and a driver that retries a refused control
/// in a loop must not be able to grow a host allocation. The **count** is unbounded and
/// free; the list is not.
pub const FAULT_BUFFER_SAMPLE_MAX: usize = 8;

impl FaultBufferLog {
    /// A fresh, empty log.
    #[must_use]
    pub fn new() -> FaultBufferLog {
        FaultBufferLog::default()
    }

    /// How many times the control arrived, including repeats and anything past
    /// [`FAULT_BUFFER_SAMPLE_MAX`].
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// The registrations remembered, in arrival order.
    #[must_use]
    pub fn sample(&self) -> Vec<FaultBufferNote> {
        let s = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        s.clone()
    }

    /// Record one. Always counted; kept only while there is room.
    ///
    /// ★ Unlike the unserviced ledger this does **not** de-duplicate. A guest that
    /// registers the buffer twice at two addresses is a fact worth having, and collapsing
    /// the two would hide a re-registration — which is exactly what an unregister/register
    /// cycle looks like from here.
    pub fn note(&self, note: FaultBufferNote) {
        self.total.fetch_add(1, Ordering::Relaxed);
        let mut s = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        if s.len() < FAULT_BUFFER_SAMPLE_MAX {
            s.push(note);
        }
    }
}

/// The front-seat observer: writes down the fault-buffer registration, and **cannot**
/// answer — `observe` has no return value.
#[derive(Debug, Clone)]
pub struct FaultBufferRecorder {
    driver: DriverAbiTable,
    log: FaultBufferLog,
}

impl FaultBufferRecorder {
    /// Build a recorder writing into `log`.
    #[must_use]
    pub fn new(driver: DriverAbiTable, log: FaultBufferLog) -> FaultBufferRecorder {
        FaultBufferRecorder { driver, log }
    }
}

impl CommandObserver for FaultBufferRecorder {
    fn observe(&mut self, cmd: &RpcCommand) {
        if cmd.function != RpcFunction::RmControl {
            return;
        }
        let Ok(h) = self.driver.decode_rpc_control(&cmd.payload) else {
            return;
        };
        if h.cmd != NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER {
            return;
        }
        // The declared params window, bounded by what actually arrived — the same
        // `params_at`/`params_size` pair `kayfabe_rmrpc::translate_control` bounds, and
        // bounded here too because `paramsSize` is the guest's assertion and never a fact.
        let params = cmd
            .payload
            .get(h.params_at..)
            .and_then(|tail| tail.get(..h.params_size as usize))
            .unwrap_or(&[]);
        self.log.note(match decode_register_fault_buffer(params) {
            Ok(r) => FaultBufferNote::Registered(r),
            Err(_) => FaultBufferNote::Malformed { len: params.len() },
        });
        // ⊘ Nothing is returned, and nothing CAN be: `observe` has no return value, so
        // "this link changes no reply byte" is rustc's guarantee rather than a comment.
    }
}

kayfabe_util::assert_send_sync!(FaultBufferNote, FaultBufferLog, FaultBufferRecorder);
