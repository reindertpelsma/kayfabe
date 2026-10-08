//! ★★★★★ §16.76 — **the os-event registry**: which `(hClient, hEvent, notifyIndex)` this
//! device may post a wakeup to, and when it stops being allowed to.
//!
//! ## ⊘ v3 port note
//!
//! The registry and its retire path are kept verbatim. What is NOT kept is the old delivery
//! shape: a broadcast `batch()` of every live registration, posted whenever anything might
//! have completed, plus a `note_join` instrument joined to the old CPU copy-engine executor's
//! counters. In v3 each registration is paired with a HOST event (`kf_host` os-event + event
//! fd, P5) and a wakeup is posted only for the registration whose host event fired —
//! [`OsEventLog::find`] is that seam, with the pairing marked `TODO(P5)` there. This crate
//! posts nothing and writes no guest semaphore.
//!
//! # Why a registry exists at all
//!
//! `kf_gsp::GspFsm::deliver_events` posts one
//! `NV_VGPU_MSG_EVENT_POST_EVENT` per registered event, and the guest's `_kgspRpcPostEvent`
//! resolves it with `CliGetEventInfo(hClient, hEvent)` before calling `osNotifyEvent`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:497-535`). So the pair is not
//! decoration — it **is** the address of the wakeup, and the only place it is ever stated
//! is the `GSP_RM_ALLOC` of an `NV01_EVENT_OS_EVENT`.
//!
//! `[measured 2026-08-10, boot w209_ffc80f8_ctl, rev ffc80f8]`
//! (`traces/guest_boots/run_w209_ffc80f8_ctl_probe.log`) libcuda registers **seven** of
//! them under its own client `0xc1d0000c`, and every one was refused `status=0x00000056`
//! because `kf_abi::versions::DriverAbiTable::alloc_params` had no arm for the class.
//! `w210` (`8574466`) removed the guest's give-up path and the same process then never
//! returned from `cuCtxCreate` at all — the registrations were being made and nothing could
//! ever answer them.
//!
//! # ★★★ Why the FREE path is half the module, and not an afterthought
//!
//! `C: src/qemu/nvkvm_gpu_emul.c:1875-1884` carries the strongest warning in the C's whole
//! event plane, and it is a *reproduction* rather than a reading:
//!
//! > *"Without this, `nvkvm_gsp_deliver_events` keeps POSTing `POST_EVENT` to dead
//! > `(hClient, hEvent)` pairs → guest `_kgspRpcPostEvent`'s `CliGetEventInfo` returns
//! > `OBJECT_NOT_FOUND`, the SHARED status queue's seqNum desyncs (\"Bad sequence
//! > number\"), and the whole RPC/event path wedges. Reproduced THREE independent ways on
//! > bare-metal .32: PyTorch CUDA-init hang, 2-process concurrent compute hang, and
//! > nvidia-smi-then-cup8."*
//!
//! ⇒ a registry without a retire path is not a smaller feature, it is a **different and
//! worse** one: it converts a missing wakeup into a broken transport. The retire path is
//! [`OsEventLog::retire`], driven from the guest's own `FREE`.
//!
//! # ⊘ Why this is a `CommandObserver` and not a policy link
//!
//! It must see the alloc, and it must not answer it — the answerer is the object model,
//! which now decodes the class as `AllocParams::NoDeclaredFacts`. A
//! [`kf_gsp::CommandObserver`] has no return value, so *"this link changes no reply
//! byte"* is `rustc`'s guarantee rather than a sentence in a comment. Same seat, same
//! reasoning as [`crate::faultbuffer::FaultBufferRecorder`].
//!
//! # ⚠ What this module reads out of guest-supplied params, and what it refuses to
//!
//! Exactly one field: `notifyIndex`, a plain `u32` at `NV0005_ALLOC_PARAMETERS + 12`. ⊘ It
//! does **not** read `data` @ +16, which is an `NvP64` guest-kernel callback pointer
//! (`ogkm-580: src/common/sdk/nvidia/inc/class/cl0005.h:40-47`) — nothing in this tree
//! dereferences a guest pointer, and the field is not even loaded here so that no later
//! edit can start. On this RPC the params are RM's own stack-local struct
//! (`ogkm-580: inc/kernel/vgpu/rpc.h:345-357`, 24 bytes, matching the measured
//! `paramsSize=0x18`), not libcuda's, so reading one word of it is not a read of user
//! memory either.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use kf_abi::generated::classes::{
    NV01_EVENT_KERNEL_CALLBACK, NV01_EVENT_KERNEL_CALLBACK_EX, NV01_EVENT_OS_EVENT,
};
use kf_abi::postevent::PostEvent;
use kf_abi::versions::DriverAbiTable;
use kf_gsp::{CommandObserver, RpcCommand, RpcFunction};

/// `NV0005_ALLOC_PARAMETERS.notifyIndex`'s byte offset within `params[]`.
///
/// `{ NvHandle hParentClient; NvHandle hSrcResource; NvV32 hClass; NvV32 notifyIndex;
/// NvP64 data; }` (`ogkm-580: src/common/sdk/nvidia/inc/class/cl0005.h:40-47`), so
/// `4 + 4 + 4 = 12`.
///
/// ⊘ Stated here, in the device crate, and that is a deliberate exception with a bound: the
/// quarantine rule (decision #2) exists so that no crate above `kayfabe-abi` states an
/// NVIDIA `#[repr(C)]` field offset. The *reason* `kayfabe-abi` mirrors no struct for this
/// class is that mirroring it would create a decoder for the `NvP64` callback pointer two
/// fields later — see this module's header. One named `u32` offset, read through a bounds
/// check, is the smaller of the two evils; `crate::osevent`'s tests pin it against the
/// measured `paramsSize` the guest sends.
const NOTIFY_INDEX_AT: usize = 12;

/// `sizeof(NV0005_ALLOC_PARAMETERS)` — and the `paramsSize` measured on the wire
/// (`hClass=0x00000079; paramsSize=0x00000018`, `w209`).
///
/// ⊘ Test-only, and deliberately NOT a length check on the decode path: the observer bounds
/// its read by the slice it was given, so a guest that sends a shorter params window is
/// counted [`OsEventLog::malformed`] rather than measured against a constant. This exists so
/// [`NOTIFY_INDEX_AT`] can be pinned against what the guest actually sends.
#[cfg(test)]
const NV0005_ALLOC_PARAMETERS_SIZE: usize = 24;

