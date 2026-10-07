//! ★ OWNER_RULINGS §U (2026-10-07) — **the per-object deferred-API entry tables**: what the GSP's
//! `DeferredApiObject` keeps (`ogkm-580.95.05: src/nvidia/src/kernel/gpu/deferred_api.c`), held by
//! the object graph's seat and read by the channel plane at the trigger.
//!
//! | real RM | here |
//! |---|---|
//! | `_Class5080AddDeferredApi` (`:60-116`): validate the handle as a new resource handle of the OBJECT's client, refuse a duplicate on the object, copy the params, record the caller's `privLevel` | [`Registry::register`] — same checks, same order, same statuses; the cmd is NOT checked |
//! | `_Class5080DelDeferredApi` (`:140-182`): unlink; a WAIT_FOR_TLB_FLUSH entry that executed and never saw a flush decrements `NumWaitingOnTLBFlush`; `NV_ERR_GENERIC` if absent | [`Registry::remove`] |
//! | `_class5080DeferredApiV2` (`:443-674`): look `Data` up in THIS object's tree (`NV_ERR_INVALID_DATA` if absent), run the control, then — whatever its status — mark executed and delete implicitly unless EXPLICIT or WAIT_FOR_TLB_FLUSH (then count it waiting) | [`Registry::lookup`] at the trigger, [`Registry::executed`] when the host-authored action has finished |
//! | `_Class5080UpdateTLBFlushState` (`:185-227`), run by a successful deferred `DMA_INVALIDATE_TLB` BEFORE its own entry is marked | [`Registry::executed`] with `tlb_flushed = true` |
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
use std::sync::Mutex;

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
        }
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
        let e = Entry {
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
    pub fn executed(&self, key: ObjKey, handle: u32, tlb_flushed: bool) {
        let mut st = self.st();
        let mut deleted = 0;
        if let Some(t) = st.objects.get_mut(&key) {
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
            st.count_down(key.client, deleted);
            st.stats.implicit_deletes += deleted as u64;
        }
    }

    /// ★ `defapiDestruct_IMPL`: drop the tree of every object `alive` says is gone.
    pub fn prune(&self, alive: impl Fn(ObjKey) -> bool) {
        let mut st = self.st();
        let gone: Vec<ObjKey> = st.objects.keys().copied().filter(|k| !alive(*k)).collect();
        for k in gone {
            if let Some(t) = st.objects.remove(&k) {
                let n = t.entries.len();
                st.count_down(k.client, n);
                st.stats.destroyed += n as u64;
            }
        }
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
    match ctl {
        Control::Register(form) => match abi.decode_registration(form, params) {
            Ok(r) => {
                let decoded = abi.decode_bundle(r.cmd, &r.bundle);
                reg.register(key, r, decoded, live_handle)
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
}
