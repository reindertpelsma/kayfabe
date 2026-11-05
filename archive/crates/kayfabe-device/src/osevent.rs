//! ★★★★★ §16.76 — **the os-event registry**: which `(hClient, hEvent, notifyIndex)` this
//! device may post a wakeup to, and when it stops being allowed to.
//!
//! # Why a registry exists at all
//!
//! `kayfabe_gsp::GspFsm::deliver_events` posts one
//! `NV_VGPU_MSG_EVENT_POST_EVENT` per registered event, and the guest's `_kgspRpcPostEvent`
//! resolves it with `CliGetEventInfo(hClient, hEvent)` before calling `osNotifyEvent`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:497-535`). So the pair is not
//! decoration — it **is** the address of the wakeup, and the only place it is ever stated
//! is the `GSP_RM_ALLOC` of an `NV01_EVENT_OS_EVENT`.
//!
//! `[measured 2026-08-10, boot w209_ffc80f8_ctl, rev ffc80f8]`
//! (`traces/guest_boots/run_w209_ffc80f8_ctl_probe.log`) libcuda registers **seven** of
//! them under its own client `0xc1d0000c`, and every one was refused `status=0x00000056`
//! because `kayfabe_abi::versions::DriverAbiTable::alloc_params` had no arm for the class.
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
//! [`kayfabe_gsp::CommandObserver`] has no return value, so *"this link changes no reply
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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use kayfabe_abi::generated::classes::NV01_EVENT_OS_EVENT;
use kayfabe_abi::postevent::PostEvent;
use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_gsp::{CommandObserver, RpcCommand, RpcFunction};

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

/// The shared registry. Cloneable so the plane and the chain link hold the same one.
#[derive(Debug, Clone, Default)]
pub struct OsEventLog {
    live: Arc<Mutex<Vec<OsEventRegistration>>>,
    registered: Arc<AtomicU64>,
    retired: Arc<AtomicU64>,
    overflowed: Arc<AtomicU64>,
    malformed: Arc<AtomicU64>,
    posted: Arc<AtomicU64>,
    batches: Arc<AtomicU64>,
    gated: Arc<AtomicU64>,
    not_running: Arc<AtomicU64>,
    failed: Arc<AtomicU64>,
    woke_with_nothing: Arc<AtomicU64>,
    last_join: Arc<Mutex<JoinPoint>>,
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
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if live
            .iter()
            .any(|r| r.client == reg.client && r.event == reg.event)
        {
            return false;
        }
        if live.len() >= OS_EVENT_MAX {
            self.overflowed.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        live.push(reg);
        self.registered.fetch_add(1, Ordering::Relaxed);
        true
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
        live.retain(|r| {
            let this_event = r.client == client && r.event == handle;
            let this_client_root = client == handle && r.client == handle;
            !(this_event || this_client_root)
        });
        let dropped = before - live.len();
        if dropped > 0 {
            self.retired.fetch_add(dropped as u64, Ordering::Relaxed);
        }
        dropped
    }

    /// The wakeup messages for every live registration, in registration order.
    ///
    /// ⊘ A snapshot by value, so the caller posts without holding this lock — the poster is
    /// the GSP FSM, reached under the plane's own mutex, and a lock taken inside that one
    /// would be a new edge in the port's lock order (`unranked_locks.rs`).
    #[must_use]
    pub fn batch(&self) -> Vec<PostEvent> {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        live.iter().map(OsEventRegistration::post).collect()
    }

