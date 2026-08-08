//! The `GSP_RM_CONTROL` request **and** reply body for
//! `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION` (`0x20800301`) — twenty bytes, and the first
//! control this port serves that is a **verb on the event plane** rather than a description
//! of silicon.
//!
//! ## ★★★ Why refusing it stops the boot, measured across two boots one commit apart
//!
//! `[measured]` `docs/design/boot_measured_2026_08_01.md` §6 and §7: boot `alloc1`
//! (rev `2ced035`) refused every object allocation and died in `kbusInitBar2_HAL` with a
//! null heap; boot `alloc2` (rev `a6412c0`) served **every** allocation — zero
//! `rpcRmApiAlloc_GSP` lines, zero `rpcRmApiFree_GSP` lines — and died in exactly the same
//! place. ⊘ That differential **refutes** §3's "refusing object allocation starves the
//! heap": allocation was never the dependency.
//!
//! What is left is this control, and the chain is short because there is only one call
//! site. `memmgrRegisterSuspendCallbacks` (`ogkm-580:
//! src/nvidia/src/kernel/gpu/mem_mgr/mem_mgr.c:601-634`) allocates an
//! `NV01_EVENT_KERNEL_CALLBACK_EX` and then issues this control under
//! **`NV_ASSERT_OK_OR_RETURN`** at `:625`. Its one caller is `memmgrStateInitLocked_IMPL`
//! at `:777`, under `NV_ASSERT_OK_OR_GOTO(status, …, failed)`.
//!
//! ## ★★ The heap is created and then TORN DOWN — the mechanism is rollback, not absence
//!
//! ⚠ Worth stating precisely, because the obvious reading is wrong. `memmgrCreateHeap` runs
//! at `mem_mgr.c:684`, **ninety lines before** the control is issued, and it sets
//! `pMemoryManager->pHeap` (`:1113`). So at the moment we refuse, the heap exists.
//!
//! The `failed:` label is what destroys it: `memmgrStateInitLocked_IMPL` calls
//! `memmgrStateDestroy`, which does `objDelete(pHeap); pMemoryManager->pHeap = NULL;`
//! (`mem_mgr.c:963-975`) — a WAR the source itself flags as `TODO: Bug 3482892: Need a way
//! to roll back StateInit steps`. `memmgrGetDeviceSuballocator` then returns that NULL
//! (`mem_mgr.c:1176-1190`), `_memdescSetOwnedByCurrentDeviceFlag` asserts on it
//! (`mem_desc.c:149-152` → `NV_ERR_INVALID_STATE`), and BAR2 init fails.
//!
//! ⇒ the observable end state matches "no heap", but nothing here is starved of memory. A
//! port that tried to fix the null heap by supplying *memory* would be fixing a symptom
//! ninety lines downstream of the statement that actually failed.
//!
//! ## ★★★ The reply is a PURE STATUS — and a zeroed body would still be a lie
//!
//! `NV2080_CTRL_EVENT_SET_NOTIFICATION_PARAMS` has **zero output fields**. All five members
//! are documented as client-supplied (`ogkm-580:
//! src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080event.h:45-94`), and
//! `subdeviceCtrlCmdEventSetNotification_IMPL` consumes the RPC's result only through
//! `NV_CHECK_OK_OR_RETURN(LEVEL_WARNING, …)` (`ogkm-580:
//! src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_event_kernel.c:110-117`). By
//! `kayfabe_device::inert`'s own eligibility rule — *"the guest reads nothing but the
//! status"* — that reads like an inert command.
//!
//! ⊘ **It is not, and the reason is a property of the transport rather than of this
//! control.** On the success path `rpcRmApiControl_GSP` copies the reply's params back over
//! the caller's own struct:
//!
//! ```c
//! if (paramsSize != 0)
//! {
//!     portMemCopy(pParamStructPtr, paramsSize, rpc_params->params, paramsSize);
//! }
//! ```
//!
//! (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11085-11090`) — and the guest then reads
//! `pSetEventParams->action` and `->event` **after** the RPC returns, to drive its own
//! `notifyActions[]` switch (`subdevice_ctrl_event_kernel.c:119-146`). So an inert
//! all-zero body would clobber `event = 194, action = REPEAT` into `event = 0,
//! action = ACTION_DISABLE`, and the guest would quietly register the wrong notifier and
//! carry on with `NV_OK`.
//!
//! ★ That is the exact failure shape this port refuses everywhere else: an answer that
//! passes and is wrong. The reply must therefore carry the request's own field values back.
//! [`encode_event_set_notification`] re-encodes them **from the decoded struct**, not by
//! copying the request's bytes — so a byte this port did not model (either pad run) comes
//! back as zero rather than as whatever the guest happened to leave there.
//!
//! ⚠ On the **failure** path the copyout is skipped entirely (`rpc.c:11066-11070`, this
//! control carries no `RMCTRL_FLAGS_COPYOUT_ON_ERROR` — its flags are `0x10118`,
//! `ogkm-580: src/nvidia/generated/g_subdevice_nvoc.c:1606`), so a refusal leaves the
//! guest's struct untouched and is clean.
//!
//! ## ★★★ What this port may honestly promise: silence about an event that cannot happen
//!
//! `kayfabe_device::sweep`'s triage row for this control argued that *"accepting a
//! notification registration would promise an interrupt nothing raises"*, citing
//! `IrqRaise == 1` across the whole of `cap1` with zero `IRQSCLR` writes. The observation
//! is right and the conclusion does not follow: it conflates **registering** an arming with
//! **delivering** an event, and the cost of undelivered notifications is only paid for
//! events that actually occur.
//!
//! The one this control registers is `NV2080_NOTIFIERS_POWER_RESUME` = 194
//! (`ogkm-580: src/common/sdk/nvidia/inc/class/cl2080_notification.h:235`), action
//! `REPEAT`, callback `memmgrSuspendResumeCallback` (`mem_mgr.c:567-599`) — which requeues
//! `MemoryMapper` work **on resume from a power-state transition**. This device performs no
//! power-state transition: it has no suspend path, no resume path, and no GC6. So the
//! number of `POWER_RESUME` events it will fail to deliver is **zero**, and reporting none
//! is a true statement rather than a convenient one.
//!
//! ⊘ **Which is why the promise is scoped to a list and not to the control.**
//! [`SILENT_NOTIFIERS`] names the notifier indices this device can defend that argument
//! for; every other index is refused. A notifier this port *could* have to deliver — a
//! fault, a completion, a timer — must never be silently armed, and the safe direction is
//! the one where forgetting a row costs a loud boot rather than an unexplained hang.
//!
//! ## ⚠ `[measured]` where it appears in the oracle's own boot
//!
//! `cap1b` carries it at `rpc.sequence` **25** (txn 1001), `paylen 60` = 40 header + 20
//! params — an independent confirmation of the struct size below — inside the replay's
//! closure limit of 1028. `cargo run -p kayfabe-crec --example cap1b_report`.
//!
//! ## ★★★ INDEX 35 — what refusing it costs, and what serving it would COMMIT US TO
//!
//! `[measured 2026-08-08, boot `noprobe_8088019`, rev `8088019`, stock 580.159.04 guest,
//! probe set EMPTY]` — `docs/reference/bench_evidence/run_noprobe_8088019_{qemu,dmesg}.log`.
//! This is the one refusal standing between the shipping configuration and a device table
//! from `nvidia-smi`, so the argument for it is written out rather than assumed.
//!
//! ### 1. The refusal, and the guest's whole fall from it
//!
//! The device census names it exactly once:
//!
//! ```text
//! arming event 35 action 2 client 0xc1e00005 object 0x0000000b result 0x00000056 x1 REFUSED
//! ```
//!
//! `0x56` is `NV_ERR_NOT_SUPPORTED`, from [`SILENT_NOTIFIERS`] not admitting the index.
//! ⚠ The guest does **not** treat that as a warning. `kernel_graphics.c:508` is the
//! standing reminder that a wall which logs `LEVEL_ERROR` need not be the wall that ends
//! the function — so the attribution here rests on a **status fingerprint**, not on
//! proximity. `_memmgrMemUtilsScrubInitRegisterCallback` manufactures `NV_ERR_GENERIC`
//! (`0xFFFF`) at `ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/mem_utils.c:1929-1934`, and
//! that exact value appears at every frame of the guest's own dmesg on the way out:
//!
//! | frame | `ogkm-580` site |
//! |---|---|
//! | `"event notification control failed"` → `NV_ERR_GENERIC` | `mem_utils.c:1929-1934` |
//! | `memmgrMemUtilsChannelSchedulingSetup` | `mem_utils.c:2022` |
//! | `memmgrMemUtilsCopyEngineInitialize_GM107` | `mem_utils_gm107.c:1027` |
//! | `ceutilsConstruct` | `ce_utils.c:304` |
//! | `scrubberConstruct` → `objCreate(…, CeUtils, …)` | `mem_scrub.c:181` |
//! | `memmgrScrubHandlePostSchedulingEnable_GP100` | `mem_mgr_scrub_gp100.c:63` |
//! | `memmgrScrubHandlePostSchedulingEnable_HAL` | `mem_mgr.c:487` |
//! | `kfifoTriggerPostSchedulingEnableCallback` → `NV_ASSERT(0)` | `kernel_fifo.c:3129` |
//!
//! and `gpumgrStateLoadGpu` then fails at
//! `arch/nvalloc/unix/src/osinit.c:1244-1250`, which is
//! `RM_SET_ERROR(*status, RM_INIT_GPU_LOAD_FAILED)` and the boot's verdict line:
//!
//! ```text
//! NVRM: RmInitNvDevice: *** Cannot load state into the device
//! NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x25:0xffff:1249)
//! ```
//!
//! ⇒ `0x25` is that arm's `RM_INIT_GPU_LOAD_FAILED`, `0xffff` is the `NV_ERR_GENERIC` this
//! refusal minted eight frames earlier, and `1249` is the `return` after it. The three
//! numbers name one place, the way `8088019`'s `(0x43:0x59:2239)` did for the UUID.
//!
//! ### 2. ★★★ Serving it honestly DOES require real event delivery — and of a named kind
//!
//! ⊘ This is not the general "we deliver no events" hand-wave. The registration is
//! **non-stalling**: `nv0005AllocParams.notifyIndex = NV2080_NOTIFIERS_FIFO_EVENT_MTHD |
//! NV01_EVENT_NONSTALL_INTR` (`mem_utils.c:1901`), and the non-stall path does not read
//! `notifyActions[]` at all —
//!
//! - `eventGetEngineTypeFromSubNotifyIndex` maps index 35 → `RM_ENGINE_TYPE_HOST`
//!   (`src/nvidia/src/kernel/rmapi/event_notification.c:480-484`);
//! - `_gpuEngineEventNotificationListNotify` walks
//!   `pGpu->engineNonstallIntrEventNotifications[RM_ENGINE_TYPE_HOST]` and consults no
//!   `notifyActions` (`event_notification.c:279+`). Only the *stalling* path,
//!   `gpuNotifySubDeviceEvent_IMPL`, does (`gpu_rmapi.c:572-575`);
//! - and the one producer for `RM_ENGINE_TYPE_HOST` is **`intr.c:1201-1204`**, inside the
//!   guest's *own* non-stall interrupt service, under `pIntr->bDefaultNonstallNotify` —
//!   which is `NV_TRUE` for GA106 by name (`src/nvidia/generated/g_intr_nvoc.c:299-308`).
//!
//! ⇒ what the arming asks the GSP for is bookkeeping; what actually delivers is **the CPU
//! non-stall interrupt tree**, which is this device's to raise. Saying "armed" is therefore
//! a promise to raise a non-stall vector when the engine's work completes, and there is no
//! reading of it under which the promise is free.
//!
//! ### 3. ★ What makes it an INCREMENT and not an impossibility
//!
//! Three of the four pieces already exist, which is why this belongs in the file rather
//! than on a wishlist:
//!
//! - **The vectors are already published, from real hardware.** `GA106_INTR_TABLE` (24
//!   rows, verbatim from the C capture's single `0x20800a5c` reply) carries a real
//!   `vectorNonStall` on eight of them: GR (`MC_ENGINE_IDX` 84) → `0x00`, NVENC (38) →
//!   `0x01`, SEC2 (47) → `0x02`, NVDEC (65) → `0x03`, CE2 (17) → `0x07`, CE3 (18) →
//!   `0x08`, OFA0 (81) → `0x09`, CE4 (19) → `0x0a`.
//! - **The interrupt plane works.** `kayfabe_device::cpuintr` drives
//!   `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_*` and the driver's own loopback self-test passes:
//!   `[measured 2026-08-08, boot p35_8088019]` `interrupts: 3 vectors delivered, 0
//!   undeliverable`.
//! - **The completion moment is owned, not inferred.** E10e made the scrubber's CE copy
//!   real and it runs synchronously off the doorbell, so "the work finished" is an instant
//!   this device is standing at.
//!
//! ⚠ **The one thing not settled, and it is a measurement rather than a design.** Which CE
//! the scrubber lands on: `ceutilsGetFirstAsyncCe` takes the first CE that is not a GRCE
//! (`ce_utils.c:66-81`), and the captured table gives **CE0 (15) and CE1 (16) no non-stall
//! vector at all**. If the scrubber's `pChannel->ceId` is one of those two, then this
//! device has published — on real hardware's authority — that there is no vector to raise,
//! and index 35 must stay refused on that *stronger* ground rather than this one. One boot
//! that logs the chosen `ceId` decides it.
//!
//! ⊘ **Until then the refusal stands.** Admitting 35 into [`SILENT_NOTIFIERS`] would be
//! promising a completion notification for work that really happens, which is the exact
//! shape this module's own rule forbids — and the boot it buys is
//! [`ProbeArmSet`]'s reachability result, not a rung.

