//! GPU-free model of the ogkm deferred-API table (class `0x5080`), plus the conformance checks
//! that every implementation of it must pass.
//!
//! **STATUS: LIVE, 2026-10-07.** Source of the semantics: ogkm 580.95.05
//! `src/nvidia/src/kernel/gpu/deferred_api.c` and `control.c` (privilege gate). Everything in
//! this file is SOURCE-DERIVED (INFERRED for hardware) unless a line says MEASURED; the host
//! driver is 595.91.07, so a hardware run is what upgrades a row to MEASURED.
//!
//! The point of the trait [`DeferredOracle`] is that one set of assertions, [`conformance`], can
//! be run against (1) [`ModelOracle`] here, (2) the native-RM hardware arm
//! (`kf-deferred-oracle`), and (3) kayfabe's Translated handling later. The model is not a
//! replacement for any of them: it can be wrong about the driver, which is why the hardware arm
//! exists.
//!
//! What the model encodes (each item names the C that says so):
//! - register: handle validated as a NEW resource handle (`0`, the client handle, the firmware
//!   range and the client's live handles are refused `INVALID_OBJECT_HANDLE`), a duplicate in the
//!   OBJECT's table is refused `INVALID_OBJECT_HANDLE` (`_Class5080AddDeferredApi`). The inner
//!   `cmd` is NOT checked at registration. The registrant's privilege is recorded in the entry.
//!   (MEASURED on 595.91.07: the handle rules; the `FIFO_UPDATE_CHANNEL_INFO` bundle check.)
//! - remove: `INVALID_DATA`-free: an unknown handle is `NV_ERR_GENERIC` (`_Class5080DelDeferredApi`).
//! - trigger (`0x200`, data = handle): looked up in THIS object's table only; unknown is
//!   `INVALID_DATA`. Control-call commands run `serverControl_Prologue` with the REGISTRANT's
//!   recorded privilege; `DMA_INVALIDATE_TLB` is not a control call and has no privilege gate.
//!   Whatever the status, the entry is marked executed and then deleted if `DELETE_IMPLICIT`
//!   (unless `WAIT_FOR_TLB_FLUSH` is also set, which instead counts it as waiting).
//! - a successful `DMA_INVALIDATE_TLB` completes every executed, waiting entry, in handle order,
//!   deleting those that are `DELETE_IMPLICIT`.

use std::collections::BTreeMap;

// ─── RM status codes (ogkm-580 nvstatuscodes.h) ───
/// `NV_OK`.
pub const NV_OK: u32 = 0;
/// `NV_ERR_INSUFFICIENT_PERMISSIONS`.
pub const NV_ERR_INSUFFICIENT_PERMISSIONS: u32 = 0x1B;
/// `NV_ERR_INVALID_ARGUMENT`.
pub const NV_ERR_INVALID_ARGUMENT: u32 = 0x1F;
/// `NV_ERR_INVALID_DATA`.
pub const NV_ERR_INVALID_DATA: u32 = 0x25;
/// `NV_ERR_INVALID_OBJECT_HANDLE`.
pub const NV_ERR_INVALID_OBJECT_HANDLE: u32 = 0x33;
/// `NV_ERR_GENERIC`.
pub const NV_ERR_GENERIC: u32 = 0xFFFF;

// ─── the eight accepted inner commands (ogkm-580 ctrl2080*.h) ───
/// `NV2080_CTRL_CMD_GPU_INITIALIZE_CTX` (PRIVILEGED).
pub const CMD_GPU_INITIALIZE_CTX: u32 = 0x2080_012d;
/// `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` (PRIVILEGED).
pub const CMD_GPU_PROMOTE_CTX: u32 = 0x2080_012b;
/// `NV2080_CTRL_CMD_GPU_EVICT_CTX` (kernel-only).
pub const CMD_GPU_EVICT_CTX: u32 = 0x2080_012c;
/// `NV2080_CTRL_CMD_FIFO_UPDATE_CHANNEL_INFO` (PRIVILEGED).
pub const CMD_FIFO_UPDATE_CHANNEL_INFO: u32 = 0x2080_1116;
/// `NV2080_CTRL_CMD_DMA_INVALIDATE_TLB` (NON_PRIVILEGED; not a control call in the handler).
pub const CMD_DMA_INVALIDATE_TLB: u32 = 0x2080_2502;
/// `NV2080_CTRL_CMD_GR_CTXSW_ZCULL_BIND` (NON_PRIVILEGED).
pub const CMD_GR_CTXSW_ZCULL_BIND: u32 = 0x2080_1208;
/// `NV2080_CTRL_CMD_GR_CTXSW_PM_BIND` (NON_PRIVILEGED).
pub const CMD_GR_CTXSW_PM_BIND: u32 = 0x2080_1209;
/// `NV2080_CTRL_CMD_GR_CTXSW_PREEMPTION_BIND` (NON_PRIVILEGED).
pub const CMD_GR_CTXSW_PREEMPTION_BIND: u32 = 0x2080_1211;