/// How many live registrations this device will hold.
///
/// ⊘ Bounded because the guest drives registration: a hostile or merely broken driver that
/// allocates events in a loop must not be able to grow a host allocation. The C's array is
/// 64 (`C:362`); this matches it rather than inventing a new number.
///
/// ★ What happens at the bound is a **refusal to remember**, counted as
/// [`OsEventLog::overflowed`] — never an eviction. Evicting a live registration would make
/// this device silently stop waking a waiter that is still there, which is the exact
/// failure the whole module exists to prevent.
pub const OS_EVENT_MAX: usize = 64;

/// One registered os-event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OsEventRegistration {
    /// `hClient` — the namespace, from the RPC header.
    pub client: u32,
    /// `hEvent` — the event object's handle, from the RPC header's `hObject`.
    pub event: u32,
    /// `notifyIndex`, from the params. Echoed back, interpreted by nobody here.
    pub notify_index: u32,
}

impl OsEventRegistration {
    /// The wakeup message that names this registration.
    #[must_use]
    pub fn post(&self) -> PostEvent {
        PostEvent {
            client: self.client,
            event: self.event,
            notify_index: self.notify_index,
        }
    }
}

/// ★★★ **The guest's non-stall subscriptions, by slot** (2026-10-08, owner ruling §X) — read
/// lock-free by the device's interrupt plane on every host non-stall edge.
///
/// A slot (`kf_abi::eventnotify::nonstall_slot`) is "the guest has a live event with
/// `NV01_EVENT_NONSTALL_INTR` on this notifier": the guest CPU-RM's own subscription, which it
/// walks when the device raises the engine's vector (`ogkm-595.84: event_notification.c:688-737`).
/// Counted up on the registration, down on the `FREE` that retires it, and cleared when the
/// guest's RM starts again (fn 1, [`OsEventLog::clear`]).
///
/// ★ Every event class counts, not only userspace's `NV01_EVENT_OS_EVENT`: the guest KERNEL's
/// own non-stall waiters are `NV01_EVENT_KERNEL_CALLBACK_EX` / `NV01_EVENT_KERNEL_CALLBACK` and
/// reach this device as the same alloc RPC (`ogkm-595.84: event_api.c:149-171`): CeUtils' channel
/// (`mem_utils.c:1905-1906`, `FIFO_EVENT_MTHD`), semaphore-surface waiters (`sem_surf.c:477-497`),
/// nvkms (`nvkms-rm.c:4274-4290`). Those are counted here only, never in the `POST_EVENT` registry
/// (a kernel callback is not a wakeup this device posts). Windows' KMD use of them is unverified;
/// the rule is generic.
///
/// ⊘ **Errs towards waking, never towards silence.** A registration the bounded table could not
/// remember ([`OS_EVENT_MAX`]) makes its slot STICKY (armed for the rest of the VM's life): its
/// retire could never be seen, and a missed wake is a correctness bug while a spurious one is
/// harmless (the guest's waiter re-checks its semaphore). An alloc the object model later
/// refused is still counted here (the observer cannot see the answer): spurious wakes only.
#[derive(Debug)]
pub struct NonstallArms {
    counts: [AtomicU32; kf_abi::eventnotify::NONSTALL_SLOTS],
    sticky: AtomicU64,
    /// Non-stall registrations of the guest kernel's callback classes seen (`0x78`/`0x7e`).
    pub kernel_registered: AtomicU64,
    /// Times the guest's RM started again (fn 1) and the subscriptions were cleared.
    pub clears: AtomicU64,
}

impl Default for NonstallArms {
    fn default() -> NonstallArms {
        NonstallArms {
            counts: std::array::from_fn(|_| AtomicU32::new(0)),
            sticky: AtomicU64::new(0),
            kernel_registered: AtomicU64::new(0),
            clears: AtomicU64::new(0),
        }
    }
}

impl NonstallArms {
    /// Whether the guest has a live (or sticky) non-stall subscription on `slot`. Two relaxed
    /// loads; an out-of-range slot is never armed.
    #[must_use]
    pub fn armed(&self, slot: usize) -> bool {
        self.counts
            .get(slot)
            .is_some_and(|c| c.load(Ordering::Relaxed) > 0)
            || (slot < 64 && self.sticky.load(Ordering::Relaxed) & (1 << slot) != 0)
    }

    /// Live subscriptions on `slot` (for the report).
    #[must_use]
    pub fn count(&self, slot: usize) -> u32 {
        self.counts
            .get(slot)
            .map_or(0, |c| c.load(Ordering::Relaxed))
    }

    /// The sticky slots, as a bit mask (for the report).
    #[must_use]
    pub fn sticky(&self) -> u64 {
        self.sticky.load(Ordering::Relaxed)
    }

