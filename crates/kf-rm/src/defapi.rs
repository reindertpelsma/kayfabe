//! ★ OWNER_RULINGS §U (2026-10-07) — **the per-object deferred-API entry tables**: what the GSP's
//! `DeferredApiObject` keeps (`ogkm-580.95.05: src/nvidia/src/kernel/gpu/deferred_api.c`), held by
//! the object graph's seat and read by the channel plane at the trigger.
//!
//! | real RM | here |
//! |---|---|
//! | `_Class5080AddDeferredApi` (`:60-116`): validate the handle as a new resource handle of the OBJECT's client, refuse a duplicate on the object, copy the params, record the caller's `privLevel` | [`Registry::register`] — same checks, same order, same statuses; the cmd is NOT checked |
//! | `_Class5080DelDeferredApi` (`:140-182`): unlink; a WAIT_FOR_TLB_FLUSH entry that executed and never saw a flush decrements `NumWaitingOnTLBFlush`; `NV_ERR_GENERIC` if absent | [`Registry::remove`] |
//! | `_class5080DeferredApiV2` (`:443-674`): look `Data` up in THIS object's tree (`NV_ERR_INVALID_DATA` if absent), run the control, then — whatever its status — mark executed and delete implicitly unless EXPLICIT or WAIT_FOR_TLB_FLUSH (then count it waiting) | [`Registry::lookup`] at the trigger, [`Registry::executed_entry`] when the host-authored action has finished |
//! | `_Class5080UpdateTLBFlushState` (`:185-227`), run by a successful deferred `DMA_INVALIDATE_TLB` BEFORE its own entry is marked | [`Registry::executed_entry`] with `tlb_flushed = true` |
//! | `defapiDestruct_IMPL` (`:257-278`): every entry freed with the object | [`Registry::prune`] after every graph free |
//!
//! ⊘ **The tables never execute anything.** Execution is the channel plane's, on Translated
//! channels only (§U.1), by host-authored equivalents; an entry carries the guest's bundle as
//! opaque, bounded bytes and nothing here forwards a byte of it.
//!
//! **Bounds (kayfabe's, not RM's):** the real tree grows until `portMemAllocNonPaged` fails
//! (`NV_ERR_NO_MEMORY`, `:112-113`); a hostile guest could otherwise hold unbounded host memory.
//! [`Bounds`] caps entries per object, per client and per VM and answers the same
//! `NV_ERR_NO_MEMORY` at the cap. `[measured: the 2026-10-05 VFIO boots 8/9/10]` Windows 580.88 holds at most 2 live
//! entries on its one registering object (implicit deletes), 8 registrations per boot.
//!
//! **Privilege (inferred, not measured):** the real GSP records the RPC's privilege for every
//! registration; a registration reaches the GSP only from the guest's CPU-RM (the controls are
//! ROUTE_TO_PHYSICAL), so the recorded level is the guest kernel's ([`Registrant`]). kayfabe's
//! dispatch never grants anything on that basis: what runs is decided by the plane's own rules.

use kf_abi::defapi::{
    Bundle, Control, DefApiAbi, Flags, Form, HandleRules, NV_ERR_GENERIC, NV_ERR_INVALID_DATA,
    NV_ERR_INVALID_OBJECT_HANDLE, NV_ERR_NO_MEMORY, NV_OK, Registration,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

/// A 5080 object, by the guest's names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjKey {
    /// `hClient`.
    pub client: u32,
    /// The object's handle.
    pub object: u32,
}

/// Who registered an entry, as the GSP would record it (`privLevel`, `deferred_api.c:100`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registrant {
    /// The guest's CPU-RM, through `GSP_RM_CONTROL` (the only route: ROUTE_TO_PHYSICAL).
    GuestKernelRpc,
}

/// One registered entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    // Retained by action snapshots; numeric handle and identical bytes are not
    // allocation identity. Compare this only with Arc::ptr_eq.
    allocation: Arc<()>,
    /// `hApiHandle`.
    pub handle: u32,
    /// The control that carried it.
    pub form: Form,
    /// The command the trigger will dispatch.
    pub cmd: u32,
    /// The flags.
    pub flags: Flags,
    /// `hClientVA` (ignored by the trigger, §U.2).
    pub client_va: u32,
    /// `hDeviceVA` (ignored by the trigger, §U.2).
    pub device_va: u32,
    /// The bundle, exactly the union's bytes.
    pub bundle: Vec<u8>,
    /// The bundle decoded at the registering guest's own layout (registration never refuses on
    /// it: the trigger decides).
    pub decoded: Bundle,
    /// The recorded privilege.
    pub registrant: Registrant,
    /// `DEFERRED_API_INFO_FLAGS_HAS_EXECUTED`.
    pub executed: bool,
    /// `DEFERRED_API_INFO_FLAGS_HAS_TLB_FLUSHED`.
    pub tlb_flushed: bool,
}

/// Kayfabe's bounds on what one guest may hold (see the module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    /// Live entries on one object.
    pub per_object: usize,
    /// Live entries over one client's objects.
    pub per_client: usize,
    /// Live entries in the VM.
    pub per_vm: usize,
}