/// `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080event.h:79`).
pub const NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION: u32 = 0x2080_0301;

/// Byte offset of `event` — `[IN]`, the notifier index.
pub const EVENT_OFF: usize = 0;

/// Byte offset of `action` — `[IN]`.
pub const ACTION_OFF: usize = 4;

/// Byte offset of `bNotifyState` — `[IN]`, an `NvBool` and therefore **one** byte
/// (`ogkm-580: src/common/sdk/nvidia/inc/nvtypes.h:272`, `typedef NvU8 NvBool`).
pub const NOTIFY_STATE_OFF: usize = 8;

/// Byte offset of `info32` — `[IN]`. Three bytes of padding precede it, because `NvU32`
/// realigns after the one-byte `bNotifyState`.
pub const INFO32_OFF: usize = 12;

/// Byte offset of `info16` — `[IN]`, an `NvU16`.
pub const INFO16_OFF: usize = 16;

/// `sizeof(NV2080_CTRL_EVENT_SET_NOTIFICATION_PARAMS)` — 20, with two trailing pad bytes
/// rounding the `NvU16` up to the struct's 4-byte alignment.
///
/// ★ Confirmed independently of the header read: `cap1b`'s `rpc.sequence` 25 carries
/// `paylen 60`, and this port's control header is 40 bytes.
pub const EVENT_SET_NOTIFICATION_PARAMS_SIZE: usize = 20;