/// The eight commands, in the order the handler's `switch` lists them.
pub const EIGHT: [u32; 8] = [
    CMD_GPU_INITIALIZE_CTX,
    CMD_GPU_PROMOTE_CTX,
    CMD_GPU_EVICT_CTX,
    CMD_FIFO_UPDATE_CHANNEL_INFO,
    CMD_DMA_INVALIDATE_TLB,
    CMD_GR_CTXSW_ZCULL_BIND,
    CMD_GR_CTXSW_PM_BIND,
    CMD_GR_CTXSW_PREEMPTION_BIND,
];

/// Name of an accepted command, for reports.
#[must_use]
pub fn cmd_name(cmd: u32) -> &'static str {
    match cmd {
        CMD_GPU_INITIALIZE_CTX => "GPU_INITIALIZE_CTX",
        CMD_GPU_PROMOTE_CTX => "GPU_PROMOTE_CTX",
        CMD_GPU_EVICT_CTX => "GPU_EVICT_CTX",
        CMD_FIFO_UPDATE_CHANNEL_INFO => "FIFO_UPDATE_CHANNEL_INFO",
        CMD_DMA_INVALIDATE_TLB => "DMA_INVALIDATE_TLB",
        CMD_GR_CTXSW_ZCULL_BIND => "GR_CTXSW_ZCULL_BIND",
        CMD_GR_CTXSW_PM_BIND => "GR_CTXSW_PM_BIND",
        CMD_GR_CTXSW_PREEMPTION_BIND => "GR_CTXSW_PREEMPTION_BIND",
        _ => "UNKNOWN",
    }
}

/// `DELETE` flag bit 0: `1` is EXPLICIT, `0` is IMPLICIT (`ctrl5080.h`).
pub const FLAGS_DELETE_EXPLICIT: u32 = 0x1;
/// `DELETE_IMPLICIT`.
pub const FLAGS_DELETE_IMPLICIT: u32 = 0x0;
/// `WAIT_FOR_TLB_FLUSH` flag bit 1.
pub const FLAGS_WAIT_FOR_TLB_FLUSH: u32 = 0x2;

/// `RS_PRIV_LEVEL_*` (`nvsecurityinfo.h`), ordered. A root userspace client is `UserRoot`, never
/// `Kernel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Privilege {
    /// `RS_PRIV_LEVEL_USER`.
    User,
    /// `RS_PRIV_LEVEL_USER_ROOT`.
    UserRoot,
    /// `RS_PRIV_LEVEL_KERNEL`.
    Kernel,
}

/// What `serverControl_Prologue` requires of the inner command (`control.c`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// No gate: `RMCTRL_FLAGS_NON_PRIVILEGED`, or a handler that is not a control call.
    Open,
    /// `RMCTRL_FLAGS_PRIVILEGED`: `>= UserRoot`.
    Privileged,
    /// The default for a control with no flag: `>= Kernel`.
    KernelOnly,
}

/// The gate the handler applies at trigger, per command. `None` for a command it does not accept.
#[must_use]
pub fn gate(cmd: u32) -> Option<Gate> {
    Some(match cmd {
        CMD_DMA_INVALIDATE_TLB
        | CMD_GR_CTXSW_ZCULL_BIND
        | CMD_GR_CTXSW_PM_BIND
        | CMD_GR_CTXSW_PREEMPTION_BIND => Gate::Open,
        CMD_GPU_PROMOTE_CTX | CMD_GPU_INITIALIZE_CTX | CMD_FIFO_UPDATE_CHANNEL_INFO => {
            Gate::Privileged
        }
        CMD_GPU_EVICT_CTX => Gate::KernelOnly,
        _ => return None,
    })
}

/// The status the handler reports for a command with LEGAL parameters, run at `who`.
/// Unknown commands: `INVALID_ARGUMENT`. This is the one expectation table the hardware arm and
/// the model share.
#[must_use]
pub fn expected_status(cmd: u32, who: Privilege) -> u32 {
    match gate(cmd) {
        None => NV_ERR_INVALID_ARGUMENT,
        Some(Gate::Open) => NV_OK,
        Some(Gate::Privileged) if who >= Privilege::UserRoot => NV_OK,
        Some(Gate::KernelOnly) if who >= Privilege::Kernel => NV_OK,
        Some(_) => NV_ERR_INSUFFICIENT_PERMISSIONS,
    }
}