impl Default for Bounds {
    /// 64 per object (32× the peak of 2 in the 2026-10-05 VFIO boots), 256 per client, 1024 per VM: at most
    /// ~600 KiB of bundles (584-byte params each) whatever the guest does.
    fn default() -> Self {
        Self {
            per_object: 64,
            per_client: 256,
            per_vm: 1024,
        }
    }
}

/// One object's tree.
#[derive(Debug, Default, Clone)]
struct Table {
    entries: BTreeMap<u32, Entry>,
    /// `NumWaitingOnTLBFlush`.
    waiting_tlb: u32,
}

impl Table {
    /// `_Class5080DelDeferredApi`: `true` when an entry was removed.
    fn delete(&mut self, handle: u32) -> bool {
        let Some(e) = self.entries.remove(&handle) else {
            return false;
        };
        if e.flags.wait_tlb_flush && e.executed && !e.tlb_flushed {
            self.waiting_tlb = self.waiting_tlb.saturating_sub(1);
        }
        true
    }

    /// `_Class5080UpdateTLBFlushState`: in handle order while anything waits, every executed
    /// WAIT_FOR_TLB_FLUSH entry not yet flushed is marked flushed and, if implicit, deleted.
    fn tlb_flushed(&mut self) {
        let handles: Vec<u32> = self.entries.keys().copied().collect();
        for h in handles {
            if self.waiting_tlb == 0 {
                break;
            }
            let Some(e) = self.entries.get_mut(&h) else {
                continue;
            };
            if e.flags.wait_tlb_flush && e.executed && !e.tlb_flushed {
                e.tlb_flushed = true;
                self.waiting_tlb -= 1;
                if !e.flags.explicit_delete {
                    // Flushed first, so the delete does not decrement again (`:168-172`).
                    self.entries.remove(&h);
                }
            }
        }
    }
}

/// Counters, for the log.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// Registrations accepted.
    pub registered: u64,
    /// Registrations refused (any status).
    pub refused: u64,
    /// Deliberate FIFO-update registration refusals in the default-off fallback experiment.
    pub fifo_update_refused: u64,
    /// `_REMOVE_API`s that removed an entry.
    pub removed: u64,
    /// Triggers that found their entry.
    pub looked_up: u64,
    /// Triggers that named no entry (`NV_ERR_INVALID_DATA`).
    pub unknown: u64,
    /// Entries deleted implicitly after execution.
    pub implicit_deletes: u64,
    /// Entries freed with their object.
    pub destroyed: u64,
    /// Deferred completions whose exact registration was removed or replaced.
    pub stale_completions: u64,
}

#[derive(Debug, Default)]
struct State {
    objects: BTreeMap<ObjKey, Table>,
    per_client: HashMap<u32, usize>,
    total: usize,
    bounds: Bounds,
    stats: Stats,
}

impl State {
    fn count_down(&mut self, client: u32, n: usize) {
        self.total = self.total.saturating_sub(n);
        if let Some(c) = self.per_client.get_mut(&client) {
            *c = c.saturating_sub(n);
            if *c == 0 {
                self.per_client.remove(&client);
            }
        }
    }

    /// Returns the number of entries deleted.
    fn executed(&mut self, key: ObjKey, handle: u32, tlb_flushed: bool) -> usize {
        let mut deleted = 0;
        if let Some(t) = self.objects.get_mut(&key) {
            let before = t.entries.len();
            if tlb_flushed && t.waiting_tlb > 0 {
                t.tlb_flushed();
            }
            if let Some(e) = t.entries.get_mut(&handle) {
                e.executed = true;
                if !e.flags.explicit_delete {
                    if e.flags.wait_tlb_flush {
                        t.waiting_tlb += 1;
                    } else {
                        t.entries.remove(&handle);
                    }
                }
            }
            deleted = before - t.entries.len();
        }
        if deleted > 0 {
            self.count_down(key.client, deleted);
            self.stats.implicit_deletes += deleted as u64;
        }
        deleted
    }

    fn live(&self, key: ObjKey) -> usize {
        self.objects.get(&key).map_or(0, |t| t.entries.len())
    }
}

/// ★ Every 5080 object's tree in the VM — one per device, shared by the object seat (which
/// registers and removes) and the channel plane (which looks up at the trigger and records the
/// execution). One mutex, held for O(log n) map work; never taken by a vCPU.
#[derive(Debug, Default)]
pub struct Registry {
    inner: Mutex<State>,
    /// ★ 2026-10-16 (`V3_USER_RING_LIFECYCLE_20261016.md` §1.1): bumped (release) AFTER every change of the
    /// set of live entries, while the lock that made the change is still held, and so BEFORE the
    /// registration's reply is built. A worker that loads it (acquire) after a guest GP entry and sees a
    /// new value finds the change in [`Registry::live_count`].
    epoch: std::sync::atomic::AtomicU64,
}