/// `NV2080_CTRL_EVENT_SET_NOTIFICATION_ACTION_DISABLE`
/// (`ogkm-580: ctrl2080event.h:92`).
pub const ACTION_DISABLE: u32 = 0;

/// `NV2080_CTRL_EVENT_SET_NOTIFICATION_ACTION_SINGLE` (`ogkm-580: ctrl2080event.h:93`).
pub const ACTION_SINGLE: u32 = 1;

/// `NV2080_CTRL_EVENT_SET_NOTIFICATION_ACTION_REPEAT` (`ogkm-580: ctrl2080event.h:94`).
pub const ACTION_REPEAT: u32 = 2;

/// `NV2080_NOTIFIERS_MAXCOUNT` — the guest's own bound on the notifier index
/// (`ogkm-580: src/common/sdk/nvidia/inc/class/cl2080_notification.h:239`).
pub const NV2080_NOTIFIERS_MAXCOUNT: u32 = 198;

/// `NV2080_NOTIFIERS_TIMER` — rejected by the guest's own handler before it ever reaches
/// the GSP (`ogkm-580: cl2080_notification.h:47`, and
/// `subdevice_ctrl_event_kernel.c:102-106`).
pub const NV2080_NOTIFIERS_TIMER: u32 = 11;

/// `NV2080_NOTIFIERS_POWER_RESUME` (`ogkm-580: cl2080_notification.h:235`).
pub const NV2080_NOTIFIERS_POWER_RESUME: u32 = 194;