    fn up(&self, slot: usize) {
        if let Some(c) = self.counts.get(slot) {
            c.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Only ever called under [`OsEventLog`]'s row lock (as is `up`), so writers never race and
    /// a load-then-store cannot lose an update; readers stay lock-free.
    fn down(&self, slot: usize) {
        if let Some(c) = self.counts.get(slot) {
            let v = c.load(Ordering::Relaxed);
            c.store(v.saturating_sub(1), Ordering::Relaxed);
        }
    }

    fn stick(&self, slot: usize) {
        if slot < 64 {
            self.sticky.fetch_or(1 << slot, Ordering::Relaxed);
        }
    }

    /// Every count and sticky bit back to zero — under the row locks, like `up`/`down`.
    fn zero(&self) {
        for c in &self.counts {
            c.store(0, Ordering::Relaxed);
        }
        self.sticky.store(0, Ordering::Relaxed);
    }
}

/// How many non-stall registrations of the guest kernel's callback classes this device will
/// remember (for their `FREE`). Past it a registration's slot STICKS ([`NonstallArms`]).
pub const KERNEL_NONSTALL_MAX: usize = 64;

/// One remembered kernel-callback non-stall registration: `(hClient, hEvent, slot)`.
type KernelRow = (u32, u32, usize);

/// One remembered registration and its non-stall subscription slot, if any.
type Row = (OsEventRegistration, Option<usize>);

/// The shared registry. Cloneable so the plane and the chain link hold the same one.
#[derive(Debug, Clone, Default)]
pub struct OsEventLog {
    live: Arc<Mutex<Vec<Row>>>,
    /// The guest kernel's non-stall callback registrations — arms only, never posted to.
    kernel: Arc<Mutex<Vec<KernelRow>>>,
    nonstall: Arc<NonstallArms>,
    registered: Arc<AtomicU64>,
    retired: Arc<AtomicU64>,
    overflowed: Arc<AtomicU64>,
    malformed: Arc<AtomicU64>,
    posted: Arc<AtomicU64>,
    batches: Arc<AtomicU64>,
    gated: Arc<AtomicU64>,
    not_running: Arc<AtomicU64>,
    failed: Arc<AtomicU64>,
}

impl OsEventLog {
    /// A fresh, empty registry.
    #[must_use]
    pub fn new() -> OsEventLog {
        OsEventLog::default()
    }

    /// Register `(client, event, notify_index)`, de-duplicated on `(client, event)`.
    ///
    /// ★ De-duplicated because that pair is the guest's own match key: two rows for one key
    /// would post the same wakeup twice per batch, doubling the ring pressure the gate
    /// exists to bound. The C dedups on exactly this pair (`C:2855-2859`).
    ///
    /// Returns whether a new row was added.
    pub fn register(&self, reg: OsEventRegistration) -> bool {
        self.register_slot(reg, None)
    }

    /// [`OsEventLog::register`], and count it on the guest's non-stall subscription `slot`
    /// ([`NonstallArms`]) — `None` for a registration that is not a non-stall one.
    ///
    /// ★ A pair already registered with a DIFFERENT notifier (the guest reused the handle pair
    /// after a `FREE` this device did not see) REPLACES the row and moves its arm to the new slot:
    /// keeping the old slot would leave the new subscription unarmed, a lost wake. Returns `true`
    /// for a replacement too.
    pub fn register_slot(&self, reg: OsEventRegistration, slot: Option<usize>) -> bool {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(row) = live
            .iter_mut()
            .find(|(r, _)| r.client == reg.client && r.event == reg.event)
        {
            if row.0 == reg && row.1 == slot {
                return false;
            }
            if let Some(old) = row.1 {
                self.nonstall.down(old);
            }
            if let Some(new) = slot {
                self.nonstall.up(new);
            }
            *row = (reg, slot);
            return true;
        }
        if live.len() >= OS_EVENT_MAX {
            self.overflowed.fetch_add(1, Ordering::Relaxed);
            // ⊘ Not remembered, so its FREE can never be seen: the slot stays armed for good
            // (a spurious wake is harmless, a missed one is not — [`NonstallArms`]).
            if let Some(s) = slot {
                self.nonstall.stick(s);
            }
            return false;
        }
        if let Some(s) = slot {
            self.nonstall.up(s);
        }
        live.push((reg, slot));
        self.registered.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// ★ A non-stall registration of the guest KERNEL's callback classes (`0x78`/`0x7e`) on
    /// `slot`: counted in [`NonstallArms`] only (never posted to), retired by the same `FREE`s as
    /// every row ([`OsEventLog::retire`]). Dedup and replacement as [`OsEventLog::register_slot`].
    pub fn register_kernel_nonstall(&self, client: u32, event: u32, slot: usize) {
        let mut rows = self.kernel.lock().unwrap_or_else(|e| e.into_inner());
        self.nonstall
            .kernel_registered
            .fetch_add(1, Ordering::Relaxed);
        if let Some(row) = rows.iter_mut().find(|r| r.0 == client && r.1 == event) {
            if row.2 != slot {
                self.nonstall.down(row.2);
                self.nonstall.up(slot);
                row.2 = slot;
            }
            return;
        }
        if rows.len() >= KERNEL_NONSTALL_MAX {
            self.overflowed.fetch_add(1, Ordering::Relaxed);
            self.nonstall.stick(slot);
            return;
        }
        self.nonstall.up(slot);
        rows.push((client, event, slot));
    }

    /// ★ The guest's RM started again (fn 1, `SET_GUEST_SYSTEM_INFO`): every registration of the
    /// previous RM life is dead. Drop all rows and zero every non-stall arm (a reused handle pair
    /// must not inherit an old slot, and a stale arm must not outlive its RM).
    pub fn clear(&self) {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let mut rows = self.kernel.lock().unwrap_or_else(|e| e.into_inner());
        live.clear();
        rows.clear();
        self.nonstall.zero();
        self.nonstall.clears.fetch_add(1, Ordering::Relaxed);
    }

    /// ★ The guest's non-stall subscriptions — the same counters the registry keeps, shared
    /// (`Arc`), read lock-free by the device's interrupt plane.
    #[must_use]
    pub fn nonstall_arms(&self) -> Arc<NonstallArms> {
        Arc::clone(&self.nonstall)
    }

    /// ★★★ Retire every row the guest's `FREE` killed. See this module's header for what
    /// posting to a dead pair does to the shared status queue.
    ///
    /// Two shapes, both the C's (`C:1885-1903`):
    ///
    /// - `handle` is the **event** — drop that one row;
    /// - `client == handle` — the guest freed its client ROOT, which tears down every
    ///   object under it, so drop every row in that namespace.
    ///
    /// ⚠ The second test is `fClient == fObj`, RM's own encoding of a root free
    /// (`serverAllocClient` writes `hResource = hClient`, so a root's handle *is* its
    /// client). ⊘ `kayfabe_rmrpc::translate_free` deliberately refuses to make this
    /// inference for the OBJECT MODEL, where a dup can keep a resource alive past its
    /// origin handle and the mis-fire is catastrophic. It is safe **here** and only here,
    /// because the consequence is opposite in sign: a row dropped too eagerly costs a
    /// wakeup the guest can still get from the next batch, while a row kept too long
    /// wedges the transport for everyone.
    ///
    /// Returns how many rows were dropped.
    pub fn retire(&self, client: u32, handle: u32) -> usize {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let before = live.len();
        let nonstall = &self.nonstall;
        live.retain(|(r, slot)| {
            let this_event = r.client == client && r.event == handle;
            let this_client_root = client == handle && r.client == handle;
            let drop = this_event || this_client_root;
            if drop && let Some(s) = slot {
                nonstall.down(*s);
            }
            !drop
        });
        let dropped = before - live.len();
        if dropped > 0 {
            self.retired.fetch_add(dropped as u64, Ordering::Relaxed);
        }
        drop(live);
        // The kernel's callback registrations die by the same two FREE shapes.
        let mut rows = self.kernel.lock().unwrap_or_else(|e| e.into_inner());
        rows.retain(|&(c, e, slot)| {
            let gone = (c == client && e == handle) || (client == handle && c == handle);
            if gone {
                nonstall.down(slot);
            }
            !gone
        });
        dropped
    }

    /// ★ The ONE registration a host event names — `(client, event)` is the guest's own match
    /// key, so it is the lookup key here too.
    ///
    /// ⊘ **This replaces the old `batch()`**, which handed every live registration to the GSP
    /// FSM so it could post a `POST_EVENT` to all of them whenever anything might have
    /// completed. That is a wakeup with nothing necessarily behind it — the old tree measured
    /// it with a `woke_with_nothing` counter joined to its CPU copy-engine executor's
    /// `doorbells_served` (both dropped: v3 has no CPU executor and no broadcast). In v3 a
    /// wakeup is posted for exactly the registration whose **host** event fired.
    ///
    /// # TODO(P5, host events → eventfd): the pairing seam
    ///
    /// Each registration is to be paired, at registration time, with a host
    /// `NV01_EVENT_OS_EVENT` allocated through the `RmObjects` seam (`kf_host::HostRm`'s
    /// `alloc_os_event` + `open_event_fd` + `set_notification`, `V3_P2_PORT_MAP.md` §3), and
    /// the eventfd routed to the one wake path (kf-chan). When that fd fires, the worker calls
    /// this function and posts [`OsEventRegistration::post`] for the result. Nothing in this
    /// crate posts a wakeup on its own, and nothing here may ever write a guest semaphore.
    #[must_use]
    pub fn find(&self, client: u32, event: u32) -> Option<OsEventRegistration> {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        live.iter()
            .map(|(r, _)| *r)
            .find(|r| r.client == client && r.event == event)
    }

    /// The live registrations, for the end-of-run report.
    #[must_use]
    pub fn live(&self) -> Vec<OsEventRegistration> {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        live.iter().map(|(r, _)| *r).collect()
    }

    /// How many distinct `(hClient, hEvent)` pairs have ever been registered.
    #[must_use]
    pub fn registered(&self) -> u64 {
        self.registered.load(Ordering::Relaxed)
    }

    /// How many rows a `FREE` retired.
    #[must_use]
    pub fn retired(&self) -> u64 {
        self.retired.load(Ordering::Relaxed)
    }

    /// How many registrations were refused because the table was full.
    ///
    /// ★ Its healthy value is zero, and a non-zero one is not a tuning signal — it means
    /// this device is knowingly not waking someone.
    #[must_use]
    pub fn overflowed(&self) -> u64 {
        self.overflowed.load(Ordering::Relaxed)
    }

    /// How many `NV01_EVENT_OS_EVENT` allocs arrived whose params this port could not read.
    ///
    /// ⊘ Its own counter rather than a silent skip: *"the guest never registered"* and
    /// *"the guest registered in a shape we could not read"* are different findings, and
    /// only the second says this port's layout reading is wrong.
    #[must_use]
    pub fn malformed(&self) -> u64 {
        self.malformed.load(Ordering::Relaxed)
    }

    /// How many `POST_EVENT` messages have been put on the wire.
    #[must_use]
    pub fn posted(&self) -> u64 {
        self.posted.load(Ordering::Relaxed)
    }

    /// How many batches were delivered (each one raises exactly one interrupt).
    #[must_use]
    pub fn batches(&self) -> u64 {
        self.batches.load(Ordering::Relaxed)
    }

    /// ★★★ How many delivery attempts the flow-control gate refused.
    ///
    /// Healthy in steady state — it is what bounds the shared ring to one outstanding
    /// batch. **The number to read when delivery stops**: a large `gated` beside
    /// `batches == 1` says the guest never wrote `IRQSCLR`, i.e. the opener never fired and
    /// the gate is stuck, which no test in this repository can observe because `cap1`
    /// contains zero `IRQSCLR` writes.
    #[must_use]
    pub fn gated(&self) -> u64 {
        self.gated.load(Ordering::Relaxed)
    }

    /// How many attempts were made before the guest drained `GSP_INIT_DONE`.
    #[must_use]
    pub fn not_running(&self) -> u64 {
        self.not_running.load(Ordering::Relaxed)
    }

    /// How many attempts posted nothing at all because the ring refused the first message.
    #[must_use]
    pub fn failed(&self) -> u64 {
        self.failed.load(Ordering::Relaxed)
    }

    /// Record what one [`kf_gsp::GspFsm::deliver_events`] call did.
    ///
    /// ⊘ `match` rather than a pair of numeric arguments, for
    /// [`crate::faultbuffer::FaultBufferLog::note`]'s reason: which counter an outcome lands
    /// on is a property OF THE OUTCOME, so a caller cannot give the wrong one.
    pub fn note(&self, outcome: &kf_gsp::EventDelivery) {
        match outcome {
            kf_gsp::EventDelivery::NoneRegistered => {}
            kf_gsp::EventDelivery::Gated => {
                self.gated.fetch_add(1, Ordering::Relaxed);
            }
            kf_gsp::EventDelivery::NotRunning => {
                self.not_running.fetch_add(1, Ordering::Relaxed);
            }
            kf_gsp::EventDelivery::Delivered { posted, .. } => {
                self.posted.fetch_add(*posted as u64, Ordering::Relaxed);
                self.batches.fetch_add(1, Ordering::Relaxed);
            }
            kf_gsp::EventDelivery::Failed { .. } => {
                self.failed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn note_malformed(&self) {
        self.malformed.fetch_add(1, Ordering::Relaxed);
    }
}

/// The front-seat observer: writes down `NV01_EVENT_OS_EVENT` registrations and retires
/// them on `FREE`. It **cannot** answer — `observe` has no return value.
#[derive(Debug, Clone)]
pub struct OsEventRecorder {
    driver: DriverAbiTable,
    log: OsEventLog,
}

impl OsEventRecorder {
    /// Build a recorder writing into `log`.
    #[must_use]
    pub fn new(driver: DriverAbiTable, log: OsEventLog) -> OsEventRecorder {
        OsEventRecorder { driver, log }
    }
}

impl CommandObserver for OsEventRecorder {
    fn observe(&mut self, cmd: &RpcCommand) {
        match cmd.function {
            RpcFunction::RmAlloc => {
                // ⚠⚠ **`wire_body()`, NOT `payload`** — and this is not a style choice, it
                // is the difference between this module working and registering nothing.
                // `rpcRmApiAlloc_GSP` sets the envelope's `length` to plain
                // `sizeof(rpc_gsp_rm_alloc_v03_00)` (`ogkm-580: rpc.c:11196-11199`), whose
                // last member is a **flexible** `NvU8 params[]`, so the declared length
                // stops exactly where the params begin and `RpcCommand::payload` is 32
                // bytes with nothing after it. The params still arrive — whole elements are
                // copied into the queue — which is what `delivered` carries and what
                // `kayfabe_rmrpc::translate_alloc` reads for the same reason.
                //
                // ⊘ Had this read `payload`, every registration would have counted as
                // `malformed` and the whole plane would have been silently dead on a live
                // boot, with all six unit tests below still green if they had been built on
                // the same mistake. `RpcCommand::wire_body`'s own docs are where this is
                // argued; this comment exists because the wrong call typechecks.
                let body = cmd.wire_body();
                let Ok(h) = self.driver.decode_rpc_alloc(body) else {
                    return;
                };
                let kernel_callback = h.class == NV01_EVENT_KERNEL_CALLBACK
                    || h.class == NV01_EVENT_KERNEL_CALLBACK_EX;
                if h.class != NV01_EVENT_OS_EVENT && !kernel_callback {
                    return;
                }
                // ⚠ `paramsSize` is the guest's assertion about its own message, so it is
                // bounded by what actually arrived before anything is sliced with it — the
                // same pair `kayfabe_rmrpc::translate_alloc` bounds, for the same reason.
                // ⊘ And these bytes are NOT covered by the queue checksum (the guest sums
                // `msgLen` only), so they are hostile bytes in a bounded window — which is
                // exactly what `AllocParams::NoDeclaredFacts` already assumes, and why the
                // one field read out of them is a plain `u32` nothing dereferences.
                let params = body
                    .get(h.params_at..)
                    .and_then(|tail| tail.get(..h.params_size as usize))
                    .unwrap_or(&[]);
                let Some(notify_index) = params
                    .get(NOTIFY_INDEX_AT..NOTIFY_INDEX_AT + 4)
                    .and_then(|b| <[u8; 4]>::try_from(b).ok())
                    .map(u32::from_le_bytes)
                else {
                    // ⊘ Counted and NOT registered. A registration with a fabricated
                    // `notifyIndex` would post a wakeup the guest's own notifier cannot
                    // attribute — worse than no registration, which merely leaves the
                    // waiter where it already was.
                    self.log.note_malformed();
                    return;
                };
                // ★ 2026-10-08 (§X): a NON-STALL registration is the guest's subscription to an
                // engine's (or the host's) non-stall edges, resolved at the guest's version.
                let slot =
                    kf_abi::eventnotify::nonstall_slot(self.driver.driver_version(), notify_index);
                if kernel_callback {
                    // The guest kernel's own waiter: an arm, never a POST_EVENT row.
                    if let Some(s) = slot {
                        self.log.register_kernel_nonstall(h.client, h.handle, s);
                    }
                    return;
                }
                self.log.register_slot(
                    OsEventRegistration {
                        client: h.client,
                        event: h.handle,
                        notify_index,
                    },
                    slot,
                );
            }
            // ★★★ The retire path. `rpc_free_v03_00` IS `NVOS00_PARAMETERS`
            // (`hRoot = hClient`, `hObjectOld = hObject`), which is why the ordinary free
            // decoder applies verbatim — see `kayfabe_rmrpc::translate_free`.
            //
            // ⊘ It does NOT gate on "was this handle one of ours": the guest frees far more
            // than events, and `retire` is a no-op for a handle that names no row. Gating
            // would mean holding a second copy of the object model here.
            // ★ 2026-10-08 (§X): the guest's RM (re)starts — every earlier registration is dead.
            RpcFunction::SetGuestSystemInfo => self.log.clear(),
            RpcFunction::Free => {
                let Ok(f) = self.driver.decode_free(&cmd.payload) else {
                    return;
                };
                self.log.retire(f.client, f.handle);
            }
            _ => {}
        }
        // ⊘ Nothing is returned, and nothing CAN be.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One `GSP_RM_ALLOC` payload, in the shape `DriverAbiTable::decode_rpc_alloc` reads:
    /// `hClient` @ +0, `hParent` @ +4, `hObject` @ +8, `hClass` @ +12, `paramsSize` @ +20,
    /// `flags` @ +24, `params[]` @ +32.
    fn alloc_rpc(client: u32, handle: u32, class: u32, params: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; 32 + params.len()];
        b[0..4].copy_from_slice(&client.to_le_bytes());
        b[4..8].copy_from_slice(&client.to_le_bytes());
        b[8..12].copy_from_slice(&handle.to_le_bytes());
        b[12..16].copy_from_slice(&class.to_le_bytes());
        b[20..24].copy_from_slice(&(params.len() as u32).to_le_bytes());
        b[32..].copy_from_slice(params);
        b
    }

    fn recorder(log: &OsEventLog) -> OsEventRecorder {
        let abi = kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER)
            .expect("the bench driver table");
        OsEventRecorder::new(*abi, log.clone())
    }

    /// ★★★ Built the way the TRANSPORT builds one, which is the half a hand-rolled fixture
    /// gets wrong: an alloc's declared `payload` stops at the 32-byte header and its
    /// `params[]` arrive only in `delivered`. A fixture that put the params in `payload`
    /// would be green against a recorder reading the wrong buffer — the defect this
    /// distinction exists to catch.
    fn observe(rec: &mut OsEventRecorder, function: RpcFunction, wire: Vec<u8>) {
        let declared = match function {
            // `rpcWriteCommonHeader(…, sizeof(rpc_gsp_rm_alloc_v03_00))` — 32, excluding
            // the flexible params tail.
            RpcFunction::RmAlloc => 32.min(wire.len()),
            _ => wire.len(),
        };
        rec.observe(&RpcCommand {
            function,
            code: 0,
            sequence: 0,
            payload: wire[..declared].to_vec(),
            elements: 1,
            delivered: wire,
        });
    }

    /// ★★★ The registration this whole module exists for, decoded off a **real** wire
    /// image: the guest's own 24-byte `NV0005_ALLOC_PARAMETERS`, `paramsSize = 0x18` as
    /// measured in `w209`.
    ///
    /// ⊘ And the negative half, which is the security-relevant one: `data` @ +16 — the
    /// `NvP64` guest-kernel callback pointer — is filled with a recognisable value, and the
    /// registration must not carry a byte of it anywhere. A test that only checked
    /// `notify_index` would pass on a decoder that had also read the pointer.
    #[test]
    fn a_real_wire_image_registers_the_pair_and_never_touches_the_pointer() {
        const POISON: u64 = 0xdead_beef_feed_face;
        let mut params = [0u8; NV0005_ALLOC_PARAMETERS_SIZE];
        params[0..4].copy_from_slice(&0xc1d0_000cu32.to_le_bytes()); // hParentClient
        params[8..12].copy_from_slice(&NV01_EVENT_OS_EVENT.to_le_bytes()); // hClass
        params[NOTIFY_INDEX_AT..NOTIFY_INDEX_AT + 4].copy_from_slice(&35u32.to_le_bytes());
        params[16..24].copy_from_slice(&POISON.to_le_bytes()); // data — the pointer
        let log = OsEventLog::new();
        let mut rec = recorder(&log);
        observe(
            &mut rec,
            RpcFunction::RmAlloc,
            alloc_rpc(0xc1d0_000c, 0x5c00_0079, NV01_EVENT_OS_EVENT, &params),
        );

        assert_eq!(
            log.live(),
            vec![OsEventRegistration {
                client: 0xc1d0_000c,
                event: 0x5c00_0079,
                notify_index: 35,
            }],
            "the (hClient, hEvent) pair comes from the HEADER and notifyIndex from +12"
        );
        let posted = log.live()[0].post().encode();
        let poison = POISON.to_le_bytes();
        assert!(
            !posted
                .windows(4)
                .any(|w| w == &poison[..4] || w == &poison[4..]),
            "⊘ no half of the guest-kernel callback pointer reached the wire — nothing in \
             this tree dereferences one, and the way that stays true is that no decoder \
             exists to hand one up"
        );
    }

    /// ⊘ A params window too short to hold `notifyIndex` is COUNTED, not guessed at. A
    /// fabricated index would post a wakeup the guest's own notifier cannot attribute.
    #[test]
    fn a_short_params_window_is_malformed_rather_than_defaulted() {
        let log = OsEventLog::new();
        let mut rec = recorder(&log);
        observe(
            &mut rec,
            RpcFunction::RmAlloc,
            alloc_rpc(0xc1d0_000c, 0x5c00_0079, NV01_EVENT_OS_EVENT, &[0u8; 8]),
        );
        assert_eq!(log.live(), vec![], "nothing registered");
        assert_eq!(log.malformed(), 1, "and it said so");
    }

    /// A `FREE` seen on the wire retires the row — the observer's half of the C's
    /// three-times-reproduced fix.
    #[test]
    fn a_free_on_the_wire_retires_the_registration() {
        let mut params = [0u8; NV0005_ALLOC_PARAMETERS_SIZE];
        params[NOTIFY_INDEX_AT..NOTIFY_INDEX_AT + 4].copy_from_slice(&35u32.to_le_bytes());
        let log = OsEventLog::new();
        let mut rec = recorder(&log);
        observe(
            &mut rec,
            RpcFunction::RmAlloc,
            alloc_rpc(0xc1d0_000c, 0x5c00_0079, NV01_EVENT_OS_EVENT, &params),
        );
        assert_eq!(log.live().len(), 1);
        // `rpc_free_v03_00` IS `NVOS00_PARAMETERS`: hRoot @ +0, hObjectParent @ +4,
        // hObjectOld @ +8.
        let mut free = vec![0u8; 16];
        free[0..4].copy_from_slice(&0xc1d0_000cu32.to_le_bytes());
        free[8..12].copy_from_slice(&0x5c00_0079u32.to_le_bytes());
        observe(&mut rec, RpcFunction::Free, free);
        assert_eq!(log.live(), vec![], "the row is gone");
        assert_eq!(log.retired(), 1);
    }

    /// ⊘ Another event class must NOT land in this registry: `NV01_EVENT_KERNEL_CALLBACK_EX`
    /// (`0x7e`) is the guest KERNEL's own callback event, and `osNotifyEvent` on it would be
    /// this device calling into guest-kernel state it has no business waking.
    #[test]
    fn only_the_os_event_class_registers() {
        let mut params = [0u8; NV0005_ALLOC_PARAMETERS_SIZE];
        params[NOTIFY_INDEX_AT..NOTIFY_INDEX_AT + 4].copy_from_slice(&35u32.to_le_bytes());
        let log = OsEventLog::new();
        let mut rec = recorder(&log);
        observe(
            &mut rec,
            RpcFunction::RmAlloc,
            alloc_rpc(0xc1d0_000c, 0x5c00_007e, 0x7e, &params),
        );
        assert_eq!(log.live(), vec![]);
        assert_eq!(log.registered(), 0);
        assert_eq!(log.malformed(), 0, "declined by CLASS, not by shape");
    }

    #[test]
    fn a_registration_is_deduped_on_the_client_event_pair() {
        let log = OsEventLog::new();
        let r = OsEventRegistration {
            client: 0xc1d0_000c,
            event: 0x5c00_0079,
            notify_index: 35,
        };
        assert!(log.register(r));
        assert!(!log.register(r), "the same pair must not register twice");
        assert!(
            log.register(OsEventRegistration {
                event: 0x5c00_007a,
                ..r
            }),
            "a different hEvent is a different registration"
        );
        assert_eq!(log.registered(), 2);
        assert_eq!(log.live().len(), 2);
    }

    /// ★★★ The C's reproduction, as a property: a freed event stops being posted to.
    #[test]
    fn a_freed_event_is_retired() {
        let log = OsEventLog::new();
        for e in [0x5c00_0079u32, 0x5c00_007a, 0x5c00_007b] {
            assert!(log.register(OsEventRegistration {
                client: 0xc1d0_000c,
                event: e,
                notify_index: 35,
            }));
        }
        assert_eq!(log.retire(0xc1d0_000c, 0x5c00_007a), 1);
        assert_eq!(log.live().len(), 2);
        // ⊘ A free in ANOTHER namespace must not touch these rows.
        assert_eq!(log.retire(0xc1d0_000d, 0x5c00_0079), 0);
        assert_eq!(log.live().len(), 2);
        assert_eq!(log.retired(), 1);
    }

    /// Freeing the client ROOT (`fClient == fObj`) tears down every row it owns.
    #[test]
    fn freeing_the_client_root_retires_all_of_its_events() {
        let log = OsEventLog::new();
        for e in [0x5c00_0079u32, 0x5c00_007a] {
            log.register(OsEventRegistration {
                client: 0xc1d0_000c,
                event: e,
                notify_index: 35,
            });
        }
        log.register(OsEventRegistration {
            client: 0xc1d0_000d,
            event: 0x5c00_0080,
            notify_index: 35,
        });
        assert_eq!(log.retire(0xc1d0_000c, 0xc1d0_000c), 2);
        let left = log.live();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].client, 0xc1d0_000d, "the other client is untouched");
    }

    /// ★ 2026-10-08 (§X): a NON-STALL registration arms its slot, its `FREE` disarms it; a stall
    /// registration on the same index arms nothing.
    #[test]
    fn a_nonstall_registration_arms_its_slot_until_freed() {
        use kf_abi::eventnotify::{NONSTALL_SLOT_FIFO_EVENT_MTHD, NV01_EVENT_NONSTALL_INTR};
        let log = OsEventLog::new();
        let arms = log.nonstall_arms();
        let mut rec = recorder(&log);
        let mut params = [0u8; NV0005_ALLOC_PARAMETERS_SIZE];
        params[NOTIFY_INDEX_AT..NOTIFY_INDEX_AT + 4].copy_from_slice(&35u32.to_le_bytes());
        observe(
            &mut rec,
            RpcFunction::RmAlloc,
            alloc_rpc(0xc1d0_000c, 0x5c00_0079, NV01_EVENT_OS_EVENT, &params),
        );
        assert!(!arms.armed(NONSTALL_SLOT_FIFO_EVENT_MTHD), "a stall event");
        params[NOTIFY_INDEX_AT..NOTIFY_INDEX_AT + 4]
            .copy_from_slice(&(35u32 | NV01_EVENT_NONSTALL_INTR).to_le_bytes());
        for ev in [0x5c00_007a, 0x5c00_007b] {
            observe(
                &mut rec,
                RpcFunction::RmAlloc,
                alloc_rpc(0xc1d0_000c, ev, NV01_EVENT_OS_EVENT, &params),
            );
        }
        assert_eq!(arms.count(NONSTALL_SLOT_FIFO_EVENT_MTHD), 2);
        assert_eq!(log.retire(0xc1d0_000c, 0x5c00_007a), 1);
        assert!(
            arms.armed(NONSTALL_SLOT_FIFO_EVENT_MTHD),
            "one subscription left"
        );
        assert_eq!(log.retire(0xc1d0_000c, 0xc1d0_000c), 2, "the root free");
        assert!(!arms.armed(NONSTALL_SLOT_FIFO_EVENT_MTHD));
        assert!(!arms.armed(usize::MAX), "a hostile slot is never armed");
    }

    fn nonstall_params(notify_index: u32) -> [u8; NV0005_ALLOC_PARAMETERS_SIZE] {
        let mut params = [0u8; NV0005_ALLOC_PARAMETERS_SIZE];
        params[NOTIFY_INDEX_AT..NOTIFY_INDEX_AT + 4].copy_from_slice(&notify_index.to_le_bytes());
        params
    }

    fn free_rpc(client: u32, handle: u32) -> Vec<u8> {
        let mut free = vec![0u8; 16];
        free[0..4].copy_from_slice(&client.to_le_bytes());
        free[8..12].copy_from_slice(&handle.to_le_bytes());
        free
    }

    /// ★ The review's finding 1: the guest KERNEL's non-stall waiters (`NV01_EVENT_KERNEL_CALLBACK_EX`
    /// for CeUtils and semaphore surfaces, `NV01_EVENT_KERNEL_CALLBACK`) arm their slot — and are
    /// never put in the POST_EVENT registry — and their FREE (event or client root) disarms it.
    #[test]
    fn a_kernel_callback_nonstall_registration_arms_its_slot_until_freed() {
        use kf_abi::eventnotify::{
            NONSTALL_SLOT_FIFO_EVENT_MTHD, NV01_EVENT_NONSTALL_INTR, nonstall_slot_ce,
        };
        let ce2 = 25u32; // NV2080_NOTIFIERS_CE2 at the bench driver
        assert_eq!(
            kf_abi::eventnotify::nonstall_slot(
                kf_abi::versions::BENCH_DRIVER,
                ce2 | NV01_EVENT_NONSTALL_INTR
            ),
            nonstall_slot_ce(2)
        );
        let log = OsEventLog::new();
        let arms = log.nonstall_arms();
        let mut rec = recorder(&log);
        let fifo = NONSTALL_SLOT_FIFO_EVENT_MTHD;
        // 0x7e, CeUtils' shape: FIFO_EVENT_MTHD | NONSTALL (mem_utils.c:1905-1906).
        observe(
            &mut rec,
            RpcFunction::RmAlloc,
            alloc_rpc(
                0xc1e0_0001,
                0x5c00_0001,
                NV01_EVENT_KERNEL_CALLBACK_EX,
                &nonstall_params(35 | NV01_EVENT_NONSTALL_INTR),
            ),
        );
        // 0x78 on CE2, in another client.
        observe(
            &mut rec,
            RpcFunction::RmAlloc,
            alloc_rpc(
                0xc1e0_0002,
                0x5c00_0002,
                NV01_EVENT_KERNEL_CALLBACK,
                &nonstall_params(ce2 | NV01_EVENT_NONSTALL_INTR),
            ),
        );
        // A kernel callback WITHOUT the non-stall flag arms nothing.
        observe(
            &mut rec,
            RpcFunction::RmAlloc,
            alloc_rpc(
                0xc1e0_0001,
                0x5c00_0003,
                NV01_EVENT_KERNEL_CALLBACK_EX,
                &nonstall_params(35),
            ),
        );
        assert!(
            arms.armed(fifo),
            "the kernel's FIFO_EVENT_MTHD waiter is armed"
        );
        assert!(arms.armed(nonstall_slot_ce(2).unwrap()));
        assert_eq!(arms.count(fifo), 1);
        assert_eq!(arms.kernel_registered.load(Ordering::Relaxed), 2);
        assert_eq!(log.live(), vec![], "never a POST_EVENT row");
        // FREE(hClient, hParent, hEvent) retires the 0x7e …
        observe(
            &mut rec,
            RpcFunction::Free,
            free_rpc(0xc1e0_0001, 0x5c00_0001),
        );
        assert_eq!(arms.count(fifo), 0, "freed: back to 0");
        // … and FREE(hClient, NULL, hClient) — the client root — the 0x78.
        observe(
            &mut rec,
            RpcFunction::Free,
            free_rpc(0xc1e0_0002, 0xc1e0_0002),
        );
        assert!(
            !arms.armed(nonstall_slot_ce(2).unwrap()),
            "freed: back to 0"
        );
    }

    /// ★ The review's finding 2: a reused `(hClient, hEvent)` on ANOTHER notifier moves the arm
    /// (the old slot is disarmed, the new one armed) in both tables.
    #[test]
    fn a_reused_pair_on_another_notifier_moves_its_arm() {
        let log = OsEventLog::new();
        let arms = log.nonstall_arms();
        let reg = |notify_index| OsEventRegistration {
            client: 7,
            event: 0x70,
            notify_index,
        };
        assert!(log.register_slot(reg(35), Some(0)));
        assert!(!log.register_slot(reg(35), Some(0)), "identical: a no-op");
        assert_eq!(arms.count(0), 1);
        assert!(log.register_slot(reg(25), Some(4)), "replaced");
        assert_eq!((arms.count(0), arms.count(4)), (0, 1));
        assert_eq!(log.live(), vec![reg(25)]);
        assert!(log.register_slot(reg(0), None), "now not a non-stall event");
        assert_eq!(arms.count(4), 0);
        // The kernel table likewise.
        log.register_kernel_nonstall(9, 0x90, 0);
        log.register_kernel_nonstall(9, 0x90, 1);
        assert_eq!((arms.count(0), arms.count(1)), (0, 1));
        assert_eq!(
            log.retire(9, 0x90),
            0,
            "kernel rows are not POST_EVENT rows"
        );
        assert_eq!(arms.count(1), 0);
    }

    /// ★ The review's finding 2, second half: fn 1 (the guest's RM starting again) drops every
    /// row and every arm, sticky included — seen by the observer, in RPC order.
    #[test]
    fn the_guest_rm_starting_again_clears_every_registration_and_arm() {
        let log = OsEventLog::new();
        let arms = log.nonstall_arms();
        let mut rec = recorder(&log);
        log.register_slot(
            OsEventRegistration {
                client: 1,
                event: 2,
                notify_index: 35,
            },
            Some(0),
        );
        log.register_kernel_nonstall(3, 4, 5);
        for i in 0..KERNEL_NONSTALL_MAX as u32 + 1 {
            log.register_kernel_nonstall(10, 0x100 + i, 6);
        }
        assert_eq!(arms.sticky(), 1 << 6, "the overflow stuck");
        observe(&mut rec, RpcFunction::SetGuestSystemInfo, vec![0u8; 8]);
        assert_eq!(log.live(), vec![]);
        for s in 0..kf_abi::eventnotify::NONSTALL_SLOTS {
            assert!(!arms.armed(s), "slot {s} still armed after fn 1");
        }
        assert_eq!(arms.clears.load(Ordering::Relaxed), 1);
        // A registration after fn 1 arms again.
        log.register_kernel_nonstall(3, 4, 5);
        assert!(arms.armed(5));
    }

    /// ⊘ A non-stall registration past the bound is not remembered — so its slot sticks armed
    /// (a missed wake is the bug; a spurious one is not).
    #[test]
    fn an_overflowed_nonstall_registration_sticks_its_slot() {
        let log = OsEventLog::new();
        for i in 0..OS_EVENT_MAX as u32 {
            log.register(OsEventRegistration {
                client: 1,
                event: 0x1000 + i,
                notify_index: 0,
            });
        }
        let reg = OsEventRegistration {
            client: 1,
            event: 0x9000,
            notify_index: 0,
        };
        assert!(!log.register_slot(reg, Some(3)));
        assert!(log.nonstall_arms().armed(3));
        assert_eq!(log.nonstall_arms().sticky(), 1 << 3);
        assert_eq!(log.retire(1, 1), OS_EVENT_MAX);
        assert!(
            log.nonstall_arms().armed(3),
            "still armed: its free was never seen"
        );
    }

    /// The table refuses to remember past its bound, and says so — it never evicts.
    #[test]
    fn the_registry_refuses_rather_than_evicting() {
        let log = OsEventLog::new();
        for i in 0..(OS_EVENT_MAX as u32 + 4) {
            log.register(OsEventRegistration {
                client: 1,
                event: 0x1000 + i,
                notify_index: 0,
            });
        }
        assert_eq!(log.live().len(), OS_EVENT_MAX);
        assert_eq!(log.overflowed(), 4);
        assert_eq!(
            log.live()[0].event,
            0x1000,
            "the FIRST registration is still there — a full table refuses, it does not \
             evict a waiter that is still waiting"
        );
    }
}