impl Registry {
    /// An empty registry with `bounds`.
    #[must_use]
    pub fn new(bounds: Bounds) -> Registry {
        Registry {
            inner: Mutex::new(State {
                bounds,
                ..State::default()
            }),
            epoch: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// ★ Device reset (`docs/design/V3_DEVICE_RESET.md`): every 5080 object's table goes — they
    /// were the previous guest life's, and a handle the next life reuses must find nothing (an old
    /// registration names guest memory of a life that is gone). Bounds and the counters stay.
    /// Returns the entries dropped.
    pub fn clear_life(&self) -> usize {
        let mut st = self.st();
        let n = st.total;
        st.objects.clear();
        st.per_client.clear();
        st.total = 0;
        if n > 0 {
            self.bump();
        }
        n
    }

    /// The epoch moved: the set of live entries changed. Called with the state lock held.
    fn bump(&self) {
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }

    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        // A poisoned registry is a bug in this module; its state stays consistent per call.
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// ★ `_Class5080AddDeferredApi` on the 5080 object `key` (the caller has checked that `key` is
    /// a live 5080 object in the graph). `live_handle(h)`: `h` names a live resource of
    /// `key.client` (`clientGetResourceRef`). Returns the status the GSP answers.
    pub fn register(
        &self,
        key: ObjKey,
        reg: Registration,
        decoded: Bundle,
        live_handle: impl Fn(u32) -> bool,
    ) -> u32 {
        self.register_filtered(key, reg, decoded, live_handle, false)
    }

    fn register_filtered(
        &self,
        key: ObjKey,
        reg: Registration,
        decoded: Bundle,
        live_handle: impl Fn(u32) -> bool,
        refuse_fifo_update: bool,
    ) -> u32 {
        let mut st = self.st();
        // `serverutilValidateNewResourceHandle(hClient, hDeferredApi)` (`:76-79`), then a
        // duplicate on THIS object (`:81-87`) — both INVALID_OBJECT_HANDLE.
        let refused = HandleRules::structurally_refused(key.client, reg.handle)
            || live_handle(reg.handle)
            || st
                .objects
                .get(&key)
                .is_some_and(|t| t.entries.contains_key(&reg.handle));
        if refused {
            st.stats.refused += 1;
            return NV_ERR_INVALID_OBJECT_HANDLE;
        }
        let b = st.bounds;
        if st.live(key) >= b.per_object
            || st.per_client.get(&key.client).copied().unwrap_or(0) >= b.per_client
            || st.total >= b.per_vm
        {
            st.stats.refused += 1;
            return NV_ERR_NO_MEMORY;
        }
        if refuse_fifo_update {
            st.stats.refused += 1;
            st.stats.fifo_update_refused += 1;
            if st.stats.fifo_update_refused == 1 {
                eprintln!(
                    "kf-rm: EXPERIMENT KF3_WIN_REFUSE_PT_FIFO_UPDATE: first refusal client={:#x} object={:#x} api={:#x} cmd={:#x} status=NV_ERR_NOT_SUPPORTED count=1; deliberately differs from native registration OK",
                    key.client, key.object, reg.handle, reg.cmd
                );
            }
            return kf_abi::defapi::NV_ERR_NOT_SUPPORTED;
        }
        let e = Entry {
            allocation: Arc::new(()),
            handle: reg.handle,
            form: reg.form,
            cmd: reg.cmd,
            flags: reg.flags,
            client_va: reg.client_va,
            device_va: reg.device_va,
            bundle: reg.bundle,
            decoded,
            registrant: Registrant::GuestKernelRpc,
            executed: false,
            tlb_flushed: false,
        };
        st.objects
            .entry(key)
            .or_default()
            .entries
            .insert(e.handle, e);
        *st.per_client.entry(key.client).or_default() += 1;
        st.total += 1;
        st.stats.registered += 1;
        self.bump();
        NV_OK
    }

    /// ★ `_REMOVE_API` (`_Class5080DelDeferredApi`): `NV_OK`, or `NV_ERR_GENERIC` for a handle the
    /// object does not hold.
    pub fn remove(&self, key: ObjKey, handle: u32) -> u32 {
        let mut st = self.st();
        let removed = st.objects.get_mut(&key).is_some_and(|t| t.delete(handle));
        if !removed {
            return NV_ERR_GENERIC;
        }
        st.count_down(key.client, 1);
        st.stats.removed += 1;
        self.bump();
        NV_OK
    }

    /// ★ The trigger's lookup (`_Class5080GetDeferredApiInfo`): the entry `Data` names on THIS
    /// object, or `NV_ERR_INVALID_DATA`.
    ///
    /// # Errors
    /// `NV_ERR_INVALID_DATA`.
    pub fn lookup(&self, key: ObjKey, handle: u32) -> Result<Entry, u32> {
        let mut st = self.st();
        let found = st
            .objects
            .get(&key)
            .and_then(|t| t.entries.get(&handle))
            .cloned();
        match found {
            Some(e) => {
                st.stats.looked_up += 1;
                Ok(e)
            }
            None => {
                st.stats.unknown += 1;
                Err(NV_ERR_INVALID_DATA)
            }
        }
    }

    /// ★ The trigger's cleanup (`deferred_api.c:653-670`), once the host-authored action has
    /// finished, WHATEVER its outcome (the real RM runs it on success and failure alike):
    /// `tlb_flushed` — the entry was a deferred `DMA_INVALIDATE_TLB` that succeeded, so the
    /// waiting entries are updated FIRST (`:525-526`), then this one is marked executed and
    /// deleted implicitly unless EXPLICIT or WAIT_FOR_TLB_FLUSH (then counted waiting). An entry
    /// a `_REMOVE_API` took while the action ran is simply gone.
    /// This numeric compatibility API requires external exclusion of remove and
    /// reuse. Asynchronous channel actions must use executed_entry instead.
    pub fn executed(&self, key: ObjKey, handle: u32, tlb_flushed: bool) {
        let mut st = self.st();
        if st.executed(key, handle, tlb_flushed) > 0 {
            self.bump();
        }
    }

    /// Complete only the exact registration resolved by the trigger. A guest
    /// remove/re-register or object destruction must not let a delayed action
    /// mark/delete a replacement or flush its new table. Returns false without
    /// changing entries when the original allocation is gone.
    pub fn executed_entry(&self, key: ObjKey, entry: &Entry, tlb_flushed: bool) -> bool {
        let mut st = self.st();
        let current = st
            .objects
            .get(&key)
            .and_then(|t| t.entries.get(&entry.handle));
        if !current.is_some_and(|e| Arc::ptr_eq(&e.allocation, &entry.allocation)) {
            st.stats.stale_completions += 1;
            return false;
        }
        if st.executed(key, entry.handle, tlb_flushed) > 0 {
            self.bump();
        }
        true
    }

    /// ★ `defapiDestruct_IMPL`: drop the tree of every object `alive` says is gone.
    pub fn prune(&self, alive: impl Fn(ObjKey) -> bool) {
        let mut st = self.st();
        let gone: Vec<ObjKey> = st.objects.keys().copied().filter(|k| !alive(*k)).collect();
        let mut dropped = 0;
        for k in gone {
            if let Some(t) = st.objects.remove(&k) {
                let n = t.entries.len();
                st.count_down(k.client, n);
                st.stats.destroyed += n as u64;
                dropped += n;
            }
        }
        if dropped > 0 {
            self.bump();
        }
    }

    /// ★ 2026-10-16 (`V3_USER_RING_LIFECYCLE_20261016.md` §1.1): the monotone epoch of the set of live
    /// entries (acquire load; see the field). One atomic load, no lock: the USER-ring worker calls it for
    /// every GP entry.
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.epoch.load(std::sync::atomic::Ordering::Acquire)
    }

    /// ★ 2026-10-16: the number of live entries of `key`, whatever their kind and state (an executed
    /// EXPLICIT-delete entry is still live). Takes the registry lock: call it only when [`Registry::epoch`]
    /// moved.
    #[must_use]
    pub fn live_count(&self, key: ObjKey) -> u32 {
        u32::try_from(self.st().live(key)).unwrap_or(u32::MAX)
    }

    /// The live entries of `key`, in handle order (tests, the log).
    #[must_use]
    pub fn entries(&self, key: ObjKey) -> Vec<Entry> {
        self.st()
            .objects
            .get(&key)
            .map(|t| t.entries.values().cloned().collect())
            .unwrap_or_default()
    }

    /// `NumWaitingOnTLBFlush` of `key`.
    #[must_use]
    pub fn waiting_tlb(&self, key: ObjKey) -> u32 {
        self.st().objects.get(&key).map_or(0, |t| t.waiting_tlb)
    }

    /// `(live entries in the VM, counters)`.
    #[must_use]
    pub fn stats(&self) -> (usize, Stats) {
        let st = self.st();
        (st.total, st.stats)
    }
}

/// ★ Serve one 5080 control on `key` (the caller resolved `ctl` from the command id and checked
/// that `key` is a live 5080 object): the GSP's status. `params` are the control's bytes.
pub fn serve(
    reg: &Registry,
    abi: &DefApiAbi,
    key: ObjKey,
    ctl: Control,
    params: &[u8],
    live_handle: impl Fn(u32) -> bool,
) -> u32 {
    serve_with_fifo_refusal(reg, abi, key, ctl, params, live_handle, false)
}

/// Diagnostic registration filter. `refuse_on_channel` is host-authored from the live
/// parent channel, not the bundle's guest-supplied target. Other commands/removals are unchanged.
pub fn serve_with_fifo_refusal(
    reg: &Registry,
    abi: &DefApiAbi,
    key: ObjKey,
    ctl: Control,
    params: &[u8],
    live_handle: impl Fn(u32) -> bool,
    refuse_on_channel: bool,
) -> u32 {
    match ctl {
        Control::Register(form) => match abi.decode_registration(form, params) {
            Ok(r) => {
                let decoded = abi.decode_bundle(r.cmd, &r.bundle);
                let refuse = refuse_on_channel && r.cmd == abi.cmds.update_channel_info;
                reg.register_filtered(key, r, decoded, live_handle, refuse)
            }
            Err(_) => kf_abi::defapi::NV_ERR_INVALID_PARAM_STRUCT,
        },
        Control::Remove => match abi.decode_remove(params) {
            Ok(h) => reg.remove(key, h),
            Err(_) => kf_abi::defapi::NV_ERR_INVALID_PARAM_STRUCT,
        },
    }
}

kf_util::assert_send_sync!(Registry);

/// Optional host-authored admission policy, consulted only on the RPC drainer, never a vCPU.
/// No host action, guest-memory access, or waiting for GPU progress is permitted here.
pub trait FifoUpdateFilter: std::fmt::Debug + Send + Sync {
    /// Whether this live parent channel is in the FIFO-refusal experiment's scope.
    fn refuse_on_channel(&self, client: u32, channel: u32) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_abi::DriverVersion;