/// ★★★ **The notifier indices this device can truthfully promise silence for.**
///
/// A registration this port accepts is a statement that it will deliver the event when it
/// occurs. This port delivers no events at all, so the only registrations it may accept are
/// those for events that **cannot occur on this device** — where "we delivered none" and
/// "none happened" are the same sentence.
///
/// ⊘ Deliberately a list and not a predicate over `< NV2080_NOTIFIERS_MAXCOUNT`. The
/// argument is per-notifier and rests on a property of *this device*, not on the index
/// being in range; a rule shaped `everything below 198` would quietly cover fault and
/// completion notifiers whose silence would be a hang nobody could attribute.
/// `gates_quantified_over_a_list` is the memory this follows: pin the list, or derive the
/// universe — never widen it because the wider rule is shorter to write.
pub const SILENT_NOTIFIERS: &[SilentNotifier] = &[SilentNotifier {
    index: NV2080_NOTIFIERS_POWER_RESUME,
    why: "NV2080_NOTIFIERS_POWER_RESUME fires from a power-state transition, and this \
          device performs none: it has no suspend path, no resume path and no GC6. Its \
          callback, memmgrSuspendResumeCallback (ogkm-580: mem_mgr.c:567-599), requeues \
          MemoryMapper work on resume — work that has nothing to resume from. So the count \
          of events this port fails to deliver for it is zero, which is a measurement and \
          not an excuse",
}];

