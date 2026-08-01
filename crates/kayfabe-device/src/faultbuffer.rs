//! ★ **Where the guest's replayable fault buffer is — written down, and nothing else.**
//!
//! `docs/design/resume_from_fault.md` §7 step **5a**, which is the only part of step 5
//! this port does: *"One RPC; turns "where is the buffer" from an unknown into a fact. Do
//! this now — it costs nothing and it de-risks everything after it."*
//!
//! ⊘ **There is no fault buffer.** Nothing here writes a 32-byte packet, moves `PUT`, or
//! pulses an interrupt leaf, and none of that is half-built. Steps 5b–5d are gated on a
//! decision this does not pre-empt (`resume_from_fault.md` §7 step 0).
//!
//! # ★★ Recording is not answering — the same rule as [`crate::unserviced`]
//!
//! [`FaultBufferRecorder`] is a [`CommandPolicy`] that **always returns `None`**. It sees
//! the command, decodes it, writes it down, and declines — so the FSM refuses it by name
//! exactly as it did before this module existed, and the guest observes **nothing new**.
//!
//! That property is the whole reason this is free. `kgmmuFaultBufferReplayableAllocate_IMPL`
//! frees the buffer and propagates the status when the control fails
//! (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:1262-1268`), so *answering* would
//! change what the guest's UVM bring-up does, on a path this port cannot yet service. A
//! link that both recorded and answered would be instrumentation that changes what it
//! observes.
//!
//! ★ **And that is also the whole of its sticky-answer argument.** The guest's control
//! cache is populated only from a reply the RPC layer treats as accepted
//! (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11093-11104`, `ogkm-610: :10898-10909` — the
//! `else` arm of `if (rpc_params->status != NV_OK)`). A policy that never returns `Some`
//! authors no reply at all, so there is nothing for either branch to persist. ⊘ Note the
//! direction: this is safe because it declines, **not** because the control it decodes is
//! outside the mask — `NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER` is a fn-76
//! control and the day this recorder starts answering it inherits the whole question. See
//! [`crate::sticky`]; `tests/tests/sticky_answer.rs` executes the claim against this type.
//!
//! ⚠ **What it therefore does NOT establish.** Seeing this control means the guest asked;
//! it does not mean the guest went on to use the buffer, because we refuse. The C's
//! captures show the *registers* being polled on the compute path
//! (`resume_from_fault.md` §4.3 `[meas]`), which is the independent half of the evidence.

use std::sync::{Arc, Mutex};

use core::sync::atomic::{AtomicU64, Ordering};

use kayfabe_abi::faultbuffer::{
    FaultBufferRegistration, NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER,
    decode_register_fault_buffer,
};
use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

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

/// The chain link: writes down the fault-buffer registration, and answers nothing.
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

impl CommandPolicy for FaultBufferRecorder {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let h = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        if h.cmd != NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER {
            return None;
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
        // ⊘ Always `None`. See this module's docs: recording is not answering, and
        // answering would change a bring-up path this port cannot service.
        None
    }
}

kayfabe_util::assert_send_sync!(FaultBufferNote, FaultBufferLog, FaultBufferRecorder);