    const W: DriverVersion = DriverVersion {
        major: 580,
        minor: 65,
        patch: 6,
    };
    const OBJ: ObjKey = ObjKey {
        client: 0xc1d0_0015,
        object: 0xff1f_e010,
    };

    fn abi() -> DefApiAbi {
        DefApiAbi::at(W).unwrap()
    }

    fn params(handle: u32, cmd: u32, flags: u32) -> Vec<u8> {
        let mut p = vec![0u8; 584];
        p[0..4].copy_from_slice(&handle.to_le_bytes());
        p[4..8].copy_from_slice(&cmd.to_le_bytes());
        p[8..12].copy_from_slice(&flags.to_le_bytes());
        p
    }

    fn reg(r: &Registry, handle: u32, cmd: u32, flags: u32) -> u32 {
        serve(
            r,
            &abi(),
            OBJ,
            Control::Register(Form::V1),
            &params(handle, cmd, flags),
            |_| false,
        )
    }

    /// ★ Device reset: the previous life's registrations are gone (a handle the next life reuses
    /// finds nothing), the bounds hold for the next life exactly as on a fresh registry.
    #[test]
    fn a_cleared_registry_holds_nothing_of_the_old_life_and_admits_the_next_one() {
        let r = Registry::new(Bounds::default());
        let fresh = Registry::new(Bounds::default());
        assert_eq!(reg(&r, 0x4000_0002, 0x2080_012d, 0), NV_OK);
        assert_eq!(reg(&r, 0x4000_0003, 0x2080_012b, 0), NV_OK);
        assert_eq!(r.stats().0, 2);
        assert_eq!(r.clear_life(), 2);
        assert_eq!(r.stats().0, 0);
        assert_eq!(r.lookup(OBJ, 0x4000_0002), fresh.lookup(OBJ, 0x4000_0002));
        assert_eq!(r.entries(OBJ), fresh.entries(OBJ));
        assert_eq!(
            reg(&r, 0x4000_0002, 0x2080_012d, 0),
            reg(&fresh, 0x4000_0002, 0x2080_012d, 0)
        );
        assert_eq!(r.entries(OBJ), fresh.entries(OBJ));
        assert_eq!(r.clear_life(), 1);
        assert_eq!(r.clear_life(), 0);
    }