/// One row of [`SILENT_NOTIFIERS`] — an index, and the argument for accepting it.
///
/// ★ The argument is a field rather than a comment so a test can demand every row carry
/// one, the way `kayfabe_device::sweep::SweepControl` does for a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SilentNotifier {
    /// The `NV2080_NOTIFIERS_*` index.
    pub index: u32,
    /// Why this port may report no deliveries of it without lying.
    pub why: &'static str,
}

/// Whether this port can promise silence about `index` — see [`SILENT_NOTIFIERS`].
#[must_use]
pub fn is_silent_notifier(index: u32) -> bool {
    SILENT_NOTIFIERS.iter().any(|n| n.index == index)
}

/// The most notifier indices a [`ProbeArmSet`] can carry. A probe naming more than this
/// is refused at parse time — loudly, because a silently truncated probe set is a boot
/// that ran with different instrumentation than its operator believes.
pub const PROBE_ARM_MAX: usize = 8;

/// ★★★ **PROBE ONLY. NEVER A SHIPPING PATH.** Notifier indices one *device instance* will
/// arm even though [`SILENT_NOTIFIERS`] does not admit them.
///
/// # ★ A device property, not an environment variable — and why that history matters
///
/// This began as `KAYFABE_PROBE_ARM_NOTIFIER`, read once per process from the environment.
/// **Three consecutive boots ran without the probe while looking correct from the
/// launching shell** — an env var set in one shell is invisible to a boot launched from
/// another, and nothing in the boot's own output said which set it ran with. The value now
/// arrives as a property on `-device nvkvm-gpu,probe-arm-notifier=…`, is parsed
/// **strictly** (junk refuses the device at realize rather than silently booting
/// probe-off), and the set in effect is reported in the device's own end-of-run census —
/// so a boot's report proves what it ran with.
///
/// # ⊘ What this is for, and what its green boot does NOT mean
///
/// `[measured]` boot `m1`, 2026-08-07 (`docs/reference/bench_evidence/m1_809b040_dmesg.log`):
/// the guest dies at `mem_utils.c:2022` arming **index 35**,
/// `NV2080_NOTIFIERS_FIFO_EVENT_MTHD` — a **completion** notifier, i.e. exactly the class
/// [`SILENT_NOTIFIERS`]'s own rationale forbids admitting, because *"a fault, a completion,
/// a timer must never be silently armed"* and the count of undelivered events would not be
/// zero.
///
/// So the honest question is not *"may we admit it"* (no) but *"does the guest **block** on
/// the callback during `RmInitAdapter`, or merely register it and continue?"* — and that is
/// a fact about the guest, measurable in one boot and derivable from no amount of reading.
/// This switch buys that one measurement.
///
/// ★ **ANSWERED**: it registers and continues. `[measured]` boot `subdev_probe35_925b27b`,
/// 2026-08-07 (`docs/reference/bench_evidence/run_subdev_probe35_925b27b_dmesg.log`): with
/// both scrubber subdevices' index-35 armings served (per-subdevice `notify_actions`), the
/// guest did not wait for any delivery — it advanced straight to `memmgrTestCeUtils`' real
/// CE memset and timed out on the **CE semaphore** (`mem_mgr.c:463`, `NV_ERR_TIMEOUT`),
/// not on a notifier. The blocking dependency is the copy engine, not the callback.
///
/// ⊘ **A boot that goes further with this set is NOT a rung.** It is a *reachability*
/// result: "the guest reaches X once this refusal stops firing". It says nothing about
/// correctness, and anything it unblocks still owes the real answer — which for a
/// completion notifier means **delivering the event**, not admitting the arming.
///
/// ★ [`ProbeArmSet::default`] is empty, and the default is asserted by a test rather than
/// trusted: the failure mode being guarded is a probe that quietly becomes the product,
/// which is `mock_fidelity_both_directions`' too-capable half with a flag on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProbeArmSet {
    indices: [u32; PROBE_ARM_MAX],
    len: u8,
}