/// One entry to register (the bundle's parameters are the implementation's business: it supplies
/// LEGAL ones for the command).
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// `hApiHandle`.
    pub handle: u32,
    /// The inner command.
    pub cmd: u32,
    /// `DELETE_*` and `WAIT_FOR_TLB_FLUSH`.
    pub flags: u32,
}

/// What a trigger told the caller. `status` is `None` where the implementation cannot observe it
/// (on hardware the software method's status is not returned to the CPU).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trigger {
    /// The handler's status where observable.
    pub status: Option<u32>,
}

/// Anything that implements the deferred-API object: the model, the hardware arm, kayfabe.
pub trait DeferredOracle {
    /// The privilege the registrant runs at (recorded at registration).
    fn caller_privilege(&self) -> Privilege;
    /// `NV5080_CTRL_CMD_DEFERRED_API_V2`. `Err` carries the RM status.
    ///
    /// # Errors
    /// The RM status.
    fn register(&mut self, e: &Entry) -> Result<(), u32>;
    /// `NV5080_CTRL_CMD_REMOVE_API`. `Err` carries the RM status.
    ///
    /// # Errors
    /// The RM status.
    fn remove(&mut self, handle: u32) -> Result<(), u32>;
    /// Software method `0x200` with `data = handle`, executed and drained.
    ///
    /// # Errors
    /// The implementation could not run the trigger at all.
    fn trigger(&mut self, handle: u32) -> Result<Trigger, String>;
    /// Whether `handle` is still in the table. On hardware this is the non-destructive
    /// re-register probe (`INVALID_OBJECT_HANDLE` means present).
    ///
    /// # Errors
    /// The probe could not run.
    fn is_present(&mut self, handle: u32) -> Result<bool, String>;
    /// Handles a registration must refuse (0, the client handle, the reserved range, live handles).
    fn invalid_handles(&self) -> Vec<u32>;
    /// A handle never used before.
    fn fresh_handle(&mut self) -> u32;
}

// ─── the model ───────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
struct Slot {
    cmd: u32,
    flags: u32,
    privilege: Privilege,
    executed: bool,
    flushed: bool,
}

impl Slot {
    fn waits(&self) -> bool {
        self.flags & FLAGS_WAIT_FOR_TLB_FLUSH != 0
    }
    fn implicit(&self) -> bool {
        self.flags & FLAGS_DELETE_EXPLICIT == 0
    }
}

/// A side effect the model performed (what the real handler would have done to the GPU).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// `vaspaceInvalidateTlb`.
    TlbInvalidated,
    /// An inner control ran past the privilege gate.
    Control(u32),
}

/// The firmware-reserved handle range measured on 595.91.07 (`F7a`), inclusive.
pub const FW_RESERVED: (u32, u32) = (0xc9f0_0000, 0xc9f7_ffff);

/// One 5080 object's table: ogkm's `DeferredApiObject`.
#[derive(Debug, Default)]
pub struct DeferredObject {
    list: BTreeMap<u32, Slot>,
    num_waiting: u32,
    /// Everything executed, in order.
    pub effects: Vec<Effect>,
}

/// The client-side facts registration depends on.
#[derive(Clone, Debug)]
pub struct Env {
    /// The client's own handle.
    pub client: u32,
    /// The client's other live resource handles.
    pub live: Vec<u32>,
    /// `IS_GSP_CLIENT`: `FIFO_UPDATE_CHANNEL_INFO` registration validates its bundle.
    pub gsp_client: bool,
    /// Whether the bundle of a `FIFO_UPDATE_CHANNEL_INFO` names a real client and USERD.
    pub update_bundle_ok: bool,
    /// Whether `DMA_INVALIDATE_TLB`'s `hClientVA`/`hDeviceVA`/`hVASpace` resolve.
    pub va_ok: bool,
    /// Whether a control's own parameter validation accepts the bundle (legal parameters).
    pub inner_ok: bool,
}

impl Default for Env {
    fn default() -> Env {
        Env {
            client: 0xcafe_0001,
            live: vec![0xcafe_0004, 0xcafe_0007, 0xcafe_0008, 0xcafe_000a],
            gsp_client: true,
            update_bundle_ok: true,
            va_ok: true,
            inner_ok: true,
        }
    }
}

impl DeferredObject {
    /// An empty object.
    #[must_use]
    pub fn new() -> DeferredObject {
        DeferredObject::default()
    }

    /// `NumWaitingOnTLBFlush` (a `u32` that wraps, as in the C).
    #[must_use]
    pub fn num_waiting(&self) -> u32 {
        self.num_waiting
    }