    /// Fallback experiment: refuse only the requested inner command, with no saved entry.
    #[test]
    fn fifo_refusal_is_narrow_and_preserves_validation_and_object() {
        let r = Registry::new(Bounds::default());
        let abi = abi();
        let update = abi.cmds.update_channel_info;
        let call = |handle, cmd, scope| {
            serve_with_fifo_refusal(
                &r,
                &abi,
                OBJ,
                Control::Register(Form::V1),
                &params(handle, cmd, 0),
                |_| false,
                scope,
            )
        };
        assert_eq!(call(0, update, true), NV_ERR_INVALID_OBJECT_HANDLE);
        assert_eq!(
            call(0x4000_0002, update, true),
            kf_abi::defapi::NV_ERR_NOT_SUPPORTED
        );
        assert_eq!(r.lookup(OBJ, 0x4000_0002), Err(NV_ERR_INVALID_DATA));
        assert_eq!(r.stats().0, 0);
        assert_eq!(r.stats().1.fifo_update_refused, 1);
        // Other deferred APIs and channels outside the experiment remain unchanged.
        assert_eq!(call(0x4000_0003, abi.cmds.invalidate_tlb, true), NV_OK);
        assert_eq!(call(0x4000_0002, update, false), NV_OK);
        assert_eq!(
            call(0x4000_0002, update, true),
            NV_ERR_INVALID_OBJECT_HANDLE
        );
        assert_eq!(r.stats().1.fifo_update_refused, 1);
        assert_eq!(r.remove(OBJ, 0x4000_0002), NV_OK);
        assert_eq!(
            call(0x4000_0002, update, true),
            kf_abi::defapi::NV_ERR_NOT_SUPPORTED
        );
        assert_eq!(r.stats().1.fifo_update_refused, 2);
        assert_eq!(
            r.lookup(OBJ, 0x4000_0003).unwrap().cmd,
            abi.cmds.invalidate_tlb
        );
    }