/// Why a probe string could not become a [`ProbeArmSet`].
///
/// ⊘ Every variant is a **refusal to boot**, not a value. The predecessor env-var parser
/// used `filter_map(… .ok())`, under which `probe-arm-notifier=3S` silently booted with a
/// *smaller* probe set than asked — exactly the "boot ran with different instrumentation
/// than reported" failure the property exists to kill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeArmParseError {
    /// A comma-separated token was not a decimal `u32`.
    NotDecimal,
    /// More than [`PROBE_ARM_MAX`] indices were named.
    TooMany,
}

impl ProbeArmSet {
    /// Parse a comma-separated decimal list (`"35"`, `"35, 37"`). An empty or
    /// all-whitespace string is the empty set — the default, spelled out.
    ///
    /// # Errors
    ///
    /// [`ProbeArmParseError`] — strictly: any token that is not a decimal `u32`, or more
    /// than [`PROBE_ARM_MAX`] of them, refuses the whole string rather than keeping the
    /// parseable subset.
    pub fn parse(s: &str) -> Result<ProbeArmSet, ProbeArmParseError> {
        let mut set = ProbeArmSet::default();
        if s.trim().is_empty() {
            return Ok(set);
        }
        for token in s.split(',') {
            let idx: u32 = token
                .trim()
                .parse()
                .map_err(|_| ProbeArmParseError::NotDecimal)?;
            if usize::from(set.len) == PROBE_ARM_MAX {
                return Err(ProbeArmParseError::TooMany);
            }
            set.indices[usize::from(set.len)] = idx;
            set.len += 1;
        }
        Ok(set)
    }

    /// Whether this run's probe admits arming `index`.
    #[must_use]
    pub fn contains(&self, index: u32) -> bool {
        self.as_slice().contains(&index)
    }

    /// The indices named, in the order they were named.
    #[must_use]
    pub fn as_slice(&self) -> &[u32] {
        &self.indices[..usize::from(self.len)]
    }

    /// `true` iff no index is admitted — the shipping configuration.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// A decoded `NV2080_CTRL_EVENT_SET_NOTIFICATION_PARAMS`.
///
/// ⊘ Every field is `[IN]`. There is nothing here for this port to *answer* — the struct
/// exists so the reply can be rebuilt from values that were understood, rather than by
/// reflecting the request's bytes. See the module docs for why the reply cannot simply be
/// empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventSetNotification {
    /// The notifier index.
    pub event: u32,
    /// `ACTION_DISABLE` / `ACTION_SINGLE` / `ACTION_REPEAT`.
    pub action: u32,
    /// The notifier's state at registration time.
    pub notify_state: bool,
    /// Client-supplied 32-bit initial state.
    pub info32: u32,
    /// Client-supplied 16-bit initial state.
    pub info16: u16,
}

/// Why a registration could not be decoded, or may not be accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventNotifyError {
    /// The guest's params are shorter than the struct it declared.
    ShortParams {
        /// How many bytes arrived.
        got: usize,
    },
    /// `event >= NV2080_NOTIFIERS_MAXCOUNT` — the guest's own bound
    /// (`ogkm-580: subdevice_ctrl_event_kernel.c:96-100`).
    EventOutOfRange {
        /// The index asked for.
        event: u32,
    },
    /// ★ `NV2080_NOTIFIERS_TIMER`, which has its own control and which the guest's handler
    /// rejects before the RPC (`ogkm-580: subdevice_ctrl_event_kernel.c:102-106`). Reaching
    /// the GSP at all means the guest is not the one we modelled.
    TimerEvent,
    /// An `action` outside the three the SDK names. The guest's own `switch` has a
    /// `default: status = NV_ERR_INVALID_ARGUMENT` arm
    /// (`ogkm-580: subdevice_ctrl_event_kernel.c:141-145`).
    UnknownAction {
        /// The action asked for.
        action: u32,
    },
    /// ★★★ The index is legal, and this port cannot promise silence about it — see
    /// [`SILENT_NOTIFIERS`]. ⊘ The safe direction: a notifier nobody has written an
    /// argument for is refused loudly, not armed quietly.
    NotifierNotSilent {
        /// The index asked for.
        event: u32,
    },
    /// ★★ Arming an already-armed notifier. The guest enforces exactly this on its own copy
    /// — *"must be in disabled state to transition to an active state"*
    /// (`ogkm-580: subdevice_ctrl_event_kernel.c:124-131`, `NV_ERR_INVALID_STATE`) — and
    /// the physical RM on a real GSP keeps the same `notifyActions[]` array for its own
    /// subdevice. Modelling it is transcription, not invention.
    AlreadyArmed {
        /// The index asked for.
        event: u32,
        /// What it is currently armed with.
        current: u32,
    },
}

