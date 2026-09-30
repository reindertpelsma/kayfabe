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
//! is strictly stronger than the always-`None` [`kf_gsp::CommandPolicy`] it replaced,
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
//! [`kf_abi::faultbuffer::DELIVERY_UNBUILT`] is that gap said out loud, and
//! [`FaultBufferLog::total`] is what makes the report print it.

use std::sync::{Arc, Mutex};

use core::sync::atomic::{AtomicU64, Ordering};

use kf_abi::faultbuffer::{
    AccessCntrBufferRegistration, FaultBufferRegistration,
    NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_CLIENT_SHADOW_FAULT_BUFFER,
    NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER,
    NV2080_CTRL_CMD_INTERNAL_UVM_REGISTER_ACCESS_CNTR_BUFFER, ShadowFaultBufferRegistration,
    decode_register_access_cntr_buffer, decode_register_client_shadow_fault_buffer,
    decode_register_fault_buffer,
};
use kf_abi::versions::DriverAbiTable;
use kf_gsp::{CommandObserver, RpcCommand, RpcFunction};

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
    /// ★ `0x20800a9d` — a **client shadow** fault buffer registration decoded.
    ///
    /// ⊘ Its own variant rather than a field on [`Self::Registered`]: the two controls
    /// register different buffers, with different geometry bounds, and — the part that
    /// matters — different promises. Answering `0x20800a9b` says a register we serve will
    /// stay empty; answering this one says **we** will write packets into the guest's own
    /// sysmem. Collapsing them into one record would make the report unable to say which
    /// promise a boot took on.
    ShadowRegistered(ShadowFaultBufferRegistration),
    /// `0x20800a9d` arrived and its params did **not** decode.
    ShadowMalformed {
        /// Bytes the params carried.
        len: usize,
    },
    /// ★ `0x20800a1d` — an **access-counter** notification buffer registration decoded.
    ///
    /// ⊘ A third variant for the third buffer, for [`Self::ShadowRegistered`]'s reason: this
    /// is the one whose SIZE this port also invents, so a report that could not tell it apart
    /// from the other two could not say which fiction a boot was resting on.
    AccessCntrRegistered(AccessCntrBufferRegistration),
    /// `0x20800a1d` arrived and its params did **not** decode.
    AccessCntrMalformed {
        /// Bytes the params carried.
        len: usize,
    },
}

/// The shared record. Cloneable so the plane and the chain link hold the same one.
#[derive(Debug, Clone, Default)]
pub struct FaultBufferLog {
    seen: Arc<Mutex<Vec<FaultBufferNote>>>,
    total: Arc<AtomicU64>,
    shadow_total: Arc<AtomicU64>,
    access_cntr_total: Arc<AtomicU64>,
    /// ★ The LATEST decoded replayable registration and its generation (bumped per registration)
    /// — what the guest fault plane (`V3_UVM_GUEST_FAULT_PLANE.md` §3.1) follows. ⊘ Separate from
    /// the capped sample: the sample stops at [`FAULT_BUFFER_SAMPLE_MAX`], the buffer in use
    /// never may.
    latest: Arc<Mutex<Option<FaultBufferRegistration>>>,
    generation: Arc<AtomicU64>,
    /// ★ Who is told, synchronously, when a replayable registration decodes
    /// ([`FaultBufferLog::on_registration`]).
    hook: RegistrationHookCell,
}

/// ★ A listener for replayable registrations — the guest fault plane
/// (`V3_UVM_GUEST_FAULT_PLANE.md` §3.1). Called on the drainer, inside the served `0x20800a9b`,
/// BEFORE its reply: the guest's UVM reads `GET`/`PUT` once right after the registration, so the
/// plane's reset of both must already be visible. ⊘ It must not block (the drainer holds the GSP
/// lock): bookkeeping and shadow stores only.
pub type RegistrationHook = Box<dyn Fn(&FaultBufferRegistration) + Send + Sync>;

/// The set-once cell holding the hook (its own type so the log keeps its `Debug`).
#[derive(Clone, Default)]
struct RegistrationHookCell(Arc<std::sync::OnceLock<RegistrationHook>>);