    /// ★ The VFIO sequence: register, look up, execute → implicitly deleted; the same handle may
    /// then be registered again (the falsification's "executed" observable).
    #[test]
    fn implicit_delete_after_execution_frees_the_handle() {
        let r = Registry::new(Bounds::default());
        assert_eq!(reg(&r, 0x4000_0002, 0x2080_012d, 0), NV_OK);
        assert_eq!(reg(&r, 0x4000_0003, 0x2080_012b, 0), NV_OK);
        assert_eq!(
            reg(&r, 0x4000_0002, 0x2080_012d, 0),
            NV_ERR_INVALID_OBJECT_HANDLE,
            "duplicate"
        );
        let e = r.lookup(OBJ, 0x4000_0002).unwrap();
        assert_eq!(e.cmd, 0x2080_012d);
        assert_eq!(e.registrant, Registrant::GuestKernelRpc);
        r.executed(OBJ, 0x4000_0002, false);
        assert_eq!(r.lookup(OBJ, 0x4000_0002), Err(NV_ERR_INVALID_DATA));
        assert_eq!(
            reg(&r, 0x4000_0002, 0x2080_012d, 0),
            NV_OK,
            "re-registrable"
        );
        assert_eq!(r.stats().0, 2);
    }

    #[test]
    fn deferred_completion_cannot_delete_reused_registration() {
        let r = Registry::default();
        let h = 0x4000_0002;
        assert_eq!(reg(&r, h, 0x2080_012d, 0), NV_OK);
        let old = r.lookup(OBJ, h).unwrap();
        assert_eq!(r.remove(OBJ, h), NV_OK);
        // Identical bytes and numeric names still denote a NEW registration.
        assert_eq!(reg(&r, h, 0x2080_012d, 0), NV_OK);
        assert!(!r.executed_entry(OBJ, &old, false));
        assert!(!r.lookup(OBJ, h).unwrap().executed);
        assert_eq!(r.stats().0, 1);
        let current = r.lookup(OBJ, h).unwrap();
        assert!(r.executed_entry(OBJ, &current, false));
        assert_eq!(r.lookup(OBJ, h), Err(NV_ERR_INVALID_DATA));
    }

    #[test]
    fn stale_flush_cannot_update_recreated_object_table() {
        let r = Registry::default();
        let flush = 0x4000_0002;
        let wait = 0x4000_0003;
        assert_eq!(reg(&r, flush, 0x2080_2502, 0), NV_OK);
        let old = r.lookup(OBJ, flush).unwrap();
        r.prune(|_| false);
        assert_eq!(reg(&r, flush, 0x2080_2502, 0), NV_OK);
        // Public WAIT_FOR_TLB_FLUSH bit from the existing ABI decoder.
        assert_eq!(reg(&r, wait, 0x2080_012d, 2), NV_OK);
        let waiting = r.lookup(OBJ, wait).unwrap();
        assert!(waiting.flags.wait_tlb_flush);
        assert!(r.executed_entry(OBJ, &waiting, false));
        assert_eq!(r.waiting_tlb(OBJ), 1);
        assert!(!r.executed_entry(OBJ, &old, true));
        assert_eq!(r.waiting_tlb(OBJ), 1);
        assert_eq!(r.stats().0, 2);
        let current = r.lookup(OBJ, flush).unwrap();
        assert!(r.executed_entry(OBJ, &current, true));
        assert_eq!(r.waiting_tlb(OBJ), 0);
        assert_eq!(r.stats().0, 0);
    }

    /// ★ Hostile handles, the F7a set (2026-10-07, host 595.91.07): 0, the client's own, the firmware range, a live
    /// handle; 0xffffffff and an arbitrary one register. The inner cmd is never checked.
    #[test]
    fn handle_validation_and_unchecked_cmd() {
        let r = Registry::new(Bounds::default());
        let live = |h: u32| h == 0xff04_0001;
        for h in [0, OBJ.client, 0xc9f0_0000, 0xc9f7_ffff, 0xff04_0001] {
            let st = serve(
                &r,
                &abi(),
                OBJ,
                Control::Register(Form::V1),
                &params(h, 0x2080_2502, 0),
                live,
            );
            assert_eq!(st, NV_ERR_INVALID_OBJECT_HANDLE, "{h:#x}");
        }
        assert_eq!(
            reg(&r, 0xffff_ffff, 0xdead_beef, 0),
            NV_OK,
            "cmd unchecked at registration"
        );
        assert_eq!(reg(&r, 1, 0x2080_2502, 0), NV_OK);
        // Wrong sizes are the param-struct refusal, and touch nothing.
        let st = serve(
            &r,
            &abi(),
            OBJ,
            Control::Register(Form::V1),
            &[0u8; 583],
            |_| false,
        );
        assert_eq!(st, kf_abi::defapi::NV_ERR_INVALID_PARAM_STRUCT);
        assert_eq!(r.stats().1.registered, 2);
        // Per-object trees: the same handle on another object is independent.
        let other = ObjKey {
            client: OBJ.client,
            object: 0xff1f_e011,
        };
        assert_eq!(
            serve(
                &r,
                &abi(),
                other,
                Control::Register(Form::V1),
                &params(1, 0, 0),
                |_| false
            ),
            NV_OK
        );
        assert_eq!(
            r.lookup(other, 0xffff_ffff),
            Err(NV_ERR_INVALID_DATA),
            "never cross-object"
        );
    }