    /// Whether the table holds `handle`.
    #[must_use]
    pub fn contains(&self, handle: u32) -> bool {
        self.list.contains_key(&handle)
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    fn new_handle_ok(env: &Env, h: u32) -> bool {
        h != 0
            && h != env.client
            && !(FW_RESERVED.0..=FW_RESERVED.1).contains(&h)
            && !env.live.contains(&h)
    }

    /// `NV5080_CTRL_CMD_DEFERRED_API_V2` from a caller at `who`.
    ///
    /// # Errors
    /// `INVALID_OBJECT_HANDLE` for a bad, live or duplicate handle, or for a
    /// `FIFO_UPDATE_CHANNEL_INFO` whose bundle names no real client/USERD on a GSP client.
    pub fn register(&mut self, env: &Env, e: &Entry, who: Privilege) -> Result<(), u32> {
        if e.cmd == CMD_FIFO_UPDATE_CHANNEL_INFO && env.gsp_client && !env.update_bundle_ok {
            return Err(NV_ERR_INVALID_OBJECT_HANDLE);
        }
        if !Self::new_handle_ok(env, e.handle) || self.list.contains_key(&e.handle) {
            return Err(NV_ERR_INVALID_OBJECT_HANDLE);
        }
        self.list.insert(
            e.handle,
            Slot {
                cmd: e.cmd,
                flags: e.flags,
                privilege: who,
                executed: false,
                flushed: false,
            },
        );
        Ok(())
    }

    /// `NV5080_CTRL_CMD_REMOVE_API`. Faithful to `_Class5080DelDeferredApi`, including that the
    /// waiting-count decrement does not look at the DELETE flag.
    ///
    /// # Errors
    /// `NV_ERR_GENERIC` for an unknown handle.
    pub fn remove(&mut self, handle: u32) -> Result<(), u32> {
        let Some(s) = self.list.remove(&handle) else {
            return Err(NV_ERR_GENERIC);
        };
        if s.waits() && s.executed && !s.flushed {
            self.num_waiting = self.num_waiting.wrapping_sub(1);
        }
        Ok(())
    }

    fn update_tlb_flush_state(&mut self) {
        let keys: Vec<u32> = self.list.keys().copied().collect();
        for k in keys {
            if self.num_waiting == 0 {
                break;
            }
            let Some(s) = self.list.get_mut(&k) else {
                continue;
            };
            if s.waits() && s.executed && !s.flushed {
                s.flushed = true;
                self.num_waiting = self.num_waiting.wrapping_sub(1);
                if s.implicit() {
                    self.list.remove(&k);
                }
            }
        }
    }

    /// Software method `0x200`, `data = handle`. Returns the handler's status.
    pub fn trigger(&mut self, env: &Env, data: u32) -> u32 {
        let Some(s) = self.list.get(&data).copied() else {
            return NV_ERR_INVALID_DATA;
        };
        let status = match s.cmd {
            CMD_DMA_INVALIDATE_TLB => {
                if env.va_ok {
                    self.effects.push(Effect::TlbInvalidated);
                    if self.num_waiting != 0 {
                        self.update_tlb_flush_state();
                    }
                    NV_OK
                } else {
                    NV_ERR_INVALID_OBJECT_HANDLE
                }
            }
            c if gate(c).is_some() => {
                let st = expected_status(c, s.privilege);
                if st != NV_OK {
                    st
                } else if env.inner_ok {
                    self.effects.push(Effect::Control(c));
                    NV_OK
                } else {
                    NV_ERR_INVALID_ARGUMENT
                }
            }
            _ => NV_ERR_INVALID_ARGUMENT,
        };
        // `cleanup:` runs whatever the status was. The entry may already be gone (a TLB
        // invalidate that completed itself cannot: it is not yet marked executed).
        if let Some(e) = self.list.get_mut(&data) {
            e.executed = true;
            if s.implicit() {
                if s.waits() {
                    self.num_waiting = self.num_waiting.wrapping_add(1);
                } else {
                    self.list.remove(&data);
                }
            }
        }
        status
    }
}

/// [`DeferredOracle`] over one [`DeferredObject`], for a caller at a fixed privilege.
pub struct ModelOracle {
    /// The object under test.
    pub obj: DeferredObject,
    /// Registration facts.
    pub env: Env,
    /// The privilege entries are registered at (changeable between register and trigger to show
    /// that the REGISTRANT's privilege, not the trigger's, is what runs).
    pub who: Privilege,
    next: u32,
}

impl ModelOracle {
    /// A fresh object for a caller at `who`.
    #[must_use]
    pub fn new(who: Privilege) -> ModelOracle {
        ModelOracle {
            obj: DeferredObject::new(),
            env: Env::default(),
            who,
            next: 0x5151_0000,
        }
    }
}

impl DeferredOracle for ModelOracle {
    fn caller_privilege(&self) -> Privilege {
        self.who
    }
    fn register(&mut self, e: &Entry) -> Result<(), u32> {
        self.obj.register(&self.env, e, self.who)
    }
    fn remove(&mut self, handle: u32) -> Result<(), u32> {
        self.obj.remove(handle)
    }
    fn trigger(&mut self, handle: u32) -> Result<Trigger, String> {
        Ok(Trigger {
            status: Some(self.obj.trigger(&self.env, handle)),
        })
    }
    fn is_present(&mut self, handle: u32) -> Result<bool, String> {
        Ok(self.obj.contains(handle))
    }
    fn invalid_handles(&self) -> Vec<u32> {
        let mut v = vec![0, self.env.client, FW_RESERVED.0, FW_RESERVED.1];
        v.extend(self.env.live.iter().copied());
        v
    }
    fn fresh_handle(&mut self) -> u32 {
        self.next += 1;
        self.next
    }
}

// ─── the shared conformance checks ────────────────────────────────────────────────────────────

/// One named assertion's result.
#[derive(Debug)]
pub struct Verdict {
    /// Check name (stable: the hardware arm prints the same names).
    pub name: String,
    /// Whether it held.
    pub pass: bool,
    /// Why, in one line.
    pub why: String,
}

fn put(out: &mut Vec<Verdict>, name: impl Into<String>, pass: bool, why: impl Into<String>) {
    out.push(Verdict {
        name: name.into(),
        pass,
        why: why.into(),
    });
}

fn st(r: &Result<(), u32>) -> String {
    match r {
        Ok(()) => "OK".into(),
        Err(c) => format!("{c:#x}"),
    }
}

/// The assertions every implementation must satisfy. Requires an empty table. Every check is on
/// observable behaviour (statuses, presence); trigger statuses are only asserted where the
/// implementation reports one. LEGAL parameters for each command are the implementation's job.
///
/// # Errors
/// An infrastructure failure (a trigger could not run, a probe failed), not a failed assertion.
pub fn conformance<T: DeferredOracle + ?Sized>(t: &mut T) -> Result<Vec<Verdict>, String> {
    let mut out = Vec::new();
    let who = t.caller_privilege();
    let e = |h: u32, cmd: u32, flags: u32| Entry {
        handle: h,
        cmd,
        flags,
    };

    // register / duplicate / remove / re-register
    let h = t.fresh_handle();
    let r = t.register(&e(h, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_EXPLICIT));
    put(&mut out, "register_ok", r.is_ok(), st(&r));
    let r = t.register(&e(h, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_EXPLICIT));
    put(
        &mut out,
        "duplicate_refused_0x33",
        r == Err(NV_ERR_INVALID_OBJECT_HANDLE),
        st(&r),
    );
    let r = t.remove(h);
    put(&mut out, "remove_ok", r.is_ok(), st(&r));
    let r = t.remove(h);
    put(
        &mut out,
        "remove_unknown_generic",
        r == Err(NV_ERR_GENERIC),
        st(&r),
    );
    let r = t.register(&e(h, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_EXPLICIT));
    put(&mut out, "reregister_after_remove", r.is_ok(), st(&r));
    let _ = t.remove(h);

    // handles a registration must refuse
    for bad in t.invalid_handles() {
        let r = t.register(&e(bad, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_EXPLICIT));
        put(
            &mut out,
            format!("invalid_handle_{bad:#x}_refused"),
            r == Err(NV_ERR_INVALID_OBJECT_HANDLE),
            st(&r),
        );
        if r.is_ok() {
            let _ = t.remove(bad);
        }
    }

    // trigger of an unknown handle: INVALID_DATA, table unchanged
    let ghost = t.fresh_handle();
    let tr = t.trigger(ghost)?;
    put(
        &mut out,
        "trigger_unknown_invalid_data",
        tr.status.is_none_or(|s| s == NV_ERR_INVALID_DATA),
        format!("status={:?}", tr.status),
    );
    let present = t.is_present(ghost)?;
    put(&mut out, "trigger_unknown_no_entry_created", !present, "");
    if present {
        let _ = t.remove(ghost);
    }

    // implicit delete consumes; explicit keeps and can fire again
    let hi = t.fresh_handle();
    t.register(&e(hi, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_IMPLICIT))
        .map_err(|c| format!("register implicit: {c:#x}"))?;
    let tr = t.trigger(hi)?;
    put(
        &mut out,
        "implicit_trigger_status_ok",
        tr.status.is_none_or(|s| s == NV_OK),
        format!("status={:?}", tr.status),
    );
    let gone = !t.is_present(hi)?;
    put(&mut out, "implicit_entry_consumed", gone, "");
    let r = t.register(&e(hi, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_IMPLICIT));
    put(&mut out, "implicit_reregister_succeeds", r.is_ok(), st(&r));
    let _ = t.remove(hi);

    let he = t.fresh_handle();
    t.register(&e(he, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_EXPLICIT))
        .map_err(|c| format!("register explicit: {c:#x}"))?;
    for round in 0..2 {
        let tr = t.trigger(he)?;
        let kept = t.is_present(he)?;
        put(
            &mut out,
            format!("explicit_entry_kept_round{round}"),
            kept && tr.status.is_none_or(|s| s == NV_OK),
            format!("status={:?} present={kept}", tr.status),
        );
    }
    let _ = t.remove(he);

    // WAIT_FOR_TLB_FLUSH: executed entries stay until a later successful invalidate
    let hw = t.fresh_handle();
    let hf = t.fresh_handle();
    t.register(&e(
        hw,
        CMD_GR_CTXSW_PM_BIND,
        FLAGS_DELETE_IMPLICIT | FLAGS_WAIT_FOR_TLB_FLUSH,
    ))
    .map_err(|c| format!("register wait: {c:#x}"))?;
    t.register(&e(hf, CMD_DMA_INVALIDATE_TLB, FLAGS_DELETE_IMPLICIT))
        .map_err(|c| format!("register flusher: {c:#x}"))?;
    t.trigger(hw)?;
    let kept = t.is_present(hw)?;
    put(&mut out, "wait_for_flush_entry_kept_after_execute", kept, "");
    t.trigger(hf)?;
    let gone = !t.is_present(hw)?;
    put(&mut out, "wait_for_flush_entry_freed_by_next_invalidate", gone, "");
    let _ = t.remove(hw);
    let _ = t.remove(hf);

    // the eight commands and one unknown, implicit delete, at the oracle's privilege
    for cmd in EIGHT.iter().copied().chain([0x2080_dead]) {
        let h = t.fresh_handle();
        let name = format!("cmd_{}_as_{who:?}", cmd_name(cmd));
        let r = t.register(&e(h, cmd, FLAGS_DELETE_IMPLICIT));
        put(&mut out, format!("{name}_registers"), r.is_ok(), st(&r));
        if r.is_err() {
            continue;
        }
        let tr = t.trigger(h)?;
        let want = expected_status(cmd, who);
        put(
            &mut out,
            format!("{name}_status"),
            tr.status.is_none_or(|s| s == want),
            format!("got {:?} want {want:#x}", tr.status),
        );
        let consumed = !t.is_present(h)?;
        put(&mut out, format!("{name}_entry_consumed"), consumed, "");
        if !consumed {
            let _ = t.remove(h);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(who: Privilege) -> Vec<Verdict> {
        let mut m = ModelOracle::new(who);
        conformance(&mut m).expect("model never fails to run")
    }

    #[test]
    fn model_passes_conformance_at_every_privilege() {
        for who in [Privilege::User, Privilege::UserRoot, Privilege::Kernel] {
            let v = run(who);
            assert!(v.len() > 40, "{who:?}: only {} checks", v.len());
            let bad: Vec<_> = v.iter().filter(|x| !x.pass).collect();
            assert!(bad.is_empty(), "{who:?}: {bad:?}");
        }
    }

    #[test]
    fn privilege_table_matches_the_owner_ruling() {
        use Privilege::{Kernel, User, UserRoot};
        for c in [
            CMD_DMA_INVALIDATE_TLB,
            CMD_GR_CTXSW_ZCULL_BIND,
            CMD_GR_CTXSW_PM_BIND,
            CMD_GR_CTXSW_PREEMPTION_BIND,
        ] {
            for w in [User, UserRoot, Kernel] {
                assert_eq!(expected_status(c, w), NV_OK, "{} {w:?}", cmd_name(c));
            }
        }
        for c in [
            CMD_GPU_PROMOTE_CTX,
            CMD_GPU_INITIALIZE_CTX,
            CMD_FIFO_UPDATE_CHANNEL_INFO,
        ] {
            assert_eq!(expected_status(c, User), NV_ERR_INSUFFICIENT_PERMISSIONS);
            assert_eq!(expected_status(c, UserRoot), NV_OK);
            assert_eq!(expected_status(c, Kernel), NV_OK);
        }
        assert_eq!(
            expected_status(CMD_GPU_EVICT_CTX, User),
            NV_ERR_INSUFFICIENT_PERMISSIONS
        );
        // A root userspace client is PRIVILEGED, not KERNEL.
        assert_eq!(
            expected_status(CMD_GPU_EVICT_CTX, UserRoot),
            NV_ERR_INSUFFICIENT_PERMISSIONS
        );
        assert_eq!(expected_status(CMD_GPU_EVICT_CTX, Kernel), NV_OK);
    }

    #[test]
    fn privilege_is_recorded_at_registration_not_trigger() {
        let mut m = ModelOracle::new(Privilege::User);
        let h = m.fresh_handle();
        m.register(&Entry {
            handle: h,
            cmd: CMD_GPU_PROMOTE_CTX,
            flags: FLAGS_DELETE_EXPLICIT,
        })
        .unwrap();
        m.who = Privilege::Kernel; // a more privileged trigger must not upgrade the entry
        assert_eq!(m.trigger(h).unwrap().status, Some(NV_ERR_INSUFFICIENT_PERMISSIONS));
        assert!(m.obj.effects.is_empty(), "a refused control has no effect");
        // and the reverse: registered privileged, triggered from a "user" caller, still runs.
        m.who = Privilege::UserRoot;
        let h2 = m.fresh_handle();
        m.register(&Entry {
            handle: h2,
            cmd: CMD_GPU_PROMOTE_CTX,
            flags: FLAGS_DELETE_IMPLICIT,
        })
        .unwrap();
        m.who = Privilege::User;
        assert_eq!(m.trigger(h2).unwrap().status, Some(NV_OK));
        assert_eq!(m.obj.effects, vec![Effect::Control(CMD_GPU_PROMOTE_CTX)]);
    }

    #[test]
    fn a_refused_privileged_entry_is_still_consumed() {
        let mut m = ModelOracle::new(Privilege::User);
        let h = m.fresh_handle();
        m.register(&Entry {
            handle: h,
            cmd: CMD_GPU_INITIALIZE_CTX,
            flags: FLAGS_DELETE_IMPLICIT,
        })
        .unwrap();
        assert_eq!(m.trigger(h).unwrap().status, Some(NV_ERR_INSUFFICIENT_PERMISSIONS));
        assert!(!m.obj.contains(h), "cleanup runs whatever the status was");
        assert!(m.obj.effects.is_empty());
    }

    #[test]
    fn entries_are_per_object() {
        // F1/F2/F3 in the model: a handle on A is invisible to B's trigger.
        let env = Env::default();
        let (mut a, mut b) = (DeferredObject::new(), DeferredObject::new());
        let ent = Entry {
            handle: 0x5151_1001,
            cmd: CMD_DMA_INVALIDATE_TLB,
            flags: FLAGS_DELETE_IMPLICIT,
        };
        a.register(&env, &ent, Privilege::User).unwrap();
        assert_eq!(b.trigger(&env, ent.handle), NV_ERR_INVALID_DATA);
        assert!(a.contains(ent.handle) && b.effects.is_empty());
        // The same handle number registers independently on B.
        b.register(&env, &ent, Privilege::User).unwrap();
    }

    #[test]
    fn wait_for_tlb_flush_counting() {
        let env = Env::default();
        let mut o = DeferredObject::new();
        let w = |h, extra| Entry {
            handle: h,
            cmd: CMD_GR_CTXSW_PM_BIND,
            flags: FLAGS_WAIT_FOR_TLB_FLUSH | extra,
        };
        let flush = |h| Entry {
            handle: h,
            cmd: CMD_DMA_INVALIDATE_TLB,
            flags: FLAGS_DELETE_IMPLICIT,
        };
        // Two implicit waiters and one explicit waiter, plus a flusher.
        o.register(&env, &w(0x101, FLAGS_DELETE_IMPLICIT), Privilege::User).unwrap();
        o.register(&env, &w(0x102, FLAGS_DELETE_IMPLICIT), Privilege::User).unwrap();
        o.register(&env, &w(0x103, FLAGS_DELETE_EXPLICIT), Privilege::User).unwrap();
        o.register(&env, &flush(0x200), Privilege::User).unwrap();
        for h in [0x101, 0x102, 0x103] {
            assert_eq!(o.trigger(&env, h), NV_OK);
        }
        // Only the implicit waiters are counted (the C increments only in the implicit branch).
        assert_eq!(o.num_waiting(), 2);
        assert!(o.contains(0x101) && o.contains(0x102) && o.contains(0x103));
        assert_eq!(o.trigger(&env, 0x200), NV_OK);
        assert_eq!(o.num_waiting(), 0);
        assert!(!o.contains(0x101) && !o.contains(0x102), "implicit waiters freed");
        assert!(o.contains(0x103), "the explicit waiter is flushed but kept");
        // The flusher itself was implicit, not waiting: gone.
        assert!(!o.contains(0x200));
    }

    #[test]
    fn remove_of_an_unflushed_waiter_decrements() {
        let env = Env::default();
        let mut o = DeferredObject::new();
        o.register(
            &env,
            &Entry {
                handle: 0x301,
                cmd: CMD_GR_CTXSW_PM_BIND,
                flags: FLAGS_WAIT_FOR_TLB_FLUSH | FLAGS_DELETE_IMPLICIT,
            },
            Privilege::User,
        )
        .unwrap();
        o.trigger(&env, 0x301);
        assert_eq!(o.num_waiting(), 1);
        o.remove(0x301).unwrap();
        assert_eq!(o.num_waiting(), 0);
    }

    /// SOURCE-DERIVED QUIRK, not hardware-measured: an EXPLICIT+WAIT entry is never counted on
    /// execute, but `remove` decrements for it (`_Class5080DelDeferredApi` ignores the DELETE
    /// flag), so the counter wraps. Pinned here so a faithful reimplementation is not "fixed" by
    /// accident; kayfabe's handling should not copy the wrap.
    #[test]
    fn quirk_explicit_wait_remove_wraps_the_counter() {
        let env = Env::default();
        let mut o = DeferredObject::new();
        o.register(
            &env,
            &Entry {
                handle: 0x401,
                cmd: CMD_GR_CTXSW_PM_BIND,
                flags: FLAGS_WAIT_FOR_TLB_FLUSH | FLAGS_DELETE_EXPLICIT,
            },
            Privilege::User,
        )
        .unwrap();
        o.trigger(&env, 0x401);
        assert_eq!(o.num_waiting(), 0);
        o.remove(0x401).unwrap();
        assert_eq!(o.num_waiting(), u32::MAX);
    }

    #[test]
    fn registration_rules() {
        let env = Env::default();
        let mut o = DeferredObject::new();
        let ent = |h, cmd| Entry {
            handle: h,
            cmd,
            flags: FLAGS_DELETE_EXPLICIT,
        };
        for bad in [0, env.client, FW_RESERVED.0, FW_RESERVED.1, 0xcafe_0008] {
            assert_eq!(
                o.register(&env, &ent(bad, CMD_DMA_INVALIDATE_TLB), Privilege::User),
                Err(NV_ERR_INVALID_OBJECT_HANDLE),
                "{bad:#x}"
            );
        }
        for ok in [1, 0x4000_0000, 0xffff_ffff, 0xcafe_0000, 0xcaf0_0000] {
            assert_eq!(o.register(&env, &ent(ok, CMD_DMA_INVALIDATE_TLB), Privilege::User), Ok(()));
        }
        // The inner command is not checked at registration (measured: PROMOTE/INIT/EVICT register
        // as a non-root user), including an unknown one.
        for (i, c) in [CMD_GPU_PROMOTE_CTX, CMD_GPU_INITIALIZE_CTX, CMD_GPU_EVICT_CTX, 0x2080_dead]
            .into_iter()
            .enumerate()
        {
            assert_eq!(o.register(&env, &ent(0x600 + i as u32, c), Privilege::User), Ok(()));
        }
        // MEASURED: FIFO_UPDATE_CHANNEL_INFO with a bundle naming no real client is refused 0x33.
        let bad_bundle = Env {
            update_bundle_ok: false,
            ..Env::default()
        };
        assert_eq!(
            o.register(&bad_bundle, &ent(0x700, CMD_FIFO_UPDATE_CHANNEL_INFO), Privilege::User),
            Err(NV_ERR_INVALID_OBJECT_HANDLE)
        );
    }

    #[test]
    fn unknown_command_is_invalid_argument_and_consumed() {
        let mut m = ModelOracle::new(Privilege::UserRoot);
        let h = m.fresh_handle();
        m.register(&Entry {
            handle: h,
            cmd: 0x2080_dead,
            flags: FLAGS_DELETE_IMPLICIT,
        })
        .unwrap();
        assert_eq!(m.trigger(h).unwrap().status, Some(NV_ERR_INVALID_ARGUMENT));
        assert!(!m.obj.contains(h));
    }

    #[test]
    fn a_failed_tlb_invalidate_frees_no_waiter() {
        let mut env = Env::default();
        let mut o = DeferredObject::new();
        o.register(
            &env,
            &Entry {
                handle: 0x801,
                cmd: CMD_GR_CTXSW_PM_BIND,
                flags: FLAGS_WAIT_FOR_TLB_FLUSH | FLAGS_DELETE_IMPLICIT,
            },
            Privilege::User,
        )
        .unwrap();
        o.register(
            &env,
            &Entry {
                handle: 0x802,
                cmd: CMD_DMA_INVALIDATE_TLB,
                flags: FLAGS_DELETE_IMPLICIT,
            },
            Privilege::User,
        )
        .unwrap();
        o.trigger(&env, 0x801);
        env.va_ok = false; // the VA space does not resolve
        assert_eq!(o.trigger(&env, 0x802), NV_ERR_INVALID_OBJECT_HANDLE);
        assert!(o.contains(0x801) && o.num_waiting() == 1);
        assert!(o.effects.iter().all(|e| *e != Effect::TlbInvalidated));
    }
}