impl core::fmt::Display for EventNotifyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ShortParams { got } => write!(
                f,
                "event-notification params are {got} bytes; the struct is \
                 {EVENT_SET_NOTIFICATION_PARAMS_SIZE}"
            ),
            Self::EventOutOfRange { event } => write!(
                f,
                "notifier index {event} is at or past NV2080_NOTIFIERS_MAXCOUNT \
                 ({NV2080_NOTIFIERS_MAXCOUNT}); the guest's own handler rejects it \
                 (subdevice_ctrl_event_kernel.c:96-100)"
            ),
            Self::TimerEvent => write!(
                f,
                "NV2080_NOTIFIERS_TIMER has its own control and the guest rejects it before \
                 the RPC (subdevice_ctrl_event_kernel.c:102-106), so it cannot legitimately \
                 arrive here"
            ),
            Self::UnknownAction { action } => write!(
                f,
                "action {action} is none of DISABLE/SINGLE/REPEAT; the guest's own switch \
                 answers NV_ERR_INVALID_ARGUMENT (subdevice_ctrl_event_kernel.c:141-145)"
            ),
            Self::NotifierNotSilent { event } => write!(
                f,
                "notifier index {event} is not one this device can promise silence for; \
                 this port delivers no events, so accepting an arming for an event that \
                 CAN occur would promise a notification nothing raises"
            ),
            Self::AlreadyArmed { event, current } => write!(
                f,
                "notifier index {event} is already armed with action {current}; RM requires \
                 the disabled state to transition to an active one \
                 (subdevice_ctrl_event_kernel.c:124-131)"
            ),
        }
    }
}

impl core::error::Error for EventNotifyError {}

/// Decode `NV2080_CTRL_EVENT_SET_NOTIFICATION_PARAMS`, validating everything the guest's
/// own handler validates.
///
/// ⊘ Membership of [`SILENT_NOTIFIERS`] is **not** checked here: that is this port's
/// policy, not the wire format, and a decoder that refused it could not be used to describe
/// a registration this port declines. See [`EventNotifyError::NotifierNotSilent`] for where
/// the policy is applied.
///
/// # Errors
///
/// [`EventNotifyError::ShortParams`], [`EventNotifyError::EventOutOfRange`],
/// [`EventNotifyError::TimerEvent`], [`EventNotifyError::UnknownAction`].
pub fn decode_event_set_notification(
    params: &[u8],
) -> Result<EventSetNotification, EventNotifyError> {
    if params.len() < EVENT_SET_NOTIFICATION_PARAMS_SIZE {
        return Err(EventNotifyError::ShortParams { got: params.len() });
    }
    let u32_at = |off: usize| {
        u32::from_le_bytes([
            params[off],
            params[off + 1],
            params[off + 2],
            params[off + 3],
        ])
    };
    let event = u32_at(EVENT_OFF);
    if event >= NV2080_NOTIFIERS_MAXCOUNT {
        return Err(EventNotifyError::EventOutOfRange { event });
    }
    if event == NV2080_NOTIFIERS_TIMER {
        return Err(EventNotifyError::TimerEvent);
    }
    let action = u32_at(ACTION_OFF);
    if !matches!(action, ACTION_DISABLE | ACTION_SINGLE | ACTION_REPEAT) {
        return Err(EventNotifyError::UnknownAction { action });
    }
    Ok(EventSetNotification {
        event,
        action,
        // ⊘ Any non-zero byte is `NV_TRUE`. NvBool is NvU8 and RM writes it with
        // `NV_TRUE`/`NV_FALSE`, but nothing guarantees a guest only ever writes 0 or 1, and
        // `== 1` would silently read 2 as false.
        notify_state: params[NOTIFY_STATE_OFF] != 0,
        info32: u32_at(INFO32_OFF),
        info16: u16::from_le_bytes([params[INFO16_OFF], params[INFO16_OFF + 1]]),
    })
}