    #[test]
    fn remove_explicit_and_generic() {
        let r = Registry::new(Bounds::default());
        assert_eq!(reg(&r, 7, 0x2080_012b, 1), NV_OK); // DELETE_EXPLICIT
        r.executed(OBJ, 7, false);
        assert!(r.lookup(OBJ, 7).is_ok(), "explicit: survives the trigger");
        let rm = |h: u32| {
            serve(&r, &abi(), OBJ, Control::Remove, &h.to_le_bytes(), |_| {
                false
            })
        };
        assert_eq!(rm(7), NV_OK);
        assert_eq!(rm(7), NV_ERR_GENERIC, "absent");
        assert_eq!(rm(0x1234), NV_ERR_GENERIC);
        assert_eq!(r.stats().0, 0);
        assert_eq!(
            serve(&r, &abi(), OBJ, Control::Remove, &[0; 8], |_| false),
            kf_abi::defapi::NV_ERR_INVALID_PARAM_STRUCT
        );
    }

    /// ★ WAIT_FOR_TLB_FLUSH accounting, exactly `deferred_api.c`'s order: a waiting entry
    /// survives its own execution, is counted, and is deleted by the NEXT successful deferred TLB
    /// invalidate on the same object — not by its own, not by a failed one.
    #[test]
    fn wait_for_tlb_flush_accounting() {
        let r = Registry::new(Bounds::default());
        assert_eq!(reg(&r, 10, 0x2080_012b, 2), NV_OK); // implicit + WAIT
        assert_eq!(reg(&r, 11, 0x2080_2502, 2), NV_OK); // a TLB invalidate that also waits
        assert_eq!(reg(&r, 12, 0x2080_2502, 0), NV_OK);
        assert_eq!(reg(&r, 13, 0x2080_012b, 3), NV_OK); // explicit + WAIT
        r.executed(OBJ, 10, false);
        r.executed(OBJ, 13, false);
        assert_eq!(
            r.waiting_tlb(OBJ),
            1,
            "explicit entries are never counted waiting"
        );
        assert!(r.lookup(OBJ, 10).is_ok());
        // 11 succeeds: 10 is flushed and deleted first; 11 itself then waits.
        r.executed(OBJ, 11, true);
        assert!(r.lookup(OBJ, 10).is_err());
        assert!(r.lookup(OBJ, 11).is_ok());
        assert_eq!(r.waiting_tlb(OBJ), 1);
        // A failed invalidate (tlb_flushed = false) flushes nothing; 12 is consumed anyway.
        r.executed(OBJ, 12, false);
        assert!(r.lookup(OBJ, 11).is_ok());
        assert!(r.lookup(OBJ, 12).is_err());
        // Removing the executed-unflushed 11 decrements the count.
        assert_eq!(r.remove(OBJ, 11), NV_OK);
        assert_eq!(r.waiting_tlb(OBJ), 0);
        assert!(r.lookup(OBJ, 13).is_ok(), "explicit stays until removed");
    }

    #[test]
    fn bounds_answer_no_memory_and_prune_frees() {
        let r = Registry::new(Bounds {
            per_object: 2,
            per_client: 3,
            per_vm: 4,
        });
        assert_eq!(reg(&r, 1, 0, 0), NV_OK);
        assert_eq!(reg(&r, 2, 0, 0), NV_OK);
        assert_eq!(reg(&r, 3, 0, 0), NV_ERR_NO_MEMORY, "per object");
        let o2 = ObjKey {
            client: OBJ.client,
            object: 9,
        };
        let at = |k, h| {
            serve(
                &r,
                &abi(),
                k,
                Control::Register(Form::V1),
                &params(h, 0, 0),
                |_| false,
            )
        };
        assert_eq!(at(o2, 1), NV_OK);
        assert_eq!(at(o2, 2), NV_ERR_NO_MEMORY, "per client");
        let o3 = ObjKey {
            client: 5,
            object: 9,
        };
        assert_eq!(at(o3, 1), NV_OK);
        assert_eq!(at(o3, 2), NV_ERR_NO_MEMORY, "per VM");
        r.prune(|k| k != OBJ);
        assert_eq!(r.stats().0, 2);
        assert_eq!(r.stats().1.destroyed, 2);
        assert_eq!(reg(&r, 1, 0, 0), NV_OK, "a re-created object starts empty");
    }