impl core::fmt::Debug for RegistrationHookCell {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(if self.0.get().is_some() {
            "RegistrationHook(set)"
        } else {
            "RegistrationHook(none)"
        })
    }
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

    /// How many times `0x20800a9b` — the **replayable hardware** buffer — arrived, including
    /// repeats and anything past [`FAULT_BUFFER_SAMPLE_MAX`].
    ///
    /// ⊘ This counter and [`Self::shadow_total`] are kept apart at the *source* rather than
    /// derived from [`Self::sample`], because the sample is capped and a count read off it
    /// could never exceed the cap — the defect `unserviced_len` shipped with
    /// (`a_saturated_instrument_looks_exactly_like_absence`).
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// How many times `0x20800a9d` — the **client shadow** buffer — arrived.
    ///
    /// ★ Separate from [`Self::total`] because the two controls carry different promises, and
    /// a boot's report has to be able to say which one it took on.
    #[must_use]
    pub fn shadow_total(&self) -> u64 {
        self.shadow_total.load(Ordering::Relaxed)
    }

    /// How many times `0x20800a1d` — the **access-counter** buffer — arrived.
    #[must_use]
    pub fn access_cntr_total(&self) -> u64 {
        self.access_cntr_total.load(Ordering::Relaxed)
    }

    /// ★ How many replayable registrations DECODED so far — a change means a new buffer.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// ★ Install the registration listener (once; a second call is refused and returns `false`).
    pub fn on_registration(&self, hook: RegistrationHook) -> bool {
        self.hook.0.set(hook).is_ok()
    }

    /// ★ The latest decoded replayable registration (the buffer the guest uses now).
    #[must_use]
    pub fn latest(&self) -> Option<FaultBufferRegistration> {
        self.latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
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
        // ⊘ `match` rather than a boolean parameter: the counter a note lands on is a
        // property OF THE NOTE, so the two cannot be given to each other by a caller that
        // passes the wrong flag. `a_caller_side_inversion_is_invisible_to_every_test_of_the
        // _callee` is why this is not an argument.
        match note {
            FaultBufferNote::Registered(_) | FaultBufferNote::Malformed { .. } => {
                self.total.fetch_add(1, Ordering::Relaxed);
            }
            FaultBufferNote::ShadowRegistered(_) | FaultBufferNote::ShadowMalformed { .. } => {
                self.shadow_total.fetch_add(1, Ordering::Relaxed);
            }
            FaultBufferNote::AccessCntrRegistered(_)
            | FaultBufferNote::AccessCntrMalformed { .. } => {
                self.access_cntr_total.fetch_add(1, Ordering::Relaxed);
            }
        }
        if let FaultBufferNote::Registered(r) = &note {
            *self.latest.lock().unwrap_or_else(|e| e.into_inner()) = Some(r.clone());
            self.generation.fetch_add(1, Ordering::AcqRel);
            if let Some(h) = self.hook.0.get() {
                h(r);
            }
        }
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
        enum Which {
            Replayable,
            Shadow,
            AccessCntr,
        }
        let which = match h.cmd {
            NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER => Which::Replayable,
            NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_CLIENT_SHADOW_FAULT_BUFFER => Which::Shadow,
            NV2080_CTRL_CMD_INTERNAL_UVM_REGISTER_ACCESS_CNTR_BUFFER => Which::AccessCntr,
            _ => return,
        };
        // The declared params window, bounded by what actually arrived — the same
        // `params_at`/`params_size` pair `kayfabe_rmrpc::translate_control` bounds, and
        // bounded here too because `paramsSize` is the guest's assertion and never a fact.
        let params = cmd
            .payload
            .get(h.params_at..)
            .and_then(|tail| tail.get(..h.params_size as usize))
            .unwrap_or(&[]);
        self.log.note(match which {
            Which::Replayable => match decode_register_fault_buffer(params) {
                Ok(r) => FaultBufferNote::Registered(r),
                Err(_) => FaultBufferNote::Malformed { len: params.len() },
            },
            Which::Shadow => match decode_register_client_shadow_fault_buffer(params) {
                Ok(r) => FaultBufferNote::ShadowRegistered(r),
                Err(_) => FaultBufferNote::ShadowMalformed { len: params.len() },
            },
            Which::AccessCntr => match decode_register_access_cntr_buffer(params) {
                Ok(r) => FaultBufferNote::AccessCntrRegistered(r),
                Err(_) => FaultBufferNote::AccessCntrMalformed { len: params.len() },
            },
        });
        // ⊘ Nothing is returned, and nothing CAN be: `observe` has no return value, so
        // "this link changes no reply byte" is rustc's guarantee rather than a comment.
    }
}

kf_util::assert_send_sync!(FaultBufferNote, FaultBufferLog, FaultBufferRecorder);

#[cfg(test)]
mod latest_tests {
    use super::*;

    fn reg(size: u32, page: u64) -> FaultBufferRegistration {
        FaultBufferRegistration {
            h_client: 1,
            h_object: 2,
            size,
            pages: vec![page],
        }
    }

    /// ★ The latest registration survives past the capped sample, and each registration bumps
    /// the generation; malformed ones and the other buffers do not.
    #[test]
    fn the_latest_registration_outlives_the_sample() {
        let log = FaultBufferLog::new();
        assert_eq!((log.generation(), log.latest()), (0, None));
        for i in 0..(FAULT_BUFFER_SAMPLE_MAX as u64 + 3) {
            log.note(FaultBufferNote::Registered(reg(4096, 0x1000 * (i + 1))));
        }
        assert_eq!(log.generation(), FAULT_BUFFER_SAMPLE_MAX as u64 + 3);
        assert_eq!(
            log.latest().map(|r| r.pages[0]),
            Some(0x1000 * (FAULT_BUFFER_SAMPLE_MAX as u64 + 3))
        );
        log.note(FaultBufferNote::Malformed { len: 3 });
        log.note(FaultBufferNote::ShadowMalformed { len: 3 });
        assert_eq!(log.generation(), FAULT_BUFFER_SAMPLE_MAX as u64 + 3);
    }

    /// ★ The listener hears every decoded replayable registration, synchronously, and nothing
    /// else; it can be installed once.
    #[test]
    fn the_listener_hears_each_registration() {
        let log = FaultBufferLog::new();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let h = heard.clone();
        assert!(log.on_registration(Box::new(move |r| h.lock().unwrap().push(r.pages[0]))));
        assert!(!log.on_registration(Box::new(|_| {})), "set once");
        log.note(FaultBufferNote::Registered(reg(4096, 0x5000)));
        log.note(FaultBufferNote::Malformed { len: 3 });
        log.note(FaultBufferNote::Registered(reg(4096, 0x6000)));
        assert_eq!(*heard.lock().unwrap(), vec![0x5000, 0x6000]);
    }
}