/// Re-encode a decoded registration as the reply body.
///
/// ★★★ **Not an echo.** The bytes are rebuilt from [`EventSetNotification`]'s fields, so
/// both pad runs (`9..12` and `18..20`) come back **zero** whatever the guest left in them,
/// and a byte this port never modelled cannot be reflected. The values that do come back
/// are the guest's own `[IN]`s, because `rpcRmApiControl_GSP` copies this body over the
/// caller's struct and the caller reads `event` and `action` out of it afterwards
/// (`ogkm-580: rpc.c:11085-11090`, `subdevice_ctrl_event_kernel.c:119-146`) — see the
/// module docs.
#[must_use]
pub fn encode_event_set_notification(reg: &EventSetNotification) -> Vec<u8> {
    let mut params = vec![0u8; EVENT_SET_NOTIFICATION_PARAMS_SIZE];
    params[EVENT_OFF..EVENT_OFF + 4].copy_from_slice(&reg.event.to_le_bytes());
    params[ACTION_OFF..ACTION_OFF + 4].copy_from_slice(&reg.action.to_le_bytes());
    params[NOTIFY_STATE_OFF] = u8::from(reg.notify_state);
    params[INFO32_OFF..INFO32_OFF + 4].copy_from_slice(&reg.info32.to_le_bytes());
    params[INFO16_OFF..INFO16_OFF + 2].copy_from_slice(&reg.info16.to_le_bytes());
    params
}

#[cfg(test)]
mod probe_default_tests {
    use super::{PROBE_ARM_MAX, ProbeArmParseError, ProbeArmSet};

    /// ★★★ The probe is OFF unless a device property names an index — and index 35, the
    /// one the `m1` boot died on, is refused by default like any other completion notifier.
    ///
    /// ⊘ This is the guard against the probe quietly becoming the product. It asserts the
    /// DEFAULT — `ProbeArmSet::default()`, which is what every constructor that was not
    /// handed an explicit set uses — since the default is the only thing a shipped binary
    /// ever sees.
    #[test]
    fn the_notifier_probe_is_off_by_default_and_35_is_not_silent() {
        let default = ProbeArmSet::default();
        assert!(default.is_empty());
        for idx in [35u32, 0, 37, 194, 197] {
            assert!(
                !default.contains(idx),
                "★ notifier {idx} is probe-armed by the DEFAULT set — the probe has \
                 become the default, which is the failure this test exists for"
            );
        }
        assert!(
            !super::is_silent_notifier(35),
            "⊘ index 35 is NV2080_NOTIFIERS_FIFO_EVENT_MTHD, a COMPLETION notifier. If it \
             ever enters SILENT_NOTIFIERS the port is promising silence about an event it \
             would actually have to deliver — see the module docs"
        );
        assert!(super::is_silent_notifier(
            super::NV2080_NOTIFIERS_POWER_RESUME
        ));
    }

    /// The property string is parsed STRICTLY. The predecessor env-var parser dropped
    /// unparseable tokens with `filter_map`, so a typo booted with a smaller probe set
    /// than asked and nothing said so.
    #[test]
    fn probe_parse_is_strict_and_refuses_junk() {
        assert_eq!(ProbeArmSet::parse("").unwrap(), ProbeArmSet::default());
        assert_eq!(ProbeArmSet::parse("  ").unwrap(), ProbeArmSet::default());

        let one = ProbeArmSet::parse("35").unwrap();
        assert!(one.contains(35) && !one.is_empty());
        assert_eq!(one.as_slice(), &[35]);

        let two = ProbeArmSet::parse(" 35 , 37 ").unwrap();
        assert_eq!(two.as_slice(), &[35, 37]);
        assert!(!two.contains(36));

        // ⊘ Exact-variant assertions, never `is_err()` (`testing_doctrine.md`).
        assert_eq!(
            ProbeArmSet::parse("3S"),
            Err(ProbeArmParseError::NotDecimal),
            "a typo must refuse the boot, not silently shrink the probe set"
        );
        assert_eq!(
            ProbeArmSet::parse("35,"),
            Err(ProbeArmParseError::NotDecimal)
        );
        assert_eq!(
            ProbeArmSet::parse("-1"),
            Err(ProbeArmParseError::NotDecimal)
        );

        let too_many = (0..=PROBE_ARM_MAX as u32)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            ProbeArmSet::parse(&too_many),
            Err(ProbeArmParseError::TooMany),
            "overflow must refuse, never truncate: a silently clipped set is a boot that \
             ran with different instrumentation than its operator believes"
        );
        let exactly_full = (0..PROBE_ARM_MAX as u32)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            ProbeArmSet::parse(&exactly_full).unwrap().as_slice().len(),
            PROBE_ARM_MAX
        );
    }
}