    /// ★ 2026-10-16: the epoch moves exactly when the set of live entries changes, and an entry is
    /// visible to `live_count` by the time the epoch that announces it is.
    #[test]
    fn the_epoch_counts_changes_of_the_live_set_and_nothing_else() {
        let r = Registry::new(Bounds::default());
        let e0 = r.epoch();
        assert_eq!(r.live_count(OBJ), 0);
        // a refused registration changes nothing
        assert_eq!(reg(&r, 0, 0x2080_012d, 0), NV_ERR_INVALID_OBJECT_HANDLE);
        assert_eq!(r.epoch(), e0);
        assert_eq!(reg(&r, 0x4000_0002, 0x2080_012d, 0), NV_OK);
        assert!(r.epoch() > e0);
        assert_eq!(r.live_count(OBJ), 1);
        // a duplicate is refused
        let e1 = r.epoch();
        assert_eq!(
            reg(&r, 0x4000_0002, 0x2080_012d, 0),
            NV_ERR_INVALID_OBJECT_HANDLE
        );
        assert_eq!(r.epoch(), e1);
        // lookups, stats and entries() do not move it
        let _ = r.lookup(OBJ, 0x4000_0002);
        let _ = r.lookup(OBJ, 0x4000_0009);
        let _ = (r.stats(), r.entries(OBJ), r.waiting_tlb(OBJ));
        assert_eq!(r.epoch(), e1);
        // executing an implicit entry deletes it: a change
        r.executed(OBJ, 0x4000_0002, false);
        assert!(r.epoch() > e1);
        assert_eq!(r.live_count(OBJ), 0);
        // executing an EXPLICIT entry deletes nothing: the live set (and so the epoch) is unchanged
        assert_eq!(reg(&r, 7, 0x2080_012b, 1), NV_OK);
        let e2 = r.epoch();
        r.executed(OBJ, 7, false);
        assert_eq!(r.epoch(), e2);
        assert_eq!(
            r.live_count(OBJ),
            1,
            "an executed explicit entry is still live"
        );
        // REMOVE_API of an absent handle is a refusal and changes nothing
        assert_eq!(r.remove(OBJ, 99), NV_ERR_GENERIC);
        assert_eq!(r.epoch(), e2);
        assert_eq!(r.remove(OBJ, 7), NV_OK);
        assert!(r.epoch() > e2);
        assert_eq!(r.live_count(OBJ), 0);
    }

    #[test]
    fn prune_and_clear_life_move_the_epoch_only_when_they_drop_entries() {
        let r = Registry::new(Bounds::default());
        let e = r.epoch();
        r.prune(|_| true);
        assert_eq!(r.clear_life(), 0);
        assert_eq!(r.epoch(), e, "nothing to drop");
        assert_eq!(reg(&r, 0x4000_0002, 0x2080_012d, 0), NV_OK);
        let e1 = r.epoch();
        r.prune(|_| true);
        assert_eq!(r.epoch(), e1, "the object is alive");
        r.prune(|_| false);
        assert!(r.epoch() > e1);
        assert_eq!(r.live_count(OBJ), 0);
        assert_eq!(reg(&r, 0x4000_0002, 0x2080_012d, 0), NV_OK);
        let e2 = r.epoch();
        assert_eq!(r.clear_life(), 1);
        assert!(r.epoch() > e2);
        assert_eq!(r.live_count(OBJ), 0);
    }

    #[test]
    fn live_count_is_per_object_and_counts_every_bundle_kind() {
        let r = Registry::new(Bounds::default());
        let other = ObjKey {
            client: OBJ.client,
            object: 0xff1f_e011,
        };
        let abi = abi();
        for (i, cmd) in [
            abi.cmds.update_channel_info,
            abi.cmds.invalidate_tlb,
            0xdead_beef,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(reg(&r, 0x4000_0002 + i as u32, cmd, 0), NV_OK);
        }
        assert_eq!(
            serve(
                &r,
                &abi,
                other,
                Control::Register(Form::V1),
                &params(5, 0, 0),
                |_| false
            ),
            NV_OK
        );
        assert_eq!((r.live_count(OBJ), r.live_count(other)), (3, 1));
    }

    /// The order the worker relies on: the worker loads the epoch (acquire) and THEN counts. A
    /// registrar that bumped the epoch before the entry was visible would let it read an old count.
    #[test]
    fn a_reader_that_sees_an_epoch_sees_every_registration_it_announces() {
        let r = std::sync::Arc::new(Registry::new(Bounds {
            per_object: 1 << 20,
            per_client: 1 << 20,
            per_vm: 1 << 20,
        }));
        let n = 20_000u32;
        let writer = {
            let r = r.clone();
            std::thread::spawn(move || {
                for h in 0..n {
                    assert_eq!(reg(&r, 0x4000_0000 + h, 0x2080_012d, 2), NV_OK);
                }
            })
        };
        while !writer.is_finished() {
            let e = r.epoch();
            let c = r.live_count(OBJ);
            assert!(
                u64::from(c) >= e,
                "epoch {e} announced more entries than the {c} visible"
            );
        }
        writer.join().unwrap();
        assert!(
            r.epoch() >= u64::from(n),
            "every registration moved the epoch"
        );
    }
}