    /// The live registrations, for the end-of-run report.
    #[must_use]
    pub fn live(&self) -> Vec<OsEventRegistration> {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        live.clone()
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

    /// Record what one [`kayfabe_gsp::GspFsm::deliver_events`] call did.
    ///
    /// ⊘ `match` rather than a pair of numeric arguments, for
    /// [`crate::faultbuffer::FaultBufferLog::note`]'s reason: which counter an outcome lands
    /// on is a property OF THE OUTCOME, so a caller cannot give the wrong one.
    pub fn note(&self, outcome: &kayfabe_gsp::EventDelivery) {
        match outcome {
            kayfabe_gsp::EventDelivery::NoneRegistered => {}
            kayfabe_gsp::EventDelivery::Gated => {
                self.gated.fetch_add(1, Ordering::Relaxed);
            }
            kayfabe_gsp::EventDelivery::NotRunning => {
                self.not_running.fetch_add(1, Ordering::Relaxed);
            }
            kayfabe_gsp::EventDelivery::Delivered { posted, .. } => {
                self.posted.fetch_add(*posted as u64, Ordering::Relaxed);
                self.batches.fetch_add(1, Ordering::Relaxed);
            }
            kayfabe_gsp::EventDelivery::Failed { .. } => {
                self.failed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// ★★★★★ §16.76 — **record the JOIN**: what this batch was announced *on top of*.
    ///
    /// # ⊘⊘ A CORRECTION, ATTRIBUTED, because the wrong version of this comment was one
    /// edit away from becoming a semaphore writer
    ///
    /// This rung's coordinator first prescribed the join as *"something must WRITE the
    /// release semaphore before we signal"*, citing the C's own delivery comment (`write
    /// sema THEN signal, so the payload is already visible when the guest re-checks`,
    /// `C:4357-4361`). **The owner refuted it and is right.** That is the C's **FORGERY
    /// PATH**, not the architecture, and this port's own standing limit says so in as many
    /// words: *"there is NO C oracle for the completion plane — the C FORGES completions"*,
    /// and *"the completion plane splits THREE ways: emulated-and-executed ≠ forged"*
    /// (`../nvkvm-rs/docs/design/c_rust_trace_differential.md`,
    /// `mem: completion_plane_splits_three_ways`).
    ///
    /// ⊘ **This VMM must never write a guest-userspace semaphore.** The data plane is
    /// passthrough: the guest's buffers are pinned into the **host GPU's** address space,
    /// so when the guest's channel really executes, the host GPU DMAs the release semaphore
    /// straight into guest RAM and nothing here is in the path. Writing it ourselves would
    /// be forging a completion, which is a regression against the design and not a rung.
    ///
    /// # What the join therefore measures — and it is NOT a semaphore
    ///
    /// Exactly one thing: **did any of the guest's work actually execute** between this
    /// batch and the last one. `served` is
    /// [`crate::plane::Counters::doorbells_served`] — every doorbell this device either
    /// forwarded to the host (where the GPU executes and DMAs) or served on its own CPU
    /// copy-engine executor. If that number has not moved, the notification is honest and
    /// there is simply nothing behind it: libcuda wakes, re-reads a semaphore the host
    /// never DMA'd into, and blocks again.
    ///
    /// ⊘ From outside, that is **byte-identical** to never having woken it — `cuCtxCreate`
    /// does not return either way. [`Self::woke_with_nothing`] is the discriminator, and it
    /// is the whole reason this function exists.
    ///
    /// `served` is a **running total**; the delta against the previous batch is scored.
    pub fn note_join(&self, registered: usize, posted: usize, served: u64, forwarded: u64) {
        let mut last = self.last_join.lock().unwrap_or_else(|e| e.into_inner());
        let advanced = served.saturating_sub(last.served);
        if advanced == 0 {
            self.woke_with_nothing.fetch_add(1, Ordering::Relaxed);
        }
        *last = JoinPoint {
            registered,
            posted,
            served,
            forwarded,
            advanced,
        };
    }

    /// ★★★ How many announced batches carried **no new completion** — the wake-with-nothing
    /// count. See [`Self::note_join`].
    ///
    /// ⚠ Its healthy value is zero, and on this port today it is **expected to equal**
    /// [`Self::batches`]: `[measured 2026-08-10, boot w209_ffc80f8]` the guest's channel is
    /// refused before submission (`CE-SUBMIT → REFUSED BEFORE SUBMISSION`) and the isolate
    /// plane defaults to `Stillborn`, so nothing of the guest's has executed on the host at
    /// all. That is a **diagnosis, not a defect in the notification plane** — it names the
    /// next rung (get the guest's channel forwarded and executed) rather than blaming the
    /// piece that was built, and emphatically not a call to fabricate the payload.
    #[must_use]
    pub fn woke_with_nothing(&self) -> u64 {
        self.woke_with_nothing.load(Ordering::Relaxed)
    }

    /// The last join point, for the end-of-run report.
    #[must_use]
    pub fn last_join(&self) -> JoinPoint {
        *self.last_join.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn note_malformed(&self) {
        self.malformed.fetch_add(1, Ordering::Relaxed);
    }
}

/// What one announced batch was announced *on top of* — see [`OsEventLog::note_join`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JoinPoint {
    /// How many os-events were registered at that instant.
    pub registered: usize,
    /// How many `POST_EVENT` messages the batch actually landed.
    pub posted: usize,
    /// The running count of doorbells this device **served** — forwarded to the host, or
    /// executed on its own CPU copy engine. The only honest proxy for *"the guest's work
    /// ran, so the host GPU has something to DMA"*.
    pub served: u64,
    /// Of those, how many were **forwarded** — the passthrough path, where the host GPU
    /// writes the release semaphore into guest RAM and this device is not involved.
    pub forwarded: u64,
    /// How many of those were new since the previous batch. ⊘ **Zero is the finding**: a
    /// wakeup with nothing behind it.
    pub advanced: u64,
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
                if h.class != NV01_EVENT_OS_EVENT {
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
                self.log.register(OsEventRegistration {
                    client: h.client,
                    event: h.handle,
                    notify_index,
                });
            }
            // ★★★ The retire path. `rpc_free_v03_00` IS `NVOS00_PARAMETERS`
            // (`hRoot = hClient`, `hObjectOld = hObject`), which is why the ordinary free
            // decoder applies verbatim — see `kayfabe_rmrpc::translate_free`.
            //
            // ⊘ It does NOT gate on "was this handle one of ours": the guest frees far more
            // than events, and `retire` is a no-op for a handle that names no row. Gating
            // would mean holding a second copy of the object model here.
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
        let abi = kayfabe_abi::versions::table_for(kayfabe_abi::versions::BENCH_DRIVER)
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
        let posted = log.batch()[0].encode();
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
        assert_eq!(log.batch().len(), 2);
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
        assert_eq!(log.batch().len(), 2);
        // ⊘ A free in ANOTHER namespace must not touch these rows.
        assert_eq!(log.retire(0xc1d0_000d, 0x5c00_0079), 0);
        assert_eq!(log.batch().len(), 2);
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
