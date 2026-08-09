//! The command policy that answers the twenty-five `GSP_RM_CONTROL`s the guest's RM cannot
//! get past — most from the chip row's own tables, two that are verbs rather than tables,
//! and ★ one ([`WantedTable::GpuInfoV2`]) whose reply is a **function of the request**.
//!
//! ⊘ The count is stated here only as orientation; [`WantedTable::ALL`] is the universe
//! every gate quantifies over, and it is the array — not this sentence — that decides what
//! is served.
//!
//! ⚠ The type names say *table* and most of them are not one:
//! `NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO` is an identity,
//! `NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP` is a **permission policy**,
//! and `NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO` is an **instruction to construct**
//! — none of which is a list. The names are kept because what actually unifies them is
//! the property the module is about — **`[OUT]`-only, and a pure function of the chip
//! row** — and that held for all twelve.
//!
//! ⚠ **It no longer holds for all of them.** [`WantedTable::EventSetNotification`] is a
//! *verb on the event plane*: nothing about it is `[OUT]`, no chip row could answer it, and
//! serving it changes this policy's state. [`WantedTable::MemsysL2InvalidateEvict`] is a
//! verb on **hardware this device does not have** — it asks for an L2 that does not exist to
//! be evicted, and its licence is a structural claim about this port rather than a row.
//! Both are here because [`WantedTable::ALL`] is the universe every coverage gate quantifies
//! over, not because either is a table — their own rustdoc argues that trade rather than
//! eliding it.
//!
//! ## ★★ Why this is here and not in a logic crate
//!
//! It is the composition of two things that already exist: the *rows*
//! ([`crate::ChipProfile::engines`], a fact about silicon) and the *layout*
//! (`kayfabe_abi::inittables`, the Axis-A quarantine). This crate is the adapter where a
//! concrete chip's facts are allowed to meet a wire, so the join belongs here. Nothing in
//! this file names a generation, a driver version or an engine — a second chip is a second
//! row, and this file does not change.
//!
//! ## ★★★ What it does NOT do, deliberately
//!
//! Fourteen controls, twelve of them `[OUT]`-only and answered from the chip row. It
//! touches no RM graph state and allocates no handle. ⚠ It no longer *remembers nothing*:
//! [`InitTablePolicy::notify_actions`] is **per-subdevice** arming state — one action per
//! notifier index, keyed by the control header's own `(hClient, hObject)` and bounded by a
//! fixed slot count — see that field for the bound and its named residuals. Every other command
//! falls through to whatever the FSM would have done — this is a *supplement* to the
//! baseline policy, not a replacement for `kayfabe_rmrpc::GraphPolicy`, which is the
//! semantic policy the compute path will need.
//!
//! ## ★ It refuses rather than guessing, and the refusal is the loud kind
//!
//! A guest that declares a `paramsSize` other than the one this port's layout produces is
//! a guest whose struct is not the struct we encode. Answering it anyway would hand RM a
//! well-formed table read at the wrong strides — the exact failure mode the `EchoOk` doc
//! argues about at length, where the rejection lives in the payload and nothing logs. So
//! the mismatch is answered with a non-zero **envelope** `rpc_result`, which short-circuits
//! the guest ahead of both the copy-out and the control cache
//! (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:1994`).

use kayfabe_abi::NV_ERR_NOT_SUPPORTED;
use kayfabe_abi::bifstatic::{
    self, BIF_STATIC_INFO_PARAMS_SIZE, NV2080_CTRL_CMD_INTERNAL_BIF_GET_STATIC_INFO,
};
use kayfabe_abi::chipinfo::{
    self, CHIP_INFO_PARAMS_SIZE, ChipIdentity, NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO,
};
use kayfabe_abi::confcompute::{
    self, CONF_COMPUTE_STATIC_INFO_PARAMS_SIZE,
    NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_GET_STATIC_INFO,
};
use kayfabe_abi::deviceinfo::{
    self, INTERNAL_DEVICE_INFO_PARAMS_SIZE, NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE,
};
use kayfabe_abi::eventnotify;
use kayfabe_abi::falconinfo::{
    self, FALCON_INFO_PARAMS_SIZE, NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO,
};
use kayfabe_abi::fifochannels::{
    self, FIFO_NUM_CHANNELS_PARAMS_SIZE, NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_CHANNELS,
};
use kayfabe_abi::gmmustatic::{
    self, GMMU_STATIC_INFO_PARAMS_SIZE, NV2080_CTRL_CMD_INTERNAL_GMMU_GET_STATIC_INFO,
};
use kayfabe_abi::gpuinfo::{self, GPU_GET_INFO_V2_PARAMS_SIZE, NV2080_CTRL_CMD_GPU_GET_INFO_V2};
use kayfabe_abi::grstatic;
use kayfabe_abi::gvaspacepdes;
use kayfabe_abi::inittables::{
    self, DEVICE_INFO_PARAMS_SIZE, INTR_PARAMS_SIZE, NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE,
    NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE,
};
use kayfabe_abi::l2evict;
use kayfabe_abi::memsysconfig::{
    self, MEMSYS_STATIC_CONFIG_PARAMS_SIZE, NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG,
};
use kayfabe_abi::pcibars::{self, NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO, PCI_BAR_INFO_PARAMS_SIZE};
use kayfabe_abi::regaccessmap::{
    self, NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP,
    USER_REGISTER_ACCESS_MAP_PARAMS_SIZE,
};
use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

use crate::ChipProfile;

/// Byte offset of `status` within `rpc_gsp_rm_control_v03_00` — the `[OUT]` field
/// `rpcRmApiControl_GSP` reads *before* it copies params out, and the one that decides
/// whether it copies them at all (`ogkm-580: rpc.c:11061-11065`).
///
/// ★ Not derived from `RpcControlReq`, because that view deliberately omits `status`: it
/// decodes what a guest **sent**, and `status` is a field only a reply fills in.
const CONTROL_STATUS_OFF: usize = 12;

/// Byte offset of `paramsSize` in the same header.
///
/// ★★ **CORRECTED (PC-D6).** This used to say the field was rewritten *"so RM's copy-out
/// length is ours rather than the request's echo"*. That reason is **wrong**, and a
/// load-bearing wrong reason is worse than none — it is exactly what the claim ledger
/// exists to catch. On the flat path this policy serves, RM copies out with the CALLER's
/// own local `paramsSize` (set by `serverSerializeCtrlDown`, `ogkm-580: rpc.c:10937`) and
/// never reads the reply's field: `portMemCopy(pParamStructPtr, paramsSize,
/// rpc_params->params, paramsSize)` (`:11085-11089`). `rmapiControlCacheSetUnchecked` uses
/// the same local (`:11096-11103`). So writing it changes nothing a guest can observe.
///
/// It is still written, and the honest reason is smaller: the reply then **describes
/// itself** — a `paramsSize` echoed from a request whose params we replaced would state a
/// length the body does not have, and this port's own decoders read it. ⊘ Nothing depends
/// on that; it is a consistency property, not a mechanism.
///
/// ⚠ The one place the reply's `paramsSize` IS load-bearing is the FINN-serialized arm,
/// `portMemCopy(pCallContext->pSerializedParams, ..., rpc_params->paramsSize)`
/// (`ogkm-580: rpc.c:11072-11075`) — and [`InitTablePolicy::respond`] refuses serialized
/// payloads outright, so this policy never reaches it. A policy that ever stops refusing
/// them inherits a real dependency on this write.
const CONTROL_PARAMS_SIZE_OFF: usize = 16;

/// `RM_GSS_LEGACY_MASK` — bit 15 of a control id
/// (`ogkm-580: src/nvidia/interface/deprecated/rmapi_deprecated.h:41`,
/// `IsGssLegacyCall` at `rmapi_deprecated_control.c:95-98`).
///
/// ★★ **The gate on a STICKY answer.** A reply's `rmctrlFlags` decide whether the guest
/// puts our answer in its control cache *permanently*: `rmapiControlCacheSetUnchecked`
/// (`ogkm-580: rpc.c:11096-11103`) is reached only when `IsGssLegacyCall(cmd)` holds, is
/// not FINN-serialized, and the flags say cacheable.
///
/// ⚠ **CORRECTED (2026-08-01).** This used to continue: *"Our replies reflect the request's
/// `rmctrlFlags`, because the whole control header is kept — so for a GSS-legacy control
/// the guest would cache whatever we answered and never ask again."* The mechanism is
/// backwards. `rpcRmApiControl_GSP` assigns `rpc_params->rmctrlFlags = 0;
/// rpc_params->rmctrlAccessRight = 0;` before every send (`ogkm-580: rpc.c:10994-10995`,
/// `ogkm-610: :10799-10800`), and `rmapiControlIsCacheable` returns `NV_FALSE` the moment
/// `!(flags & RMCTRL_FLAGS_CACHEABLE_ANY)` (`ogkm-580: rmapi_cache.c:152-158`). So a
/// reflected header carries zero and the reflection is what SAVES an echo — against a
/// **stock** guest. What is real is that `rmctrlFlags` is a field the guest wrote, so a
/// guest that pre-sets it gets it reflected; the reasoning was wrong, the guard is not.
/// [`crate::sticky`] §2 carries the full reading and closes the branch for every policy at
/// once.
///
/// None of the controls this port serves has bit 15 set, so the branch is unreachable
/// today. ⊘ Nothing checked that, which is the defect shape: the NEXT served control with
/// bit 15 set would inherit a sticky wrong answer with no test and no log.
/// [`InitTablePolicy::respond`] now asserts it at the serve site.
const RM_GSS_LEGACY_MASK: u32 = 0x0000_8000;

/// `IsGssLegacyCall(cmd)` — the driver's own predicate, one line and one mask
/// (`ogkm-580: src/nvidia/interface/deprecated/rmapi_deprecated_control.c:95-98`).
///
/// ★ Public and separate from [`InitTablePolicy::respond`] on purpose. The guard inside
/// `respond` is **unreachable** while no served control has bit 15 set, and an unreachable
/// branch cannot be bitten — so the predicate it rests on is exposed and tested directly.
/// The refusal stays where the decision is made; the mechanism is checkable here.
#[must_use]
pub fn is_gss_legacy(cmd: u32) -> bool {
    cmd & RM_GSS_LEGACY_MASK != 0
}

/// `NV_OK`.
const NV_OK: u32 = 0;

/// How many subdevices may hold **armed** notifiers at once.
///
/// The key of [`InitTablePolicy::notify_actions`] is guest-minted (`hClient, hObject`), so
/// the state must be bounded, and the bound must fail **loud**: the arming that would need
/// a seventeenth slot is refused (a `0x56` row in the census with this comment as its
/// attribution), never silently evicted. The GA106 boot arms notifiers on **three** distinct
/// subdevices (`docs/reference/bench_evidence/census_probe35_6c51da7_census.log`), so
/// sixteen is headroom, not a squeeze.
pub const NOTIFY_SUBDEVICE_SLOTS: usize = 16;

/// One subdevice's arming state — the mirror of RM's own
/// `pSubdevice->notifyActions[NV2080_NOTIFIERS_MAXCOUNT]`
/// (`ogkm-580: subdevice_ctrl_event_kernel.c:119-146`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SubdeviceNotifyActions {
    /// `hClient` the arming control arrived under.
    client: u32,
    /// `hObject` — the subdevice the arming was issued against.
    object: u32,
    /// One action per notifier index, exactly RM's array.
    actions: [u8; eventnotify::NV2080_NOTIFIERS_MAXCOUNT as usize],
}

/// Answers the FIFO device-info table and the kernel interrupt table from a chip row.
///
/// Every other command gets `None`, i.e. the FSM's own acknowledgement.
#[derive(Debug, Clone, Copy)]
pub struct InitTablePolicy {
    chip: &'static ChipProfile,
    driver: DriverAbiTable,
    /// ★★★ **The only state this policy holds** — **per-subdevice** arming state, mirroring
    /// the `pSubdevice->notifyActions[NV2080_NOTIFIERS_MAXCOUNT]` array RM keeps on both
    /// sides of the RPC (`ogkm-580: subdevice_ctrl_event_kernel.c:119-146`).
    ///
    /// ⚠ **Per-subdevice is a bug fix, not a refinement.** This used to be ONE
    /// device-global array, and RM's already-armed transition rule reads
    /// `pSubdevice->notifyActions` — per subdevice
    /// (`ogkm-580: subdevice_ctrl_event_kernel.c:124-131`). The GA106 scrubber builds two
    /// channels (`runQueues=2`), each legitimately arming
    /// `NV2080_NOTIFIERS_FIFO_EVENT_MTHD` (35) once on its **own** subdevice; the global
    /// array aliased them into one slot and refused the second with `0x56` where a real GSP
    /// accepts — which is what reached `mem_utils_gm107.c:1027`. `[measured]` boot
    /// `census_probe35` at `6c51da7`
    /// (`docs/reference/bench_evidence/census_probe35_6c51da7_census.log`):
    /// `(0xc1e00005, 0x0b)` served, `(0xc1e00006, 0x0c)` refused.
    /// `tests/event_set_notification.rs` carries those rows as the regression fixture.
    ///
    /// ⊘ The old "keyed by nothing a guest can mint" property is **narrowed, with its
    /// residuals named**, because the key is now `(hClient, hObject)` — guest-minted:
    /// - Bounded: [`NOTIFY_SUBDEVICE_SLOTS`] fixed slots. The arming that would need one
    ///   more is refused — loud in the census — never silently evicted.
    /// - A slot is occupied only while at least one notifier on it is armed: a subdevice
    ///   whose every action returns to `ACTION_DISABLE` releases its slot, so arm/disarm
    ///   cycling cannot grow the table and an ordinary disarmed recycle costs nothing.
    /// - Residual, named: a subdevice freed while still ARMED keeps its slot (this policy
    ///   does not observe `FREE`), so a recycle of that exact `(hClient, hObject)` pair
    ///   would wrongly refuse its first arming — loudly, as a census row, with this
    ///   comment as the attribution. The boot path frees no armed subdevice.
    ///
    /// ★ It keeps the type `Copy`: a fixed array of fixed-size slots, not a map.
    notify_actions: [Option<SubdeviceNotifyActions>; NOTIFY_SUBDEVICE_SLOTS],
    /// ★ **PROBE ONLY, and empty by default** — notifier indices this device instance will
    /// arm although [`kayfabe_abi::eventnotify::SILENT_NOTIFIERS`] does not admit them.
    /// Arrives from the `probe-arm-notifier` device property (it used to be a process env
    /// var, under which three boots ran probe-off while looking armed from the launching
    /// shell). A boot that goes further because of this set measures REACHABILITY, never
    /// correctness — see `ProbeArmSet`'s docs.
    probe_arm: eventnotify::ProbeArmSet,
    /// ★★★ **The guest's own `NV_VERSION_STRING`**, latched off `SET_GUEST_SYSTEM_INFO`
    /// (fn 1) and served back as [`WantedTable::GspGetFeatures`]'s `firmwareVersion`.
    ///
    /// ★ **Why this policy latches it rather than [`crate::guestsysinfo`].** The two are
    /// separate links with no shared state, and adding some would be the larger change;
    /// but this link is seated **ahead** of `GuestSystemInfoPolicy` in the chain
    /// (`crate::gsp_policy_chain`), so it sees fn 1 first and can read it without
    /// answering it. The observation returns `None`, which is a decline — the version
    /// handshake is still answered downstream, by the link that owns it, and no reply byte
    /// changes. ⊘ That ordering is load-bearing rather than incidental, and
    /// `tests/gsp_get_features.rs` executes it.
    ///
    /// ⚠ `None` until fn 1 arrives, and then `GspGetFeatures` **refuses** rather than
    /// guessing. The safe direction, and unreachable in practice: `kgspInitRm` sends fn 1
    /// during the version handshake — a boot where it fails never initialises the adapter
    /// at all (`crate::guestsysinfo`'s `t127a`) — while `0x20803601` arrives at `cuInit`,
    /// hundreds of commands later.
    ///
    /// ⊘ It is also `None` when the guest's string is one this port will not repeat: the
    /// bytes are guest-controlled, so they are validated into
    /// [`kayfabe_abi::gspfeatures::FirmwareVersion`] at the latch and dropped if they fail.
    /// Refusing one report-only control is a strictly smaller failure than putting an
    /// unvalidated guest buffer into a string `nvidia-smi` prints.
    guest_firmware: Option<kayfabe_abi::gspfeatures::FirmwareVersion>,
}

/// Which of the two tables a command asked for. Returned by [`InitTablePolicy::wanted`] so
/// a test can ask the classification question without building a wire message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WantedTable {
    /// `NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE`.
    DeviceInfo,
    /// `NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE`.
    IntrKernelTable,
    /// `NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO` — ★ the one whose *echo* faulted the guest's
    /// own kernel, because its caller does not pre-zero its params
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/bus/kern_bus.c:585`). `[measured]` run
    /// `t126b`, a stock 580.159.04 guest at `f2acb89`, twice on two fresh boots;
    /// `kayfabe_abi::pcibars`' module docs carry the dmesg, and
    /// `tests/pci_bar_info.rs::the_policy_answers_the_control_without_reflecting_one_byte_of_the_request`
    /// is the test that fails if the reply ever carries a request byte again.
    PciBarInfo,
    /// `NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO` — ★ not a table but an **identity**,
    /// and the one whose refusal ends `RmInitNvDevice` before anything else runs
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/gpu.c:886, 2124`). Its reply's identity half
    /// comes from [`crate::identity_for`], the same call that builds configuration space,
    /// because `_gpuInitChipInfo` overwrites `pGpu->idInfo` with what it carries. See
    /// [`kayfabe_abi::chipinfo`].
    ChipInfo,
    /// `NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP` — ★★ the one that is a
    /// **permission policy** rather than a description of silicon, and the first reply this
    /// port encodes that does not fit one message-queue element (`48 + 32 + 40 + 8204` is
    /// three of 580's 4096-byte elements). Its refusal ends `RmInitNvDevice` at
    /// `ogkm-580: gpu.c:2125`, the line after [`Self::ChipInfo`]'s. See
    /// [`kayfabe_abi::regaccessmap`], and [`crate::ga10x::GA106_USER_REGISTER_ACCESS_MAP`]
    /// for why this device publishes no map.
    UserRegisterAccessMap,
    /// `NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO` — ★★ the one that is an
    /// **instruction to construct** rather than a description, and the only one the guest
    /// asks **twice**: `gpuBuildGenericKernelFalconList` (`ogkm-580: gpu.c:2126, 5344`) and
    /// `gpuBuildKernelVideoEngineList` (`:2128, 5435`) each issue their own control, and
    /// `[measured]` the oracle carries both at `rpc.sequence` 5 and 6 with byte-identical
    /// replies. Its refusal ends `RmInitNvDevice` at `gpu.c:2126`, the line after
    /// [`Self::UserRegisterAccessMap`]'s. See [`kayfabe_abi::falconinfo`], and
    /// [`crate::ga10x::GA106_CONSTRUCTED_FALCONS`] for why this device names no falcon.
    ConstructedFalconInfo,
    /// `NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG` — ★★★ the first one whose
    /// refusal is **not survivable**, and the one that ends the ladder.
    ///
    /// Every variant above is reached from `gpuPreInit`, a chain of
    /// `NV_ASSERT_OK_OR_GOTO`s where a refusal stops the boot at a named statement. This
    /// one is reached from `gpuStatePreInit_IMPL`'s **engine sweep**
    /// (`ogkm-580: gpu.c:2152-2219`), which reads `NV_ERR_NOT_SUPPORTED` as *"this engine
    /// is absent — destroy it"* and continues. It is the whole of
    /// `kmemsysStatePreInitLocked_IMPL` (`ogkm-580: kern_mem_sys.c:99-134`), so refusing it
    /// deletes `KernelMemorySystem`, and RM then dereferences the pointer it NULLed from a
    /// different subsystem: `[measured]` run `t134a`, a stock 580.159.04 guest at `1c79474` —
    /// a guest-kernel `NULL` dereference in `memmgrGetBlackListPagesForHeap_GM107`.
    ///
    /// ⊘ And it is not inert-eligible in either direction. RM pre-zeroes the params
    /// (`kern_mem_sys.c:114`), so an inert reply and an all-zero served reply are the same
    /// forty bytes — and those forty bytes violate an invariant RM asserts on itself
    /// (`kern_mem_sys.c:422`) and divide-by-zero in `mem_mgr_gm107.c:211`. See
    /// [`kayfabe_abi::memsysconfig`], which makes both unencodable.
    MemorySystemStaticConfig,
    /// `NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE` — the DEVICE_INFO2 table, and ★★★
    /// the **second** unsurvivable refusal, by a worse mechanism than
    /// [`Self::MemorySystemStaticConfig`]'s.
    ///
    /// That one is asked from `gpuStatePreInit_IMPL`, whose sweep at least *NULLs* the
    /// engine pointer, so a `NULL` check downstream can catch it. This one is asked from
    /// `gpuStateInit_IMPL`, whose loop maps `NV_ERR_NOT_SUPPORTED` to `NV_OK` and does
    /// **not** remove the engine (`ogkm-580: gpu.c:2286-2287`) — leaving a
    /// constructed-but-uninitialised `KernelFifo` that every `NULL` check passes.
    ///
    /// ⊘ `[measured]` run `t135a`, a stock 580.159.04 guest at `c84ef52`: refusing it made
    /// `gpuConstructDeviceInfoTable_HAL @ kernel_fifo.c:2208` the guest's first and only
    /// `LEVEL_ERROR`, twenty times, then `kfifoConstructEngineList_HAL`, then a guest-kernel
    /// `NULL` dereference in `memmgrCalcReservedFbSpaceHal_GM107` — the heap sizing a
    /// reservation from a `KernelFifo` that has no engine list.
    ///
    /// ★★ And *"answer honestly with nothing"* is not available here either, unlike the
    /// falcon inventory: `gpuConstructDeviceInfoTable_FWCLIENT` accepts `numEntries == 0`
    /// with `NV_OK` (`ogkm-580: gpu_gspclient.c:231-232`), and the boot then dies two engine
    /// steps later in `kgmmuInitCeMmuFaultIdRange_GA100`, attributed to nothing. See
    /// [`kayfabe_abi::deviceinfo`].
    InternalDeviceInfo,
    /// `NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_GET_STATIC_INFO` — ★★★ the first variant this
    /// port serves whose refusal is **survivable**, and it is here anyway.
    ///
    /// `confComputeStateInitLocked_IMPL` and `confComputeStatePostLoad_IMPL` both ask it
    /// under `NV_ASSERT_OK_OR_RETURN` (`ogkm-580:
    /// src/nvidia/src/kernel/gpu/conf_compute/conf_compute.c:548-566` and `:441-456`), and
    /// both of their loops map `NV_ERR_NOT_SUPPORTED` to `NV_OK` without removing the
    /// engine (`ogkm-580: gpu.c:2286-2287`, `:3437-3439`). ⇒ no amputation, no halt.
    ///
    /// ⊘ The reason to serve it is the *other* failure shape: `ccStaticInfo` is a zeroed
    /// NVOC member, so a refusal and a served all-zero reply are **byte-identical to the
    /// guest**. The port would be defaulting where it could be stating, with nothing able
    /// to tell the two apart. See [`kayfabe_abi::confcompute`], which makes the widening
    /// direction — a trust claim this port cannot back — unencodable.
    ConfComputeStaticInfo,
    /// `NV2080_CTRL_CMD_INTERNAL_BIF_GET_STATIC_INFO` — the same fail-open shape as
    /// [`Self::ConfComputeStaticInfo`], by an even quieter mechanism: `kbifStateInitLocked`
    /// calls `kbifStaticInfoInit` as a **bare statement** and discards its status
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/bif/kernel_bif.c:132`), while every other call
    /// in that function is checked.
    ///
    /// ⊘ Two of its four `NvBool`s point RM at hardware — a coherent C2C mapping of
    /// framebuffer and a PCI function 1 — and [`kayfabe_abi::bifstatic`] makes both
    /// unencodable for a device that presents neither.
    BifStaticInfo,
    /// `NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_CHANNELS` — ★★★ the first control this port
    /// serves whose refusal **halts** the boot rather than amputating or failing open, and
    /// the only one whose reply reads a field out of the request.
    ///
    /// `kfifoRunlistQueryNumChannels_KERNEL` returns zero on any failure
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_fifo.c:1330-1336`) and
    /// `kfifoChidMgrConstruct` turns that zero into `NV_ERR_INVALID_STATE` (`:300-308`) —
    /// which `gpuStateInit_IMPL` does **not** map to `NV_OK`, so the boot aborts at a named
    /// statement. ⊘ Refusing is therefore *safe*; it is simply the end of the road, and
    /// every engine after `KernelFifo` in `gpuChildOrderList_GM200` is unreachable behind
    /// it. See [`kayfabe_abi::fifochannels`].
    FifoNumChannels,
    /// `NV2080_CTRL_CMD_INTERNAL_GMMU_GET_STATIC_INFO` — ★★★ the **third** unsurvivable
    /// refusal, and by the worst mechanism of the three.
    ///
    /// [`Self::MemorySystemStaticConfig`]'s refusal NULLs the engine pointer, so a `NULL`
    /// check can catch it. [`Self::InternalDeviceInfo`]'s leaves the engine constructed but
    /// empty. This one leaves `pKernelGmmu->pStaticInfo` pointing at memory
    /// `_kgmmuInitStaticInfo` has already `portMemFree`d, because its `fail:` label frees
    /// the allocation and does **not** NULL the field
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:139-166`), while
    /// `gpuStateInit_IMPL` maps the refusal to `NV_OK` and carries on (`gpu.c:2286-2287`).
    ///
    /// ⊘ `[inferred]` from source. No boot has been spent at a revision that serves it, and
    /// serving it lets `kgmmuStateInitLocked_IMPL` reach `kgmmuFaultBufferInit_HAL` for the
    /// first time — see [`kayfabe_abi::gmmustatic`], which states that boundary rather than
    /// eliding it.
    GmmuStaticInfo,
    /// `NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER` (`0x20800a9b`) — ★★★ **the wall
    /// §14.40 left standing, and the only variant whose reply is the IDENTITY on the guest's
    /// own bytes.**
    ///
    /// `[meas]` boot `pu1448` @ `ef20ccc`: fixing `BAR0+0x88084` moved `UVM_REGISTER_GPU`'s
    /// `rmStatus` from `0x40` to `0x56` and put one new line in `cuInit`'s own `dmesg` —
    /// `NVRM: faultbufConstruct_IMPL: Failed to setup Replayable Fault buffer
    /// (status=0x00000056).` — with exactly one new id in the unserviced ledger: this one.
    /// The mechanism is not a guess: `kgmmuFaultBufferReplayableAllocate_IMPL` propagates the
    /// control's status verbatim and **frees the buffer it just allocated** on failure
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:1261-1272`), and
    /// `faultbufConstruct_IMPL` re-returns it (`.../mmu_fault_buffer.c:59-67`), which fails
    /// the `MmuFaultBuffer` alloc inside `nvGpuOpsInitFaultInfo`
    /// (`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:9410`) and so fails
    /// `UVM_REGISTER_GPU`. ⇒ refusing this control is refusing `cuInit`.
    ///
    /// # ★★ Why the reply is the guest's own params, unchanged
    ///
    /// Every field is `[IN]` — `hClient`, `hObject`, `faultBufferSize`,
    /// `faultBufferPteArray[256]` — and the documented status set is `{NV_OK}`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:1792-1823`).
    /// A real GSP writes **nothing** back, so the bytes CPU-RM reads after the call are the
    /// bytes it sent. Echoing them is therefore not a convenience: it is the only reply that
    /// is byte-accurate. ⊘ Zeroing a declared-size body — [`Self::MemsysL2InvalidateEvict`]'s
    /// shape — would have been *harmless here* (CPU-RM `portMemFree`s the params immediately,
    /// `kern_gmmu.c:1263`) and still wrong, and [`Self::EventSetNotification`] is the
    /// cautionary record of a zeroed body silently rewriting a caller's struct.
    ///
    /// ⇒ **This arm fabricates nothing.** There is no captured row for `0x20800a9b` anywhere
    /// — not in `traces/c_oracle_census/initctrl_ga106_census.tsv`, not in
    /// [`kayfabe_abi::oracle`] — and it needs none, because there is no `[OUT]` value to be
    /// right or wrong about. Contrast the oracle's `dlen = 0` rows, which are *positively
    /// wrong* precisely because they decode an unmeasured body to zeros.
    ///
    /// # ★★★ And is `NV_OK` HONEST, with no fault-delivery plane?
    ///
    /// Decided on evidence in [`kayfabe_abi::faultbuffer`]'s module docs — the three-line
    /// version: the params are pure `[IN]` so nothing is invented; UVM's init reads `GET`
    /// and `PUT` **once** and checks them against nothing
    /// (`ogkm-580: kernel-open/nvidia-uvm/uvm_gpu_replayable_faults.c:120-136`); and `[meas]`
    /// the C artifact answered this control `NV_OK` by fall-through
    /// (`C: src/qemu/nvkvm_gpu_emul.c:3057`) and reached `bad=0 maxerr=0` with the guest
    /// polling `PUT` seven times and reading `0` every time. *"Registered, and never a single
    /// fault"* is a state a real driver has been driven through to a correct matmul.
    ///
    /// ⊘ It stops being honest the first time a fault **should** be raised, and the guest
    /// gets a **hang** rather than an error. That is why serving it is paired with
    /// [`kayfabe_abi::faultbuffer::DELIVERY_UNBUILT`], which the boot's own end-of-run report
    /// prints whenever a registration was served.
    RegisterFaultBuffer,
    /// `NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_CLIENT_SHADOW_FAULT_BUFFER` (`0x20800a9d`) —
    /// ★★★ the rung [`Self::RegisterFaultBuffer`] exposed, and the one whose `NV_OK` is a
    /// **stronger** claim.
    ///
    /// `[measured 2026-08-09, boot `fb1503` at `3afa896`]` serving `0x20800a9b` moved the
    /// guest's failure to `faultbufCtrlCmdMmuFaultBufferRegisterNonReplayBuf_IMPL: Error
    /// allocating client shadow fault buffer for non-replayable faults` and put exactly this
    /// id in the unserviced ledger. The status is failure-transparent all the way up:
    /// `kgmmuClientShadowFaultBufferRegister` returns it verbatim
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:1815-1827`),
    /// `kgmmuClientShadowFaultBufferAllocate` unwinds, and `nvGpuOpsInitFaultInfo` jumps to
    /// `cleanup_fault_buffer` (`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:9457-9464`)
    /// — so `UVM_REGISTER_GPU` fails and `cuInit` with it.
    ///
    /// # ★★ Same reply shape, DIFFERENT argument for it
    ///
    /// All six fields are `[IN]`, the documented status set is `{NV_OK}`, and the caller
    /// re-reads nothing after the call — every `[OUT]` UVM consumes comes from its own local
    /// allocation (`ogkm-580: nv_gpu_ops.c:9466-9469`), not from this reply. So the identity
    /// on the guest's own 24 032 bytes is again the byte-accurate answer and nothing is
    /// fabricated.
    ///
    /// ⊘ But the *promise* is not the same one. For [`Self::RegisterFaultBuffer`] the guest
    /// polls a BAR0 register we serve; here **we are the declared writer** of a queue in the
    /// guest's own sysmem (`ogkm-580: kern_gmmu.c:1589-1593`), and on a GSP client the guest
    /// has no other route to a non-replayable fault at all. The full ruling, and what this
    /// port does *instead* (an RC plus an error notifier — `simulated_gpu_fault.md` §5.2's
    /// deliberate choice, and built), is in [`kayfabe_abi::faultbuffer`], and
    /// [`kayfabe_abi::faultbuffer::SHADOW_DELIVERY_UNBUILT`] is what the boot report prints.
    RegisterClientShadowFaultBuffer,
    /// `NV2080_CTRL_CMD_INTERNAL_UVM_REGISTER_ACCESS_CNTR_BUFFER` (`0x20800a1d`) — the third
    /// register-a-buffer control on the `UVM_REGISTER_GPU` path, and the only one that was
    /// **unreachable** until this port stopped serving zero at BAR0 `0xB83110`.
    ///
    /// `_uvmSetupAccessCntrBuffer` sends it only after `memdescCreate` has succeeded
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/uvm/uvm.c:39-81`), and that `memdescCreate` was
    /// failing on a size of zero — so this control has never appeared in any of this port's
    /// unserviced ledgers, and its absence there was evidence of nothing.
    ///
    /// ⚠ `[predicted from `ogkm-580`, NOT measured]` when it landed, deliberately: it is
    /// served in the same commit as the register so one boot adjudicates both. If the guest
    /// never sends it, the control census says so.
    ///
    /// Pure `[IN]`, `{NV_OK}`, identity echo — [`Self::RegisterFaultBuffer`]'s argument
    /// exactly. ⊘ The unbuilt half is [`kayfabe_abi::faultbuffer::ACCESS_COUNTER_DELIVERY_UNBUILT`]
    /// and it is the sharpest of the three: this is the buffer whose **size** this port also
    /// invents.
    RegisterAccessCntrBuffer,
    /// `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION` — ★★★ the first variant that is **not a
    /// table**, and the only one whose reply is a function of the *request* alone.
    ///
    /// # ⊘ Why it lives in this enum anyway
    ///
    /// Every variant above answers a description of silicon out of a
    /// [`ChipProfile`](kayfabe_device::ChipProfile) row. This one answers a **verb on the
    /// event plane**: the guest asks to arm a notifier, and there is no chip row that could
    /// have the answer. The name `WantedTable` fits it badly and that is a cost worth
    /// paying, because this enum is not merely a taxonomy — it is the universe every
    /// coverage gate quantifies over
    /// (`crates/kayfabe-crec/tests/cap1b_differential.rs::every_control_this_port_serves_is_exercised_by_the_replay`,
    /// and [`WantedTable::ALL`]'s own docs on why *"in `ALL`"* and *"served"* are one
    /// fact). A control served through a policy of its own would be a served control no
    /// differential could regress, which is the defect shape `ALL` exists to close.
    ///
    /// # ★★★ Refusing it is what stops the boot, and it is MEASURED
    ///
    /// `[measured]` two boots one commit apart, `docs/design/boot_measured_2026_08_01.md`
    /// §6 and §7: `alloc1` (`2ced035`) refused every object allocation, `alloc2`
    /// (`a6412c0`) served every one of them — and both died in `kbusInitBar2_HAL` with a
    /// null heap, identically. That differential is the only reason this can be stated as
    /// cause: `memmgrRegisterSuspendCallbacks` issues this control under
    /// `NV_ASSERT_OK_OR_RETURN` (`ogkm-580: mem_mgr.c:625`) and its caller
    /// `memmgrStateInitLocked_IMPL` rolls the whole phase back through `memmgrStateDestroy`
    /// (`:777`, `:963-975`), which deletes the heap it created ninety lines earlier.
    ///
    /// # ★★ And the reply may NOT be empty, which is why it is not `kayfabe_device::inert`
    ///
    /// The guest reads nothing but the status *from the control* — but
    /// `rpcRmApiControl_GSP` copies the reply's params back over the caller's struct
    /// (`ogkm-580: rpc.c:11085-11090`) and the caller then switches on
    /// `pSetEventParams->action`. An all-zero body silently rewrites `event = 194,
    /// action = REPEAT` into `event = 0, action = DISABLE` and returns `NV_OK`. See
    /// [`kayfabe_abi::eventnotify`], which carries the full reading and the
    /// [`SILENT_NOTIFIERS`](kayfabe_abi::eventnotify::SILENT_NOTIFIERS) scope that decides
    /// which armings this port may accept at all.
    EventSetNotification,
    /// `NV2080_CTRL_CMD_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT` — ★★★ the first variant that
    /// is neither a description of silicon nor a registration, but an **instruction to
    /// perform an action on hardware this device does not have**.
    ///
    /// # ⊘ Why this is a different kind of decision, and why it is not a generic `NV_OK`
    ///
    /// [`Self::EventSetNotification`] is a verb, but it is a verb about *bookkeeping* — the
    /// port can genuinely keep the arming it is asked to keep. This one asks for L2 to be
    /// invalidated and evicted, and **this emulated GPU has no L2**. The batched triage in
    /// [`crate::sweep`] classified it with `0x20800a70` as *"an ACTION, not a description —
    /// refuse"*, and that classification is what the `bar0win` boot falsified as a place to
    /// stop: the refusal propagates verbatim out of `kmemsysSendL2InvalidateEvict_IMPL`
    /// into `kbusVerifyBar2_GM107:4110-4115`, which prints *"L2 evict failed"* and takes its
    /// `goto`.
    ///
    /// # ★★★ The licence is that the postcondition already holds, structurally
    ///
    /// The operation's only observable is the read `kbusVerifyBar2_GM107` performs
    /// immediately afterwards (`ogkm-580: kern_bus_gm107.c:4106-4118`), and on this device
    /// that read cannot be stale: [`crate::fbwin`]'s store is the framebuffer's
    /// **authority**, not a cache over one, and the trapped write commits before the vmexit
    /// returns. ⊘ `NV_OK` is therefore *"the state you asked for already holds"*, which is a
    /// modelled yes with three named futures that would falsify it — see
    /// [`kayfabe_abi::l2evict`], which carries the full argument, the falsifiers, and the
    /// `[measured]` corroboration that a real GA106's own GSP answers `NV_OK` too.
    ///
    /// # ★★ The `0x20800301` trap, checked and NOT present
    ///
    /// The transport copies the reply's params back over the caller's struct
    /// (`ogkm-580: rpc.c:11085-11090`), which is why an empty body was wrong for
    /// [`Self::EventSetNotification`]. Here the caller's params are a **stack local** that
    /// `kmemsysSendL2InvalidateEvict_IMPL` never reads after the call
    /// (`ogkm-580: kern_mem_sys.c:1079-1093`), and the oracle's captured reply is four zero
    /// bytes (`C: mode2_initctrl_ga106.h:6245 = 0x20800a6c`, `psize = 4, dlen = 0`). Both sources agree
    /// on a zero-filled body, so that is what is encoded.
    MemsysL2InvalidateEvict,
    /// `NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE` (`0x20802a08`) — ★★★ the first
    /// variant whose reply this port **could not derive from any document it holds**, and
    /// the first answered with a number taken off a real GA106.
    ///
    /// # What refusing it costs, and why the old triage read it as free
    ///
    /// It was [`crate::sweep::SweepDisposition::RefusalIsInvisible`] for four rungs, on an
    /// argument that is *locally correct and globally wrong*:
    /// `gpuGetCeFaultMethodBufferSize_KERNEL` really does `return NV_OK` unconditionally and
    /// really does leave `*size` unwritten on failure (`ogkm-580: gpu.c:6031-6043`), so the
    /// refusal is invisible **as a status**. What that reading missed is that the invisible
    /// refusal leaves a **zero**, and the zero is not inert: it becomes the length argument
    /// to `memdescCreate` (`kernel_channel_group_gv100.c:109-110`), which rejects zero with
    /// `NV_ERR_INVALID_ARGUMENT` (`mem_desc.c:239-241`). That is the `0x25:0x1f:1249` the
    /// `irq1` boot ended on.
    ///
    /// ⚠ ★★★ **And the "halts" lesson lands differently here than it was written.** The
    /// `boot_measured_2026_08_01.md` §41.7 row says the `0x56` *"is fatal because a caller
    /// eleven frames up converts it"*. That is **refuted**: nothing converts it. The status
    /// is *discarded* one frame down, and eleven frames later an entirely **independent**
    /// `NV_ERR_INVALID_ARGUMENT` is manufactured from the zero it left behind. The
    /// distinction matters operationally — grepping for the propagation of `0x56` would
    /// never have found this, because there is none.
    ///
    /// # The value, and why it is on the chip row
    ///
    /// [`crate::ChipProfile::ce_fault_method_buffer_size`]. `[measured]` 20480 on a real
    /// RTX 3060; the argument for measuring rather than choosing, and for a chip row rather
    /// than a constant, is in [`kayfabe_abi::fmbsize`].
    ///
    /// # ★★ The `0x20800301` transport trap, checked and PRESENT
    ///
    /// Unlike [`Self::MemsysL2InvalidateEvict`], this caller **does** read its own params
    /// after the RPC returns — `*size = params.size;` on the very next line
    /// (`ogkm-580: kernel_ce.c:846`). So the body is load-bearing and an empty reply is not
    /// an option, which is exactly what the C oracle's captured row provides and exactly why
    /// that row could not be used.
    CeFaultMethodBufferSize,
    /// `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CAPS` — the first of the **five
    /// structurally mandatory** GR static-info controls, and the first this port answers
    /// about the shader core. See [`kayfabe_abi::grstatic`] for why these five and not the
    /// fourteen `kgraphicsLoadStaticInfo_KERNEL` issues, and for the ZCULL/ROP correction
    /// (their `0x56` is clobbered by the next call's assignment, so they are *not*
    /// mandatory however the `else if` reads).
    ///
    /// ★★★ Refusing any of the five is the sweep's signature failure at its purest: the
    /// refusal is silent (`gpu.c:3438` maps `NV_ERR_NOT_SUPPORTED` to `NV_OK`), GR's static
    /// info becomes permanently `NULL` (`kernel_graphics.c:1544`, `:556-564`), and the bill
    /// arrives twenty-one engines later inside **`KernelFifo`**'s `statePostLoad` as
    /// `NV_ERR_INVALID_STATE` (`kernel_graphics.c:485`) — which `gpu.c:3440` does not
    /// swallow. `[measured]` run `gmmu1` at `12b001f`: `RmInitAdapter failed!
    /// (0x25:0x40:1249)`.
    GrCaps,
    /// ★★★ `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_INFO` — GR's **legacy info list**, and
    /// the sixth GR reply rather than a sixth mandatory one. Its call site tolerates a
    /// refusal (`if (status == NV_OK)` with no `else`, `ogkm-580: kernel_graphics.c:1234`);
    /// a stranger twenty-one engines away does not. `kfifoGetMaxSubcontextFromGr_KERNEL`
    /// asserts `pGrInfo != NULL` and **returns 0** on failure (`kernel_fifo.c:2789-2792`),
    /// and that zero is what `kchangrpapiSetLegacyMode`'s `numMax != 0` rejects
    /// (`kernel_channel_group_api.c:913`).
    ///
    /// `[measured]` run `fmb1`, a stock 580.159.04 guest at `93191ee`
    /// (`/workspace/bench/run_fmb1_dmesg.log`): `RmInitAdapter failed! (0x25:0x40:1249)`.
    /// See [`kayfabe_abi::grinfo`].
    GrInfo,
    /// `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FLOORSWEEPING_MASKS` — ★ the one whose
    /// `gpcMask` is load-bearing twice over. `_kgraphicsPostSchedulingEnableHandler` returns
    /// `NV_OK` immediately when it is `0x0` (`kernel_graphics.c:486`), so a zero here would
    /// carry the boot past `gpuStatePostLoad` by *skipping* the golden-image channel. ⊘ That
    /// shortcut is named and rejected in [`kayfabe_abi::grstatic`]'s header; this device
    /// publishes `0x7`, which is what a GA106 has.
    GrFloorsweepingMasks,
    /// `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_GLOBAL_SM_ORDER` — 34 592 bytes, ★ the
    /// largest reply this port encodes, and nine of 580's 4 096-byte message-queue elements.
    /// It fits: the guest's receive staging buffer is `element_size_max` = 65 536
    /// (`kayfabe_abi::versions`), and `encode_message`'s `max_elements` guard is what says
    /// so rather than an assumption.
    GrGlobalSmOrder,
    /// `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FECS_RECORD_SIZE` — 32 bytes, one `NvU32`
    /// per engine. Mandatory (`NV_CHECK_OK_OR_GOTO`, `kernel_graphics.c:1467`).
    GrFecsRecordSize,
    /// `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_PDB_PROPERTIES` — 8 bytes, and the control
    /// whose success sets `bInitialized = NV_TRUE` on the very next line
    /// (`kernel_graphics.c:1521`). It is the last mandatory one, so it is the one that
    /// decides whether GR has static info at all.
    GrPdbProperties,
    /// `NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER` — ★★★
    /// the only control this port serves in which the guest is **telling us** something
    /// rather than asking: the physical addresses of the page-directory levels it reserved
    /// for the split VA space. See [`kayfabe_abi::gvaspacepdes`].
    ///
    /// ⚠ Its refusal is survivable (`gpuStatePostLoad` swallows the `0x56`), so it is served
    /// for what refusing *leaves behind* — a GPU group whose `pGlobalVASpace` was assigned
    /// before its constructor failed — and not for what it returns.
    GvaspaceServerReservedPdes,
    /// `NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES` (`0x90f10106`) — ★★★ **the same
    /// publication as [`WantedTable::GvaspaceServerReservedPdes`], from the arm of the same
    /// function that a real boot actually takes.**
    ///
    /// `gvaspaceCopyServerRmReservedPdesToServerRm_IMPL` branches on whether there is a
    /// resserv call context (`ogkm-580: gpu_vaspace.c:4058`): no context is the GPU group's
    /// global VAS and sends the `NV2080` wrapper; a context is a VA space under a client's
    /// **device**, and sends this id directly. Every device default VA space takes the
    /// second arm.
    ///
    /// ⚠ Serving one and not the other looked like completeness and was not. `[measured]`
    /// run `stateload2` at `7819839`: the `NV2080` id had been served for a rung and the
    /// boot still lost its device VA space, its CE utility channel and its framebuffer
    /// scrubber to this one — see [`kayfabe_abi::gvaspacepdes`] for the ten-line cascade
    /// off `/workspace/bench/run_stateload2_dmesg.log:12-30`.
    ///
    /// ⊘ It shares the decode, the validation and the re-encode with the `NV2080` arm and
    /// deliberately has no logic of its own: the payload is byte-identical
    /// (`ctrl2080internal.h:1906-1908` wraps exactly this one member), so a second copy
    /// could only ever disagree with the first.
    GvaspaceServerReservedPdesClient,
    /// `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CONTEXT_BUFFERS_INFO` — ★★★ the **sixth**
    /// mandatory GR static-info control, and the one this port refused to guess at.
    ///
    /// It sits behind `if (IS_MIG_IN_USE(pGpu) || !kgraphicsShouldDeferContextInit(...))`
    /// (`ogkm-580: kernel_graphics.c:1524-1529`), and `#150` deliberately left that
    /// predicate unevaluated rather than assume it — because `bInitialized = NV_TRUE` is set
    /// at `:1521`, *before* the branch, so the two outcomes are distinguishable in one boot.
    /// `[measured]` run `stateload1` at `041b4f1` settled it: the branch IS taken, the
    /// control IS issued, and refusing it sends the whole function to `cleanup:` — the same
    /// `NV_ERR_INVALID_STATE` in `KernelFifo`, from a different cause.
    GrContextBuffersInfo,
    /// `NV2080_CTRL_CMD_GPU_GET_INFO_V2` (`0x20800102`) — ★★★ the **first control this
    /// policy serves whose reply is a function of the request**, and the first one where
    /// most of the answer was already written by the guest's own kernel.
    ///
    /// `[measured 2026-08-08, real GA106]` refusing it makes `cuInit` return `100`. It is
    /// **co-equal** with admitting `NV2081_BINAPI`: neither alone changes `cuInit`'s answer,
    /// which is why they land together (`execution_plane_increments.md` §14.27's injection
    /// matrix).
    ///
    /// ⊘ **No fixed-body row can answer it, and no eleven-row table either.** The eleven
    /// `(index, value)` pairs §14.27 published are an *ioctl-boundary* reading; ten of them
    /// are resolved inside `getGpuInfos`'s own `switch` and never reach a GSP, and only the
    /// entries carrying `INDEX_FORWARD_TO_PHYSICAL` (bit 31) are ours to fill. The whole
    /// derivation, the three recorded GSP-level calls it rests on, and the two per-chip
    /// identity indices this port refuses by name are in [`kayfabe_abi::gpuinfo`].
    ///
    /// ⚠ The guest's `gpuInfoListSize` is a **guest-supplied count used as a loop bound over
    /// a buffer**; it is bounded against `NV2080_CTRL_GPU_INFO_MAX_LIST_SIZE` before it
    /// indexes anything, exactly as RM bounds it.
    GpuInfoV2,
    /// `NV2080_CTRL_CMD_INTERNAL_GPU_GET_SMC_MODE` (`0x20800a4c`) — ★★★★ **the control that
    /// decided `cuInit`**, and the one every instrument this port owns was structurally
    /// unable to see.
    ///
    /// # `[measured 2026-08-08, boot `gis1_e6ed6bc`]` Why it is not optional
    ///
    /// It is not asked during `RmInitAdapter` — `docs/reference/remaining_boot_surface.md`
    /// §1 proved that by set-difference and concluded the row was therefore *"by definition
    /// not part of init"*. Correct, and the wrong thing to conclude: libcuda asks
    /// `GPU_GET_INFO_V2` for index `0x2a` (`GPU_SMC_MODE`), whose arm issues **this** control
    /// on the physical RMAPI and assigns its status to the enclosing loop
    /// (`ogkm-580: subdevice_ctrl_gpu_kernel.c:232-266`). The loop `break`s on the first
    /// non-`NV_OK` and returns it **for the whole call** (`:566-569`).
    ///
    /// ⇒ Refusing this one internal control fails an eleven-index request in which the other
    /// **ten indices were already answered correctly** — and it fails it *inside the guest
    /// kernel*, with no RPC, so [`Self::GpuInfoV2`]'s own ledger row stayed green while
    /// `cuInit` returned 100. The in-guest bisect that named it is `SWEEPIDX pos=3
    /// idx=0x2a status=0x56` with all ten others `NV_OK`, and `SWEEPPFX` breaking at exactly
    /// `len=4` (`scripts/bench/guest_gpuinfo_sweep.sh`).
    ///
    /// ⊘ That also refutes the RM **control cache** as the cause: the failure depends
    /// entirely on *which* index is asked, on the same handle at the same instant, and a
    /// cache hit cannot. This control's own flags are `0xc0`
    /// (`ogkm-580: g_subdevice_nvoc.c:2530-2540`) — no `RMCTRL_FLAGS_CACHEABLE_*` bit — so it
    /// is not a [`crate::sticky::BRANCH_A_CACHEABLE`] row either.
    ///
    /// # The value
    ///
    /// [`crate::ChipProfile::smc_mode`], `[measured]` `Unsupported` on two physical GA106
    /// parts by two different instruments. ⊘ **Not** from the C oracle, whose row for this id
    /// is one of the eleven `dlen = 0` rows; the argument is in [`kayfabe_abi::smcmode`].
    InternalGpuGetSmcMode,
    /// `NV2080_CTRL_CMD_BUS_GET_INFO_V2` (`0x20801823`) — ★★★ the wall §14.29 left standing,
    /// and the **first value this port serves that is not a fact about the chip**.
    ///
    /// `[measured 2026-08-08, boot `v1429_49b182a`]` `cuInit` reaches this control's
    /// **second** call and gets `0x56` from it. Of its six indices exactly one is
    /// RPC-forwarded on a GSP client — `0x2d` `PCIE_GEN_INFO`
    /// (`ogkm-580: kern_bus_ctrl.c:283-334`) — and the other five are the guest kernel's own,
    /// the same one-of-N shape as [`Self::GpuInfoV2`]'s ten-of-eleven.
    ///
    /// ⊘ **No chip row may state `0x2d`, and that is MEASURED rather than argued.** The same
    /// physical GA106 answered `0x00302000` with its link idle and `0x00322000` with the link
    /// loaded, seconds apart: two of the word's three generation fields belong to the slot
    /// and to the live link, and only `GPU_GEN` belongs to the die. So the chip row states
    /// one enum — [`crate::ChipProfile::pcie_max_gen`] — and the word is DERIVED from it by
    /// [`kayfabe_abi::businfo::PcieGenInfo::fully_trained`]. The whole measurement, and the
    /// named residual (this describes the link this port presents, not the host's), are in
    /// [`kayfabe_abi::businfo`].
    ///
    /// ⚠ Its flags are `0x10118` (`ogkm-580: g_subdevice_nvoc.c:6700-6712`) — neither
    /// `RMCTRL_FLAGS_CACHEABLE` (`0x400`) nor `_CACHEABLE_BY_INPUT` (`0x20000`) — so unlike
    /// [`Self::GpuInfoV2`] it is **not** a [`crate::sticky::BRANCH_A_CACHEABLE`] row, and a
    /// value that moves with the link is never cached by the guest for the life of the boot.
    BusGetInfoV2,
    /// `NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS` (`0x2080182a`) — ★★★ the wall
    /// §14.30 left standing, and the one whose **instrument was the wall**.
    ///
    /// `[measured 2026-08-08, boot `gt1430_0dbbabc`]` `cuInit` reaches this control and gets
    /// `0x56`; the boot ledger carries `unserviced fn 76 cmd 0x2080182a` exactly once,
    /// because its flags `0x40048` include `ROUTE_TO_PHYSICAL` and the whole 112-byte struct
    /// is RPC'd to a GSP as one call (`ogkm-580: g_subdevice_nvoc.c:6806-6819`).
    ///
    /// ⊘ §14.30 recorded that `rmladder --probe-ctrl` was refused `0x56` on the same
    /// physical part that answers libcuda `NV_OK`, and inferred caller-dependence from the
    /// `_DISPATCH` suffix. **Refuted**: `capType` is an `[IN]` field and `probe_ctrl` seeds
    /// every byte `0xCD`, so the probe asked an undeclared captype. `[measured 2026-08-08,
    /// real GA106, `rmladder --atomics-probe` (R23)]` the **same bare Subdevice** answers
    /// `NV_OK` for `capType = SYSMEM`. The whole eight-arm measurement, the value, and why
    /// this zero is not the `0x20802a08` zero are in [`kayfabe_abi::gpuatomics`].
    ///
    /// ⚠ Its flags carry neither `RMCTRL_FLAGS_CACHEABLE` (`0x400`) nor `_CACHEABLE_BY_INPUT`
    /// (`0x20000`), so like [`Self::BusGetInfoV2`] it is **not** a
    /// [`crate::sticky::BRANCH_A_CACHEABLE`] row.
    BusGetPcieSupportedGpuAtomics,
    /// `NV2080_CTRL_CMD_FB_GET_INFO_V2` (`0x20801303`) — ★★★ the wall §14.31 named, and the
    /// first this port serves that **states no new number at all**.
    ///
    /// `[measured 2026-08-08, boot `gt1431_ff7a0ea`]` `cuInit` asks this control four times.
    /// The first three are answered `NV_OK` by the guest's own kernel and never reach us;
    /// the fourth — seven indices, byte-identical to a request a real GA106 answers `NV_OK`
    /// (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:50`) — is `0x56`.
    ///
    /// ⊘⊘ **It is absent from both boot ledgers, and that is NOT because it never arrives.**
    /// `[measured 2026-08-09]` that boot's summary lines read `67 UNSERVICED … 32 distinct`
    /// and `101 answered, 32 distinct cmd/result rows` — **both at their caps**
    /// ([`crate::unserviced::UNSERVICED_SAMPLE_MAX`],
    /// `kayfabe_qemu_raw::shim::SERVED_CONTROL_SLOTS`), so every command first seen after
    /// the thirty-second was dropped without a word. §14.31 read the miss as *"the guest
    /// kernel refuses it from its own state"*; the RPC does go out. See
    /// [`kayfabe_abi::fbinfo`] for the whole refutation.
    ///
    /// ★ Three of the seven indices really *are* the guest kernel's own
    /// (`ogkm-580: kern_mem_sys_ctrl.c:335, 711, 716`) and four are forwarded — and unlike
    /// [`Self::BusGetInfoV2`] the forward is **one compacted RPC**, not one per index, so the
    /// request this policy sees carries four entries and not seven. All four are
    /// **projections of [`crate::ChipProfile::memory_system`]**, the row already served to
    /// `0x20800a1c`; nothing here is a second description of the same silicon.
    ///
    /// ⚠ Its flags are `0x10118` (`ogkm-580: g_subdevice_nvoc.c:5845-5859`) — the same word
    /// [`Self::BusGetInfoV2`] carries, and neither `RMCTRL_FLAGS_CACHEABLE` (`0x400`) nor
    /// `_CACHEABLE_BY_INPUT` (`0x20000`) — so it is not a [`crate::sticky::BRANCH_A_CACHEABLE`]
    /// row.
    FbGetInfoV2,
    /// `NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS` (`0x20802a0b`) — ★★★ §14.32's wall, and
    /// **the id that fails is not this one**.
    ///
    /// `[measured 2026-08-08, boot `gt1432_20e319b`]` `cuInit` fails at
    /// `NV2080_CTRL_CMD_CE_GET_ALL_CAPS` (`0x20802a0a`), which this port must **not** serve:
    /// `subdeviceCtrlCmdCeGetAllCaps_IMPL` is the guest kernel's own
    /// (`ogkm-580: kernel_ce_shared.c:282-336`), and it reaches an emulated GSP only as its
    /// forward of the **physical** id under `NV_ASSERT_OK_OR_RETURN`. Serving `0x20802a0a`
    /// would be answering a boundary the guest never asks us about.
    ///
    /// ★ And unlike §14.31's, this rung's ledger silence is *trustworthy in the other
    /// direction*: `0x20802a0b` **is** in boot `gt1432_20e319b`'s unserviced list
    /// (`34 distinct`, no truncation line), one of the two rows the cap raise made visible.
    /// The repaired instrument produced the target directly, having spent two rungs hiding
    /// it.
    ///
    /// ⊘⊘ The reply is `[OUT]`-only and **constructed, not edited** — the first such arm.
    /// Every byte is a projection of [`crate::ChipProfile::engines`], the same slice
    /// [`Self::DeviceInfo`] and [`Self::InternalDeviceInfo`] serve; `present` is that
    /// slice's `DEV_TYPE_ENUM_LCE` rows and the one per-CE caps bit is
    /// `NV_CE_GRCE_ALLOWED_LCE_MASK` intersected with them. See [`kayfabe_abi::cecaps`] for
    /// the two refutations of §14.32 this cost, for why the probe it specified cannot run,
    /// and for why serving the caller-observed bytes is provably right rather than assumed.
    ///
    /// ⚠ Its flags are `0x101d0` (`ogkm-580: g_subdevice_nvoc.c:7705-7718`) — carrying
    /// `ROUTE_TO_PHYSICAL` and `INTERNAL`, and **neither** `RMCTRL_FLAGS_CACHEABLE`
    /// (`0x400`) nor `_CACHEABLE_BY_INPUT` (`0x20000`) — so it is not a
    /// [`crate::sticky::BRANCH_A_CACHEABLE`] row either.
    CeGetAllPhysicalCaps,
    /// `NV2080_CTRL_CMD_CE_GET_PHYSICAL_CAPS` (`0x20802a07`) — ★★★ §14.42's wall, **half of
    /// it**, and the id that fails is again not this one.
    ///
    /// `[measured 2026-08-09, boot `ac1710` at `1ea422f`]` `cuInit` dies in
    /// `queryCopyEngines` (`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:8449-8541`).
    /// Its per-CE loop calls `NV2080_CTRL_CMD_CE_GET_CAPS` (`0x20802a01`), which this port
    /// must **not** serve: `subdeviceCtrlCmdCeGetCaps_IMPL` is the guest kernel's own
    /// (`kernel_ce_ctrl.c:46-87`) and reaches an emulated GSP only as `kceGetDeviceCaps_IMPL`'s
    /// forward of the **physical** id, under `NV_ASSERT_OK_OR_RETURN` (`kernel_ce.c:551-556`).
    /// [`Self::CeGetAllPhysicalCaps`]'s situation exactly, one control along.
    ///
    /// ⊘⊘ **Re-asked, not inherited.** The three rungs before this one served *pure-`[IN]`*
    /// controls where the identity echo was the whole correct reply. That argument does
    /// **not** reach here: `NV2080_CTRL_CE_GET_CAPS_V2_PARAMS` is `{ NvU32 ceEngineType
    /// /*[IN]*/; NvU8 capsTbl[2] /*[OUT]*/; }` (`ogkm-580: ctrl2080ce.h:82-85`, typedef'd for
    /// this id at `:279`) and the caller `portMemCopy`s our two `[OUT]` bytes **verbatim**
    /// into the guest's per-CE capabilities. Echoing would hand it the `0xCD`-equivalent of
    /// whatever the guest's buffer held.
    ///
    /// ★★★ And the two bytes state **no new number**: they are `geometry.caps_for(publicID)`,
    /// the identical [`kayfabe_abi::cecaps::CeGeometry`] row [`Self::CeGetAllPhysicalCaps`]
    /// emits whole. One silicon, two doors, one description — a device whose CE2 is a
    /// graphics copy engine under one control id and is not under another is precisely the
    /// drift [`kayfabe_abi::deviceinfo`] exists to forbid.
    ///
    /// ⚠ Its flags are `0x301d0` (`ogkm-580: g_subdevice_nvoc.c:7645-7658`) — carrying
    /// `ROUTE_TO_PHYSICAL` and `INTERNAL` and **neither** `PRIVILEGED(0x4)` nor
    /// `NON_PRIVILEGED(0x8)`, i.e. `KERNEL_PRIVILEGED`, which is why it could not be probed
    /// on a real part and had to be derived. Neither `RMCTRL_FLAGS_CACHEABLE` (`0x400`) nor
    /// `_CACHEABLE_BY_INPUT` (`0x20000`), so not a [`crate::sticky::BRANCH_A_CACHEABLE`] row.
    CeGetPhysicalCaps,
    /// `NV2080_CTRL_CMD_CE_GET_CE_PCE_MASK` (`0x20802a02`) — ★★★ the **other half** of
    /// §14.42's wall, six lines below the first, and the one control in this rung whose value
    /// was **measured at its own boundary** instead of derived.
    ///
    /// `queryCopyEngines` issues it immediately after `CE_GET_CAPS` for the same engine and
    /// `goto done`s on any status but `NV_OK` (`nv_gpu_ops.c:8519-8531`), so serving only
    /// [`Self::CeGetPhysicalCaps`] would have moved the wall by six lines. ⇒ Both land in one
    /// rung, deliberately.
    ///
    /// ★ Unlike its neighbour this id carries `NON_PRIVILEGED(0x8)` (flags `0x30349`,
    /// `ogkm-580: g_subdevice_nvoc.c:7585-7598`) **and** `ROUTE_TO_PHYSICAL` with no body in
    /// the vendored tree — reachable and unreadable. So a real part was asked:
    /// `[measured 2026-08-09, real GA106 `GPU-d0913685`, R24,
    /// `traces/real_ga106/rmladder_r24_pcemask_real_ga106.txt`]` LCE0..3 answer
    /// `0x20, 0x10, 0x10, 0x20` and **LCE4 refuses `0x56`** — the same engine count
    /// [`kayfabe_abi::cecaps`] measured through two other callers, corroborated here by a
    /// third control that shares no code with them.
    ///
    /// ⊘ The value is the chip row's ([`crate::ChipProfile::lce_pce_masks`]), not a constant
    /// here, because a PCE→LCE map is a per-part fact. An engine this device advertises with
    /// no mask stated is refused **by name** rather than answered zero.
    ///
    /// ⚠ Flags `0x30349` carry neither `RMCTRL_FLAGS_CACHEABLE` (`0x400`) nor
    /// `_CACHEABLE_BY_INPUT` (`0x20000`), so not a [`crate::sticky::BRANCH_A_CACHEABLE`] row.
    CeGetCePceMask,
    /// `NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO` (`0x20803801`) — ★★★ §14.33's wall, and the
    /// first control this port serves whose errors are **per-item, not per-call**.
    ///
    /// `[measured 2026-08-09, boot `gt1433_0de5ddb`]` `cuInit` reaches this as its 63rd
    /// call and gets `0x56`; a real GA106 answers `NV_OK`
    /// (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:64`).
    ///
    /// ⊘ **No id translation.** Unlike [`Self::CeGetAllPhysicalCaps`], the id that fails is
    /// the id to serve: flags `0x10248` carry `ROUTE_TO_PHYSICAL`, which compiles
    /// `subdeviceCtrlCmdGrmgrGetGrFsInfo_IMPL`'s pointer to `NULL`
    /// (`ogkm-580: control.h:159-161`) and RPCs `pParams->cmd` unmodified
    /// (`resource.c:255-291`). The body is not in the open tree at all.
    ///
    /// ★★★ Its batch is **fault tolerant per query** (`ogkm-580: ctrl2080grmgr.h:42-50`):
    /// a structural fault fails the call, a query-specific one is logged in that query's own
    /// `status` and the loop marches on. ⚠ Which means a per-query refusal rides inside an
    /// `NV_OK` reply and reaches **no ledger this port keeps** — so
    /// [`kayfabe_abi::grfsinfo`] refuses per-query only where RM itself does, and takes the
    /// whole control down for any type it merely does not model.
    ///
    /// ⚠ Flags `0x10248` carry neither `RMCTRL_FLAGS_CACHEABLE` (`0x400`) nor
    /// `_CACHEABLE_BY_INPUT` (`0x20000`), so not a [`crate::sticky::BRANCH_A_CACHEABLE`] row.
    GrmgrGetGrFsInfo,
    /// `NV2080_CTRL_CMD_GSP_GET_FEATURES` (`0x20803601`) — ★★★ §14.35's wall, and the
    /// first control this port serves whose reply is a fact about the **guest** rather
    /// than about the silicon.
    ///
    /// `[measured 2026-08-09, boot `gt1434_373c145`]` `unserviced fn 76 cmd 0x20803601`;
    /// a real GA106 answers `NV_OK` with all four fields set
    /// (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:73`).
    ///
    /// ⊘ **No id translation, and no local body to race.** Flags `0x40549` carry
    /// `ROUTE_TO_PHYSICAL`, and the generated dispatch installs
    /// `subdeviceCtrlCmdGspGetFeatures_92bfc3` — a bare `NV_ERR_NOT_SUPPORTED` stub — on
    /// every `RmVariantHal` except `VF` (`ogkm-580: g_subdevice_nvoc.c:10711-10719`,
    /// `g_subdevice_nvoc.h:8017-8020`). A bare-metal GSP client is not `VF`, so `NV_OK`
    /// here can only ever have come off the RPC. See [`kayfabe_abi::gspfeatures`] for why
    /// this is a stronger statement than §14.35's prologue argument.
    ///
    /// ★★★ Its `firmwareVersion` is **latched from the guest's own fn 1**, not projected
    /// from a constant: [`InitTablePolicy::guest_firmware`]. The two candidates that look
    /// right and are not — the host driver's version, and this policy's own
    /// `DriverAbiTable::version()` (`[measured]` `580.65.06`, not `580.159.04`) — are laid
    /// out in [`kayfabe_abi::gspfeatures`]'s module docs.
    ///
    /// ⚠ Flags `0x40549` **do** carry `RMCTRL_FLAGS_CACHEABLE` (`0x400`), so unlike every
    /// row above this one it **is** a [`crate::sticky::BRANCH_A_CACHEABLE`] row — the
    /// first one this port serves. The decision that branch forces is made in
    /// [`kayfabe_abi::gspfeatures`]'s docs: the answer is constant for the life of a
    /// driver load, so caching it is correct.
    GspGetFeatures,
    /// `0x20808159` — ★★★ §14.36's wall, and **the first GSS-legacy id this port answers**.
    ///
    /// `[measured 2026-08-09, boot `gf1435` at `d24ad77`]` `cuInit` reaches it as row 80 of
    /// 87 and gets `0x56`, after which every remaining row is this port's teardown; a real
    /// GA106 answers `NV_OK` and runs eight further calls
    /// (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:80`).
    ///
    /// ⊘ It is in **no** NVOC table and **no** SDK header, so there is no `ROUTE_TO_PHYSICAL`
    /// flag to read: it is ours because bit 15 makes `_nv04ControlWithSecInfo` bypass resserv
    /// entirely and hand the raw buffer to physical RM under the same id
    /// (`ogkm-580: rmapi_deprecated_control.c:97`, `rmapi_gss_legacy_control.c`).
    ///
    /// ★★★ The reply is the **request, unchanged** — and that is a measurement rather than
    /// an echo, because this path's copy-out is unconditional on `NV_OK` and no
    /// `SKIP_COPYOUT` flag exists on it. [`kayfabe_abi::gsslegacy`] carries the argument, and
    /// carries why this does **not** relax `kayfabe-rmrpc`'s refusal of GSS-legacy commands
    /// in general: a rule permits a command to be named, only a measurement permits it to be
    /// answered.
    GssLegacy8159,
    /// `0x20808162` — §14.37, the **second** GSS-legacy id, and ⊘ **not** an identity.
    ///
    /// `[measured 2026-08-09, boot `gf1436` at `ec434b8`]` row 85 of 87 gets `0x56`; a real
    /// GA106 answers `NV_OK` with `in=00 out=01`
    /// (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:85`), and the C artifact records
    /// the same byte independently.
    ///
    /// ⚠⚠ Its branch-(b) argument is **different** from [`Self::GssLegacy8159`]'s and must not
    /// be copied from it: that one is safe under a cache because its reply is the guest's own
    /// buffer, while this one writes a byte the guest did not send. Its safety is entirely
    /// [`crate::sticky::StickyAnswerGuard`]'s — see [`kayfabe_abi::gsslegacy`].
    GssLegacy8162,
    /// `NV2080_CTRL_CMD_BUS_GET_C2C_INFO` (`0x2080182b`) — §14.37, and ★★ the one served row
    /// whose value is right by **argument** rather than by capture.
    ///
    /// `[measured 2026-08-09, boot `gf1436` at `ec434b8`]` row 86 of 87 gets `0x56`; a real
    /// GA106 answers `NV_OK`. Flags `0x50048` carry `ROUTE_TO_PHYSICAL`
    /// (`ogkm-580: g_subdevice_nvoc.c:6826`), so it is ours.
    ///
    /// ⊘ The trace's all-zero reply is corroboration, **not** the source: an all-zero row is
    /// the shape this repository refuses to decode. `bIsLinkUp = false` is true of a GA106 on
    /// first principles — C2C is a Grace-Hopper-class fabric and this die has none — so the
    /// argument survives the capture being deleted. See [`kayfabe_abi::c2cinfo`].
    C2cInfo,
    /// `NVA06C_CTRL_CMD_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS` (`0xa06c010a`) — ★★★ §14.43's
    /// wall, and **the first row in this table that is not a subdevice control at all**.
    ///
    /// `[measured 2026-08-09, boot `ce1442` at `8ea44dc`]` `cuInit` gets past
    /// `queryCopyEngines` for the first time and dies six lines into
    /// `kchangrpapiConstruct_IMPL` instead:
    /// `NVRM: kchangrpapiConstruct_IMPL: Control call to update method buffer memdesc failed`,
    /// with `unserviced fn 76 cmd 0xa06c010a` in the same boot's census
    /// (`traces/guest_boots/ce1442_8ea44dc_census.log:42`). The `NV_PRINTF` is followed by a
    /// hard `goto failed` (`ogkm-580: kernel_channel_group_api.c:492-505`), so the channel
    /// group — the TSG every UVM channel and every compute channel hangs off — never exists.
    ///
    /// ★★★ It is `KERNEL_PRIVILEGED` (flags `0x14240`,
    /// `ogkm-580: g_kernel_channel_group_api_nvoc.c:326-341`), so it cannot be measured on a
    /// real part — and it needs no measurement, because **every field is `[input]`**. This is
    /// the one place in this table where "derive, never invent" resolves to *there is nothing
    /// to state*: the reply is the guest's own facts, re-encoded from what the decoder
    /// accepted. See [`kayfabe_abi::fmbpromote`] for the honesty question re-asked per id, for
    /// why the `_v1E_07` RPC is a decoy, and for the compiler-pinned layout at both tags.
    ///
    /// ⊘ The refusal arm is **not** defensive: it is C defect **D1**'s habitat. A guest
    /// declaring more runqueues than the two-element array holds is refused by name rather
    /// than clamped, and a buffer whose aperture this port cannot name is refused rather than
    /// folded into sysmem.
    PromoteFaultMethodBuffers,
}

impl WantedTable {
    /// ★★ **Every control this policy serves**, as a value a caller can quantify over.
    ///
    /// A test that wants to say *"the differential exercises every served control"* has to
    /// get the universe from somewhere, and a list written in the test is the defect shape
    /// this repository has been bitten by most: shortening it weakens the gate with zero
    /// red tests. The list lives here, next to the `match` that consumes it.
    ///
    /// ## ★★★ Why this array is the served universe BY CONSTRUCTION, not by a test
    ///
    /// It used to be one of two lists — this array, and [`WantedTable::from_cmd`]'s `match`
    /// — and `tests/init_tables.rs` claimed the round trip kept them in step: *"a variant
    /// that has an id but is missing from `ALL` fails here"*. ⊘ **It could not.** That test
    /// iterates `ALL`, so a variant absent from `ALL` is never visited by it; a new variant
    /// with a `cmd_id` arm and a `from_cmd` arm but no row here compiled, served, and left
    /// every gate quantified over `ALL` — the sticky-answer property below, and
    /// `kayfabe-crec`'s reply-plane differential — silently one control short. That is the
    /// same defect shape as PC-D6: a load-bearing rationale that is false.
    ///
    /// The two lists are now **one**. `from_cmd` is a lookup *through this array*, so
    /// *"in `ALL`"* and *"served"* are the same fact rather than two statements that agree
    /// today. A variant left out is not merely untested — it is **not served**, and the
    /// guest gets this port's ordinary named refusal at that rung, which is loud and costs
    /// one boot. ⊘ Deliberately the safe direction: the failure of forgetting a row is a
    /// refusal, never an unchecked answer.
    ///
    /// [`WantedTable::cmd_id`] remains the mechanism on the other side — exhaustive over
    /// `Self`, so a new variant does not compile until it has an id.
    pub const ALL: [WantedTable; 41] = [
        Self::DeviceInfo,
        Self::IntrKernelTable,
        Self::PciBarInfo,
        Self::ChipInfo,
        Self::UserRegisterAccessMap,
        Self::ConstructedFalconInfo,
        Self::MemorySystemStaticConfig,
        Self::InternalDeviceInfo,
        Self::ConfComputeStaticInfo,
        Self::BifStaticInfo,
        Self::FifoNumChannels,
        Self::GmmuStaticInfo,
        Self::RegisterFaultBuffer,
        Self::RegisterClientShadowFaultBuffer,
        Self::RegisterAccessCntrBuffer,
        Self::EventSetNotification,
        Self::MemsysL2InvalidateEvict,
        Self::CeFaultMethodBufferSize,
        Self::GrCaps,
        Self::GrInfo,
        Self::GrFloorsweepingMasks,
        Self::GrGlobalSmOrder,
        Self::GrFecsRecordSize,
        Self::GrPdbProperties,
        Self::GvaspaceServerReservedPdes,
        Self::GvaspaceServerReservedPdesClient,
        Self::GrContextBuffersInfo,
        Self::GpuInfoV2,
        Self::InternalGpuGetSmcMode,
        Self::BusGetInfoV2,
        Self::BusGetPcieSupportedGpuAtomics,
        Self::FbGetInfoV2,
        Self::CeGetAllPhysicalCaps,
        Self::CeGetPhysicalCaps,
        Self::CeGetCePceMask,
        Self::GrmgrGetGrFsInfo,
        Self::GspGetFeatures,
        Self::GssLegacy8159,
        Self::GssLegacy8162,
        Self::C2cInfo,
        Self::PromoteFaultMethodBuffers,
    ];

    /// The control id this table answers — and the **only** place an id is stated.
    ///
    /// ★ An exhaustive `match`, which is the mechanism: adding a variant to this enum stops
    /// the crate compiling until the id is stated. [`WantedTable::from_cmd`] is derived
    /// from this and [`WantedTable::ALL`], so an id cannot be written down twice and cannot
    /// disagree with itself.
    #[must_use]
    pub fn cmd_id(self) -> u32 {
        match self {
            Self::DeviceInfo => NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE,
            Self::IntrKernelTable => NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE,
            Self::PciBarInfo => NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO,
            Self::ChipInfo => NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO,
            Self::UserRegisterAccessMap => {
                NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP
            }
            Self::ConstructedFalconInfo => NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO,
            Self::MemorySystemStaticConfig => NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG,
            Self::InternalDeviceInfo => NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE,
            Self::ConfComputeStaticInfo => NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_GET_STATIC_INFO,
            Self::BifStaticInfo => NV2080_CTRL_CMD_INTERNAL_BIF_GET_STATIC_INFO,
            Self::FifoNumChannels => NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_CHANNELS,
            Self::GmmuStaticInfo => NV2080_CTRL_CMD_INTERNAL_GMMU_GET_STATIC_INFO,
            Self::RegisterFaultBuffer => {
                kayfabe_abi::faultbuffer::NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER
            }
            Self::RegisterClientShadowFaultBuffer => {
                kayfabe_abi::faultbuffer::NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_CLIENT_SHADOW_FAULT_BUFFER
            }
            Self::RegisterAccessCntrBuffer => {
                kayfabe_abi::faultbuffer::NV2080_CTRL_CMD_INTERNAL_UVM_REGISTER_ACCESS_CNTR_BUFFER
            }
            Self::EventSetNotification => {
                kayfabe_abi::eventnotify::NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION
            }
            Self::MemsysL2InvalidateEvict => {
                kayfabe_abi::l2evict::NV2080_CTRL_CMD_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT
            }
            Self::CeFaultMethodBufferSize => {
                kayfabe_abi::fmbsize::NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE
            }
            Self::GrCaps => grstatic::NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CAPS,
            Self::GrInfo => kayfabe_abi::grinfo::NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_INFO,
            Self::GrFloorsweepingMasks => {
                grstatic::NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FLOORSWEEPING_MASKS
            }
            Self::GrGlobalSmOrder => {
                grstatic::NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_GLOBAL_SM_ORDER
            }
            Self::GrFecsRecordSize => {
                grstatic::NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FECS_RECORD_SIZE
            }
            Self::GrPdbProperties => {
                grstatic::NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_PDB_PROPERTIES
            }
            Self::GvaspaceServerReservedPdes => {
                gvaspacepdes::NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER
            }
            Self::GvaspaceServerReservedPdesClient => {
                gvaspacepdes::NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES
            }
            Self::GrContextBuffersInfo => {
                grstatic::NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CONTEXT_BUFFERS_INFO
            }
            Self::GpuInfoV2 => NV2080_CTRL_CMD_GPU_GET_INFO_V2,
            Self::InternalGpuGetSmcMode => {
                kayfabe_abi::smcmode::NV2080_CTRL_CMD_INTERNAL_GPU_GET_SMC_MODE
            }
            Self::BusGetInfoV2 => kayfabe_abi::businfo::NV2080_CTRL_CMD_BUS_GET_INFO_V2,
            Self::BusGetPcieSupportedGpuAtomics => {
                kayfabe_abi::gpuatomics::NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS
            }
            Self::FbGetInfoV2 => kayfabe_abi::fbinfo::NV2080_CTRL_CMD_FB_GET_INFO_V2,
            Self::CeGetAllPhysicalCaps => {
                kayfabe_abi::cecaps::NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS
            }
            Self::CeGetPhysicalCaps => kayfabe_abi::cecaps::NV2080_CTRL_CMD_CE_GET_PHYSICAL_CAPS,
            Self::CeGetCePceMask => kayfabe_abi::cepce::NV2080_CTRL_CMD_CE_GET_CE_PCE_MASK,
            Self::GrmgrGetGrFsInfo => kayfabe_abi::grfsinfo::NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO,
            Self::GspGetFeatures => kayfabe_abi::gspfeatures::NV2080_CTRL_CMD_GSP_GET_FEATURES,
            Self::GssLegacy8159 => kayfabe_abi::gsslegacy::GSS_LEGACY_0X8159,
            Self::GssLegacy8162 => kayfabe_abi::gsslegacy::GSS_LEGACY_0X8162,
            Self::C2cInfo => kayfabe_abi::c2cinfo::NV2080_CTRL_CMD_BUS_GET_C2C_INFO,
            Self::PromoteFaultMethodBuffers => {
                kayfabe_abi::fmbpromote::NVA06C_CTRL_CMD_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS
            }
        }
    }

    /// The `[OUT]` struct size RM allocates for this table.
    #[must_use]
    pub fn params_size(self) -> usize {
        match self {
            Self::DeviceInfo => DEVICE_INFO_PARAMS_SIZE,
            Self::IntrKernelTable => INTR_PARAMS_SIZE,
            Self::PciBarInfo => PCI_BAR_INFO_PARAMS_SIZE,
            Self::ChipInfo => CHIP_INFO_PARAMS_SIZE,
            Self::UserRegisterAccessMap => USER_REGISTER_ACCESS_MAP_PARAMS_SIZE,
            Self::ConstructedFalconInfo => FALCON_INFO_PARAMS_SIZE,
            Self::MemorySystemStaticConfig => MEMSYS_STATIC_CONFIG_PARAMS_SIZE,
            Self::InternalDeviceInfo => INTERNAL_DEVICE_INFO_PARAMS_SIZE,
            Self::ConfComputeStaticInfo => CONF_COMPUTE_STATIC_INFO_PARAMS_SIZE,
            Self::BifStaticInfo => BIF_STATIC_INFO_PARAMS_SIZE,
            Self::FifoNumChannels => FIFO_NUM_CHANNELS_PARAMS_SIZE,
            Self::GmmuStaticInfo => GMMU_STATIC_INFO_PARAMS_SIZE,
            Self::RegisterFaultBuffer => {
                kayfabe_abi::faultbuffer::REGISTER_FAULT_BUFFER_PARAMS_SIZE
            }
            Self::RegisterClientShadowFaultBuffer => {
                kayfabe_abi::faultbuffer::REGISTER_CLIENT_SHADOW_FAULT_BUFFER_PARAMS_SIZE
            }
            Self::RegisterAccessCntrBuffer => {
                kayfabe_abi::faultbuffer::REGISTER_ACCESS_CNTR_BUFFER_PARAMS_SIZE
            }
            Self::EventSetNotification => {
                kayfabe_abi::eventnotify::EVENT_SET_NOTIFICATION_PARAMS_SIZE
            }
            Self::MemsysL2InvalidateEvict => kayfabe_abi::l2evict::L2_INVALIDATE_EVICT_PARAMS_SIZE,
            Self::CeFaultMethodBufferSize => {
                kayfabe_abi::fmbsize::CE_FAULT_METHOD_BUFFER_SIZE_PARAMS_SIZE
            }
            Self::GrCaps => grstatic::GR_CAPS_PARAMS_SIZE,
            Self::GrInfo => kayfabe_abi::grinfo::KGR_GET_INFO_PARAMS_SIZE,
            Self::GrFloorsweepingMasks => grstatic::FLOORSWEEPING_PARAMS_SIZE,
            Self::GrGlobalSmOrder => grstatic::SM_ORDER_PARAMS_SIZE,
            Self::GrFecsRecordSize => grstatic::FECS_RECORD_SIZE_PARAMS_SIZE,
            Self::GrPdbProperties => grstatic::PDB_PROPERTIES_PARAMS_SIZE,
            Self::GvaspaceServerReservedPdes | Self::GvaspaceServerReservedPdesClient => {
                gvaspacepdes::COPY_SERVER_RESERVED_PDES_PARAMS_SIZE
            }
            Self::GrContextBuffersInfo => grstatic::CONTEXT_BUFFERS_INFO_PARAMS_SIZE,
            Self::GpuInfoV2 => GPU_GET_INFO_V2_PARAMS_SIZE,
            Self::InternalGpuGetSmcMode => {
                kayfabe_abi::smcmode::INTERNAL_GPU_GET_SMC_MODE_PARAMS_SIZE
            }
            Self::BusGetInfoV2 => kayfabe_abi::businfo::BUS_GET_INFO_V2_PARAMS_SIZE,
            Self::BusGetPcieSupportedGpuAtomics => {
                kayfabe_abi::gpuatomics::PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE
            }
            Self::FbGetInfoV2 => kayfabe_abi::fbinfo::FB_GET_INFO_V2_PARAMS_SIZE,
            Self::CeGetAllPhysicalCaps => kayfabe_abi::cecaps::CE_GET_ALL_CAPS_PARAMS_SIZE,
            Self::CeGetPhysicalCaps => kayfabe_abi::cecaps::CE_GET_CAPS_V2_PARAMS_SIZE,
            Self::CeGetCePceMask => kayfabe_abi::cepce::CE_GET_CE_PCE_MASK_PARAMS_SIZE,
            Self::GrmgrGetGrFsInfo => kayfabe_abi::grfsinfo::GR_FS_INFO_PARAMS_SIZE,
            Self::GspGetFeatures => kayfabe_abi::gspfeatures::GSP_GET_FEATURES_PARAMS_SIZE,
            Self::GssLegacy8159 => kayfabe_abi::gsslegacy::GSS_LEGACY_0X8159_PARAMS_SIZE,
            Self::GssLegacy8162 => kayfabe_abi::gsslegacy::GSS_LEGACY_0X8162_PARAMS_SIZE,
            Self::C2cInfo => kayfabe_abi::c2cinfo::C2C_INFO_PARAMS_SIZE,
            Self::PromoteFaultMethodBuffers => {
                kayfabe_abi::fmbpromote::PROMOTE_FAULT_METHOD_BUFFERS_PARAMS_SIZE
            }
        }
    }

    /// Classify a control command, or `None` if this policy does not model it.
    ///
    /// ★★★ **Derived from [`WantedTable::ALL`], and that is the whole point.** This was a
    /// second `match` listing the same seven ids a second time, which made *"the set we
    /// serve"* and *"the set our gates quantify over"* two lists that happened to agree.
    /// A lookup through `ALL` collapses them: the serve decision now reads the same array
    /// the sticky-answer property and `kayfabe-crec`'s reply-plane differential read, so a
    /// control cannot be served without being covered. See [`WantedTable::ALL`].
    ///
    /// ⊘ A linear scan of twelve, not a `match` — this runs once per RM control command,
    /// which the guest issues a few hundred times across a whole boot. Trading a jump table
    /// for an unfalsifiable pair of lists would be the wrong way round.
    #[must_use]
    pub fn from_cmd(cmd: u32) -> Option<WantedTable> {
        Self::ALL.into_iter().find(|w| w.cmd_id() == cmd)
    }
}

impl InitTablePolicy {
    /// Build the policy for one chip and one guest driver's wire table.
    ///
    /// The notifier probe is **empty** — this is the shipping constructor, and its ~25
    /// call sites are exactly the reason the probing case is a separate, named one
    /// ([`InitTablePolicy::with_probe_arm`]) rather than a parameter here.
    #[must_use]
    pub fn new(chip: &'static ChipProfile, driver: DriverAbiTable) -> InitTablePolicy {
        InitTablePolicy::with_probe_arm(chip, driver, eventnotify::ProbeArmSet::default())
    }

    /// Build the policy with a notifier **probe set** — reachability instrumentation, off
    /// unless the `probe-arm-notifier` device property names an index. See the field's
    /// docs; a boot that advances because of this set is reachability data, not a rung.
    #[must_use]
    pub fn with_probe_arm(
        chip: &'static ChipProfile,
        driver: DriverAbiTable,
        probe_arm: eventnotify::ProbeArmSet,
    ) -> InitTablePolicy {
        InitTablePolicy {
            chip,
            driver,
            // Every subdevice starts with every notifier disabled, which is what RM's own
            // zeroed `Subdevice` starts at and therefore the only starting state that
            // agrees with the guest — represented here as no slot at all.
            notify_actions: [None; NOTIFY_SUBDEVICE_SLOTS],
            probe_arm,
            // ⊘ Not a default value: nothing is known about the guest until it speaks, and
            // `GspGetFeatures` refuses while this is `None` rather than inventing one.
            guest_firmware: None,
        }
    }

    /// The guest driver version this policy has latched off fn 1, if any.
    ///
    /// Exposed so a test can ask what was observed without reaching into the reply plane,
    /// and so the distinction *"not yet seen"* / *"seen and refused"* has a name on this
    /// side too. See the field's docs.
    #[must_use]
    pub fn guest_firmware(&self) -> Option<kayfabe_abi::gspfeatures::FirmwareVersion> {
        self.guest_firmware
    }

    /// Which table this command asks for, if any — the classification step on its own.
    #[must_use]
    pub fn wanted(&self, cmd: &RpcCommand) -> Option<WantedTable> {
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        WantedTable::from_cmd(req.cmd)
    }
}

/// A reply that carries no body and a non-zero envelope result — the short-circuit.
fn refuse() -> Option<Reply> {
    Some(Reply {
        rpc_result: NV_ERR_NOT_SUPPORTED,
        body: Vec::new(),
    })
}

impl CommandPolicy for InitTablePolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        // ★★★ The one command this link READS without ANSWERING. `SET_GUEST_SYSTEM_INFO`
        // carries the guest's own `NV_VERSION_STRING` (`ogkm-580: rpc.c:8724-8727`), which
        // is the only source for `GspGetFeatures`'s `firmwareVersion` that a run backs:
        // `[measured 2026-08-09, real GA106 on 580.159.04]` the string it carries is the
        // one hardware returns, and `gsp_get_features.rs::
        // the_firmware_version_follows_the_guest_and_not_any_constant` is the test that
        // fails if this port ever takes it from a constant instead. This link
        // is seated ahead of `GuestSystemInfoPolicy`, so it sees the message first; the
        // `None` below is a decline, so that link still answers the handshake and no reply
        // byte changes. ⊘ Deliberately NOT an `Observing` seat: an observer cannot hold the
        // state, and this is state one *served* control needs.
        if cmd.function == RpcFunction::SetGuestSystemInfo {
            // ⊘ Guest bytes, so validated rather than stored. A string this port will not
            // repeat leaves the latch `None` and costs one refused report-only control,
            // which is the small side of the trade.
            //
            // ★★ **The most recent handshake always wins, including when it fails.** Both
            // failure modes — a message that does not decode, and a string this port will
            // not repeat — land on `None`, so the latch says what the guest is saying
            // *now* rather than what it once said. ⚠ Written as one assignment on purpose:
            // the first draft skipped the write when the decode failed and cleared on a
            // parse failure, which made two failures of one message behave differently and
            // could have reported a string the guest had already replaced. `tests/
            // gsp_get_features.rs::the_latch_always_reflects_the_most_recent_handshake` is
            // that defect's fixture.
            self.guest_firmware =
                kayfabe_abi::guestsysinfo::decode_guest_driver_version(&cmd.payload)
                    .ok()
                    .and_then(|text| kayfabe_abi::gspfeatures::FirmwareVersion::parse(text).ok());
            return None;
        }
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        // A payload too short to hold the control header is not a control this policy can
        // even classify; leave it to the baseline rather than inventing a refusal for a
        // message that may not be one.
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        let want = WantedTable::from_cmd(req.cmd)?;

        // A FINN-serialized payload is not the flat struct these encoders produce. Neither
        // control appears serialized anywhere this port has looked — the C answers both
        // flat and a real driver accepted it — but that is an absence of observation, not
        // a guarantee, and an unchecked flat answer is the kind of wrong that never logs.
        if kayfabe_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
            return refuse();
        }
        // The guest's own declared size must be the size we encode, and its payload must
        // actually hold it. Both are the guest's assertions, so both are checked.
        if req.params_size as usize != want.params_size()
            || cmd.payload.len() < req.params_at + want.params_size()
        {
            return refuse();
        }

        let params = match want {
            WantedTable::DeviceInfo => {
                // `baseIndex` is the guest's paging cursor, at the head of its own params.
                let at = req.params_at;
                let base_index = u32::from_le_bytes([
                    cmd.payload[at],
                    cmd.payload[at + 1],
                    cmd.payload[at + 2],
                    cmd.payload[at + 3],
                ]);
                match inittables::encode_device_info_table(self.chip.engines, base_index) {
                    Ok(p) => p.params,
                    Err(_) => return refuse(),
                }
            }
            WantedTable::IntrKernelTable => match inittables::encode_intr_kernel_table(
                self.chip.intr_table,
                &self.chip.intr_subtree_map,
            ) {
                Ok(p) => p,
                Err(_) => return refuse(),
            },
            // ★ No cursor and no request field is read: the answer is a pure function of
            // the chip row. That is not laziness — the request body for THIS control is
            // uninitialised guest stack (`ogkm-580: kern_bus.c:585`), so every byte of it
            // is untrusted, and the only field the reply keeps from it is the control
            // header the envelope path already validated.
            WantedTable::PciBarInfo => match pcibars::encode_pci_bar_info(self.chip.pci_bars) {
                Ok(p) => p,
                Err(_) => return refuse(),
            },
            // ★★ The identity half is taken from `identity_for`, which is the *same* call
            // the hypervisor shell builds configuration space from — and which refuses if
            // the chip's BAR table and its declared aperture disagree. That is deliberate:
            // `_gpuInitChipInfo` overwrites `pGpu->idInfo` with this reply
            // (`ogkm-580: gpu.c:891-893`), so a device whose reply and whose config header
            // disagreed would leave RM believing a part it did not enumerate, and nothing
            // would log. There is no second source here to drift from the first.
            WantedTable::ChipInfo => {
                let Ok(id) = crate::identity_for(self.chip) else {
                    return refuse();
                };
                let id = ChipIdentity {
                    pci_vendor_id: id.vendor_id,
                    pci_device_id: id.device_id,
                    pci_subsystem_vendor_id: id.subsystem_vendor_id,
                    pci_subsystem_id: id.subsystem_id,
                    pci_revision: id.revision,
                };
                match chipinfo::encode_chip_info(
                    &self.chip.chip_info,
                    &id,
                    self.chip.regs_aperture_len,
                ) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ A **policy** answered from the chip row, and the refusal on the error
            // arm is load-bearing rather than defensive: the encoder's job is to make the
            // one combination that means "open all of BAR0 to unprivileged guest
            // userspace" unencodable (`kayfabe_abi::regaccessmap`), and answering anyway
            // when it declines would be exactly the widening it exists to prevent.
            WantedTable::UserRegisterAccessMap => {
                match regaccessmap::encode_user_register_access_map(
                    &self.chip.user_register_access_map,
                ) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ An **inventory**, and the refusal on the error arm is the same kind of
            // load-bearing as the one above. The encoder exists to make a count RM's own
            // bounds check would let through — 65..=71, which passes its `<= 71`
            // destination-array assert and then reads past the 1284-byte source struct
            // (`kayfabe_abi::falconinfo::FalconInfoError::TooManyFalcons`) — unencodable.
            // Answering anyway when it declines would hand the guest exactly the
            // out-of-bounds construction it exists to prevent.
            WantedTable::ConstructedFalconInfo => {
                match falconinfo::encode_constructed_falcon_info(
                    &self.chip.constructed_falcons,
                    self.chip.regs_aperture_len,
                ) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ The one whose error arm is load-bearing in the OTHER direction. Every
            // refusal above stops a boot at a named statement; this one deletes an engine
            // and lets the boot continue into a NULL dereference
            // (`ogkm-580: gpu.c:2170-2214`, and `kayfabe_abi::memsysconfig`'s docs for the Oops
            // `[measured]` at `1c79474` on a 580.159.04 guest). So `refuse()` here is the
            // *worse* outcome, not the safe one —
            // and it is still right, because the combinations the encoder declines are each
            // a guest-kernel fault of their own: an all-zero policy pair violates an assert
            // RM makes on itself (`kern_mem_sys.c:422`), and a zero `comprPageSize` is a
            // divide-by-zero (`mem_mgr_gm107.c:211`). There is no answer that is safe by
            // default here; there is only a row that is right.
            WantedTable::MemorySystemStaticConfig => {
                match memsysconfig::encode_memsys_static_config(&self.chip.memory_system) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ The only arm that is a **projection** rather than a statement: the reply
            // is derived from `chip.engines` — the same slice `WantedTable::DeviceInfo`
            // serves through a different control — and `chip.device_info` supplies only the
            // one field that slice does not carry. Two hand-written descriptions of one
            // silicon is the drift `kayfabe_abi::deviceinfo` exists to forbid.
            //
            // ⚠ The error arm is the same shape as the one above and just as load-bearing:
            // this control is asked from `gpuStateInit_IMPL`, which maps a refusal to
            // `NV_OK` and leaves `KernelFifo` constructed-but-empty
            // (`ogkm-580: gpu.c:2286-2287`). `refuse()` is therefore the worse outcome here
            // too — and still the right one, because every projection the encoder declines
            // is a guest-kernel fault of its own: no `LCE` row ends the boot inside
            // `kgmmuInitCeMmuFaultIdRange_GA100` (`ogkm-580: kern_gmmu_ga100.c:281-287`),
            // and a gap in the copy-engine fault ids is an id the guest attributes to an
            // engine this device never advertised.
            WantedTable::InternalDeviceInfo => {
                match deviceinfo::encode_internal_device_info_table(
                    self.chip.engines,
                    &self.chip.device_info,
                    self.chip.regs_aperture_len,
                ) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ Two `NvBool`s, and the arm where the error branch is the ONLY thing
            // this encoder can make unencodable. Both bits clear is the truth for this
            // device AND what a refusal leaves behind — see `kayfabe_abi::confcompute` —
            // so there is no fail-open combination to forbid. What is forbidden is the
            // widening: either bit set deletes RM's own refusal to map compute-protected
            // vidmem through BAR1 (`ogkm-580: mapping_cpu.c:227-235`), and this port serves
            // no such region.
            WantedTable::ConfComputeStaticInfo => {
                match confcompute::encode_conf_compute_static_info(&self.chip.conf_compute) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★ Four `NvBool`s, two of which are directions rather than descriptions:
            // `bIsC2CLinkUp` sends `kmemsysStateInitLocked` down a coherent chip-to-chip
            // mapping of framebuffer, and `bIsDeviceMultiFunction` sends
            // `_kbifSavePcieConfigRegisters` at configuration space for a PCI function 1.
            // The encoder declines both for a device that presents neither.
            WantedTable::BifStaticInfo => {
                match bifstatic::encode_bif_static_info(&self.chip.bif_static) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ The second arm that reads the request, and for the opposite reason to
            // `DeviceInfo`'s cursor: `runlistId` is an `[IN]` field the guest chose, and
            // overwriting it with a number of our own would be answering a question the
            // guest did not ask. `numChannels` is the answer and it comes from the chip row.
            //
            // ⚠ The error arm is load-bearing in the direction `MemorySystemStaticConfig`'s
            // is: a zero count would be answered `NV_OK` and then read as
            // `NV_ERR_INVALID_STATE` by `kfifoChidMgrConstruct`
            // (`ogkm-580: kernel_fifo.c:300-308`) — an envelope that says the answer is good
            // wrapped around the content of a refusal.
            WantedTable::FifoNumChannels => {
                let at = req.params_at;
                let runlist_id = u32::from_le_bytes([
                    cmd.payload[at],
                    cmd.payload[at + 1],
                    cmd.payload[at + 2],
                    cmd.payload[at + 3],
                ]);
                match fifochannels::encode_fifo_num_channels(&self.chip.fifo_channels, runlist_id) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ The arm whose refusal is a guest-kernel USE-AFTER-FREE rather than a
            // NULL dereference: `_kgmmuInitStaticInfo`'s `fail:` label frees
            // `pKernelGmmu->pStaticInfo` and leaves the field pointing at it
            // (`ogkm-580: kern_gmmu.c:139-166`). Every declined combination here is a fault
            // of its own — a zero non-replayable size is an invariant RM asserts against
            // itself (`kern_gmmu.c:1909`), and a size that is not a multiple of
            // `NVC369_BUF_SIZE` is a partial fault packet in a queue whose capacity is a
            // division (`kern_gmmu.c:1725`).
            WantedTable::GmmuStaticInfo => {
                match gmmustatic::encode_gmmu_static_info(&self.chip.gmmu_static) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ `0x20800a9b` — the IDENTITY arm. See the variant's docs for the whole
            // argument; what happens *here* is three things and no more.
            //
            // (1) **Decode, and let the decode be load-bearing.** An arm that returned the
            //     bytes untouched would be a fall-through `NV_OK` wearing a variant's name —
            //     the exact shape §7 step 1 of `resume_from_fault.md` was written to remove.
            //     The decode is what turns "2064 bytes arrived" into "a page list this port
            //     can read", and it is the same decode `crate::faultbuffer`'s observer seat
            //     records from, so the reply and the record can never describe different
            //     messages.
            // (2) **Refuse a size CPU-RM itself refuses.** `exceeds_vendor_bound` is
            //     `kern_gmmu.c:1242-1248`'s own test, applied before it would have to answer
            //     "registered" about a buffer it recorded only the first 256 pages of. ⊘ No
            //     stock guest can reach it — CPU-RM checks first — so this can only ever fire
            //     for a hostile one, which is the guest this port is for.
            // (3) **Echo.** Pure `[IN]`, so the identity is the byte-accurate reply and the
            //     common tail below splices it back at `req.params_at` unchanged.
            //
            // ⊘ Note what is NOT here: no registration state, no second-register refusal.
            // The receiver's `NV_ERR_NOT_SUPPORTED`-on-double-register (`kern_gmmu.c:3117`)
            // is real, and its partner `0x20800a9c` UNREGISTER is **not served** — so
            // modelling one half would build a latch that can only close. Repeats are counted
            // instead and reported; see `kayfabe_abi::faultbuffer`.
            WantedTable::RegisterFaultBuffer => {
                let raw = match cmd
                    .payload
                    .get(req.params_at..req.params_at + want.params_size())
                {
                    Some(s) => s,
                    // Unreachable while the length guard above stands, and written as a
                    // refusal rather than an `expect` because a panic in a policy is a
                    // guest-reachable abort.
                    None => return refuse(),
                };
                match kayfabe_abi::faultbuffer::decode_register_fault_buffer(raw) {
                    Ok(r) if !r.exceeds_vendor_bound() => raw.to_vec(),
                    Ok(_) | Err(_) => return refuse(),
                }
            }
            // ★★★ `0x20800a9d` — the same three steps as the arm above, for the same reasons,
            // over a 24 032-byte struct. ⊘ The bound check here counts BOTH terms
            // (`RM_PAGE_ALIGN_UP(size) + RM_PAGE_ALIGN_UP(metadataSize)`,
            // `ogkm-580: kern_gmmu.c:1601`), because the metadata term is zero in every
            // configuration this port targets and a check on `size` alone would be right on
            // the only case anyone runs.
            //
            // ⊘ An unknown or replayable `shadowFaultBufferType` is NOT refused. The
            // replayable *shadow* buffer needs Confidential Compute
            // (`ogkm-580: mmu_fault_buffer_ctrl.c:148`), which is off, so a registration
            // carrying it is a finding no measurement has reached — and refusing on it would
            // model a path from a reading. It is recorded and reported instead.
            WantedTable::RegisterClientShadowFaultBuffer => {
                let raw = match cmd
                    .payload
                    .get(req.params_at..req.params_at + want.params_size())
                {
                    Some(s) => s,
                    None => return refuse(),
                };
                match kayfabe_abi::faultbuffer::decode_register_client_shadow_fault_buffer(raw) {
                    Ok(r) if !r.exceeds_vendor_bound() => raw.to_vec(),
                    Ok(_) | Err(_) => return refuse(),
                }
            }
            // ★★★ `0x20800a1d` — third of the three, and its geometry check has TWO arms
            // rather than one: the physical receiver refuses `numBufferPages > 64` **or
            // `== 0`** (`ogkm-580: access_cntr_buffer_ctrl.c:231-253`). The zero arm is not
            // decoration — a zero-size access-counter buffer is the same zero that killed
            // `cuInit` one layer up, and refusing it here is refusing to answer `NV_OK` about
            // a buffer that cannot exist.
            WantedTable::RegisterAccessCntrBuffer => {
                let raw = match cmd
                    .payload
                    .get(req.params_at..req.params_at + want.params_size())
                {
                    Some(s) => s,
                    None => return refuse(),
                };
                match kayfabe_abi::faultbuffer::decode_register_access_cntr_buffer(raw) {
                    Ok(r) if !r.is_illegal_geometry() => raw.to_vec(),
                    Ok(_) | Err(_) => return refuse(),
                }
            }
            // ★★★ The event-plane arm — the only one that CHANGES this policy's state, and
            // the only one whose reply is a function of the request alone.
            //
            // Four gates, in the order the guest's own handler applies them, and every one
            // of them is transcription rather than invention:
            //
            // 1. The wire decode, which enforces `event < NV2080_NOTIFIERS_MAXCOUNT` and
            //    `event != NV2080_NOTIFIERS_TIMER` — the two checks
            //    `subdeviceCtrlCmdEventSetNotification_IMPL` makes *before* the RPC
            //    (`ogkm-580: subdevice_ctrl_event_kernel.c:96-106`), so a request that
            //    fails them cannot legitimately have reached us at all.
            // 2. `SILENT_NOTIFIERS`, which is this port's OWN policy and the only gate here
            //    that RM does not have: we deliver no events, so we may only accept an
            //    arming for one that cannot occur. See `kayfabe_abi::eventnotify`.
            // 3. The already-armed transition rule, which the physical RM on a real GSP
            //    keeps for its own subdevice exactly as the guest keeps it for its copy
            //    (`subdevice_ctrl_event_kernel.c:124-131`).
            // 4. The re-encode, which is NOT an echo — see `encode_event_set_notification`.
            //
            // ⊘ `refuse()` on every failure, and refusing is clean here in a way it is not
            // for the amputating controls above: the copyout is skipped on a non-`NV_OK`
            // status (`ogkm-580: rpc.c:11066-11070`), so the guest's own params struct is
            // left exactly as it wrote it and `NV_CHECK_OK_OR_RETURN` returns at a named
            // statement.
            WantedTable::EventSetNotification => {
                let at = req.params_at;
                let Ok(reg) = eventnotify::decode_event_set_notification(
                    &cmd.payload[at..at + eventnotify::EVENT_SET_NOTIFICATION_PARAMS_SIZE],
                ) else {
                    return refuse();
                };
                // ★★★ §14.18 — TWO admitting lists, and they are two different promises:
                // `SILENT_NOTIFIERS` accepts an arming because the event cannot occur on
                // this device, `DELIVERED_NOTIFIERS` because it does occur and this device
                // raises it (`RegPlane::announce_completion`). ⊘ Neither is a widening of
                // the other and an index must never migrate between them: the arguments
                // are about different facts, so a row that moved would keep a sentence
                // that no longer supports it.
                //
                // ⊘ The third disjunct is a PROBE and is off unless the device property
                // names an index — see `eventnotify::ProbeArmSet`. A boot that gets
                // further because of it measures REACHABILITY, never correctness.
                if !eventnotify::is_silent_notifier(reg.event)
                    && !eventnotify::is_delivered_notifier(reg.event)
                    && !self.probe_arm.contains(reg.event)
                {
                    return refuse();
                }
                // The index is bounded by the decode; re-checked so the indexing below can
                // never panic even if the decoder's bound ever drifts.
                let ev = reg.event as usize;
                if ev >= eventnotify::NV2080_NOTIFIERS_MAXCOUNT as usize {
                    return refuse();
                }
                // RM's transition rule reads `pSubdevice->notifyActions` — PER-subdevice
                // (`ogkm-580: subdevice_ctrl_event_kernel.c:124-131`) — so the state is
                // keyed by the control header's own `(hClient, hObject)`. See the field's
                // docs for the aliasing this fixes and the bound's residuals.
                let slot_idx = self.notify_actions.iter().position(
                    |s| matches!(s, Some(s) if s.client == req.client && s.object == req.object),
                );
                let current = slot_idx
                    .and_then(|i| self.notify_actions[i].map(|s| s.actions[ev]))
                    .unwrap_or(eventnotify::ACTION_DISABLE as u8);
                if reg.action != eventnotify::ACTION_DISABLE
                    && current != eventnotify::ACTION_DISABLE as u8
                {
                    return refuse();
                }
                // ⚠ Recorded BEFORE the reply is built, and it is the port's own state
                // rather than a cache of the guest's: the day this device can raise an
                // interrupt, these slots are what say which subdevice armed which notifier
                // with what. Until then they are what makes the transition rule above
                // enforceable.
                if reg.action == eventnotify::ACTION_DISABLE {
                    // DISABLE has no precondition (`subdevice_ctrl_event_kernel.c:133-137`):
                    // on an unknown subdevice it is a legal no-op and allocates nothing.
                    if let Some(i) = slot_idx {
                        let released = match self.notify_actions[i].as_mut() {
                            Some(s) => {
                                s.actions[ev] = eventnotify::ACTION_DISABLE as u8;
                                s.actions
                                    .iter()
                                    .all(|&a| a == eventnotify::ACTION_DISABLE as u8)
                            }
                            None => false,
                        };
                        // A subdevice with nothing armed releases its slot — the reclaim
                        // that keeps arm/disarm cycling from growing state.
                        if released {
                            self.notify_actions[i] = None;
                        }
                    }
                } else if let Some(i) = slot_idx {
                    if let Some(s) = self.notify_actions[i].as_mut() {
                        s.actions[ev] = u8::try_from(reg.action).unwrap_or(0);
                    }
                } else {
                    let Some(free) = self.notify_actions.iter().position(Option::is_none) else {
                        // The arming that would need one slot more than
                        // `NOTIFY_SUBDEVICE_SLOTS`: bounded state fails LOUD — a refused
                        // row in the census — never silently evicts an armed subdevice.
                        return refuse();
                    };
                    let mut s = SubdeviceNotifyActions {
                        client: req.client,
                        object: req.object,
                        actions: [eventnotify::ACTION_DISABLE as u8;
                            eventnotify::NV2080_NOTIFIERS_MAXCOUNT as usize],
                    };
                    s.actions[ev] = u8::try_from(reg.action).unwrap_or(0);
                    self.notify_actions[free] = Some(s);
                }
                eventnotify::encode_event_set_notification(&reg)
            }
            // ★★★ The arm that answers an **action** rather than a description, and the
            // only one whose licence is a claim about this device's own structure rather
            // than a number out of a chip row. The full argument — why `NV_OK` is honest
            // here, what would make it false, and why the reply is four zeros rather than
            // an echo — is in `kayfabe_abi::l2evict`; it is deliberately not restated,
            // because a summary of an argument is the shape this repository has been bitten
            // by (`read_at_invalidate_is_false_on_compute_path`).
            //
            // ⊘ The decode is load-bearing and is the ONLY thing this arm decides. The body
            // does not depend on the request, so a decoder that could not fail would make
            // this arm a fall-through `NV_OK` — precisely what `#127`'s named-refusal
            // default forbids. `L2EvictError::UnknownFlags` is what keeps it from being
            // one: the vacuity argument enumerates six operations, so a seventh, named by a
            // bit the 580 SDK does not define, is refused rather than blanket-accepted.
            //
            // ⚠ And `refuse()` here is the SAFE direction, unlike the amputating arms
            // above: the copyout is skipped on a non-`NV_OK` status (`ogkm-580:
            // rpc.c:11066-11070`), and `kbusVerifyBar2_GM107:4110-4115` turns the refusal
            // into `"L2 evict failed"` and a `goto` — loud, named, and exactly where the
            // boot stood before this rung.
            WantedTable::MemsysL2InvalidateEvict => {
                let at = req.params_at;
                let Ok(evict) = l2evict::decode_l2_invalidate_evict(
                    &cmd.payload[at..at + l2evict::L2_INVALIDATE_EVICT_PARAMS_SIZE],
                ) else {
                    return refuse();
                };
                l2evict::encode_l2_invalidate_evict(&evict)
            }
            // ★★★ The arm answered with a number **measured on real silicon** — an RTX 3060
            // running open 580.159.04, 2026-08-01, `traces/real_ga106/`. It reads no
            // request field — the params struct is a single `[OUT]` `NvU32` and the guest
            // sends it zeroed — so the only thing this arm can get wrong is the value, and
            // the value is not this crate's to choose: it comes off the chip row, which is
            // where the measurement is attributed.
            //
            // ⊘ `refuse()` when the chip states no size, and that refusal is unreachable in
            // practice because `identity_for` will not realize such a chip. It is here
            // anyway, because the alternative to a refusal is `encode` writing four zero
            // bytes — a reply that looks served and rebuilds the wall
            // (`ogkm-580: mem_desc.c:239-241`) — and a policy whose worst case is a *silent*
            // wall is exactly the shape `#127`'s named-refusal default forbids.
            WantedTable::CeFaultMethodBufferSize => {
                match kayfabe_abi::fmbsize::encode_fault_method_buffer_size(
                    self.chip.ce_fault_method_buffer_size,
                ) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ The five GR static-info arms. Each is a pure function of the chip's own
            // GR profile — no request field is read, because none of these controls carries
            // one — so the only failure mode is a profile that describes no silicon, and
            // `GrStaticProfile::validate` is what turns that into a refusal instead of a
            // plausible reply. See `kayfabe_abi::grstatic`.
            WantedTable::GrCaps => match grstatic::encode_gr_caps(&self.chip.gr_static) {
                Ok(p) => p,
                Err(_) => return refuse(),
            },
            // ★★★ The sixth GR arm, and the only one that validates against ANOTHER
            // chip-row field before it encodes. Six of its 58 entries restate the geometry
            // the five arms above publish, and RM reads both descriptions — so a pair that
            // disagrees is refused rather than served twice over. The other 52 are litter
            // constants and are checked only for the three zeros RM would read as numbers.
            //
            // ⊘ `refuse()` here is the WORSE outcome and it is still right: a refusal
            // reproduces run `fmb1` exactly (`0x25:0x40:1249`), which is loud, attributable
            // and already in the log — while an answer built from a profile that contradicts
            // `gr_static` would hand the guest two incompatible descriptions of one chip and
            // no statement would fail.
            WantedTable::GrInfo => {
                if self
                    .chip
                    .gr_info
                    .validate_against(&self.chip.gr_static)
                    .is_err()
                {
                    return refuse();
                }
                match self.chip.gr_info.encode() {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            WantedTable::GrFloorsweepingMasks => {
                match grstatic::encode_floorsweeping_masks(&self.chip.gr_static) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            WantedTable::GrGlobalSmOrder => {
                match grstatic::encode_global_sm_order(&self.chip.gr_static) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            WantedTable::GrFecsRecordSize => {
                match grstatic::encode_fecs_record_size(&self.chip.gr_static) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            WantedTable::GrPdbProperties => {
                match grstatic::encode_pdb_properties(&self.chip.gr_static) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ The publication arm — the one control here whose reply is a function of
            // the REQUEST rather than of the chip. It is decoded, validated against
            // `ctrl90f1.h`'s own stated rules, and re-encoded from the decoded fields; an
            // echo would have made the decode dead code no test could notice was dead.
            //
            // ⊘ `refuse()` is the safe direction and it is loud: the copyout is skipped on a
            // non-`NV_OK` status (`ogkm-580: rpc.c:11066-11070`), so the guest's own
            // `globalCopyParams` is untouched and `NV_ASSERT_OK_OR_RETURN` fails at
            // `gpu_vaspace.c:4148` by name.
            WantedTable::GvaspaceServerReservedPdes
            | WantedTable::GvaspaceServerReservedPdesClient => {
                let at = req.params_at;
                let Ok(pdes) = gvaspacepdes::decode_server_reserved_pdes(
                    &cmd.payload[at..at + gvaspacepdes::COPY_SERVER_RESERVED_PDES_PARAMS_SIZE],
                ) else {
                    return refuse();
                };
                gvaspacepdes::encode_server_reserved_pdes(&pdes)
            }
            // ⚠ The one GR reply that is NOT a function of the geometry: context-buffer
            // sizes are the chip's own table, so they come off the chip row directly. See
            // `kayfabe_abi::grstatic`'s deferred-half section for why pretending they could
            // be derived from GPC/TPC counts would be inventing a relationship.
            WantedTable::GrContextBuffersInfo => {
                match grstatic::encode_context_buffers_info(&self.chip.gr_context_buffers) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ The only arm that reads the request as a whole, and the only one whose
            // reply is the request EDITED. Both properties are forced by the control:
            //
            // - The guest kernel has already written its own answers into the entries it
            //   resolved, and marked only the ones it could not with bit 31
            //   (`ogkm-580: subdevice_ctrl_gpu_kernel.c:548, 566`). Re-encoding from a table
            //   would overwrite the kernel's own state with a transcription.
            // - `[measured 2026-08-08, real GA106 (RTX 3060, GPU-d0913685), driver
            //   580.159.04, `rmladder --gpu-info-sweep` R21]` real GSP returns the untouched
            //   tail verbatim: the sweep seeds every byte past the declared entries with
            //   `0xCD` and reads them back unchanged. So the copy is what hardware does, not
            //   a shortcut.
            //
            // ⊘ The slice is in bounds by the pre-arm size check above — `req.params_size ==
            // want.params_size()` and `payload.len() >= params_at + params_size` are both
            // already asserted — and `answer_gpu_get_info_v2` re-checks the length anyway,
            // because a bound that lives only in a caller is a bound one refactor from gone.
            WantedTable::GpuInfoV2 => {
                let at = req.params_at;
                match gpuinfo::answer_gpu_get_info_v2(
                    &cmd.payload[at..at + GPU_GET_INFO_V2_PARAMS_SIZE],
                    self.chip.forwarded_gpu_info,
                ) {
                    Ok(p) => p,
                    // ⊘ Refused BY NAME, and the whole call — which is RM's own shape, its
                    // loop breaking on the first index it cannot answer. The two indices
                    // that land here (`0x23`, `0x24`) are per-chip identity values
                    // `[measured 2026-08-08]` to DIFFER between two physical RTX 3060 parts;
                    // see `kayfabe_abi::gpuinfo`.
                    Err(_) => return refuse(),
                }
            }
            // ★★★★ The control that decided `cuInit`. A pure function of the chip row — this
            // control carries no request field — so there is nothing to validate and nothing
            // that can fail, which is why this arm cannot refuse and does not pretend it can.
            //
            // ⚠ The value is an ENUM on the chip row, not a `u32`, and that is load-bearing
            // rather than tidy: the wire answer on GA106 is four zero bytes, byte-identical
            // to what the C oracle's EMPTY row for this id decodes to. A `u32` field left at
            // its default would be indistinguishable from the measurement. See
            // `kayfabe_abi::smcmode` for the two-part provenance.
            WantedTable::InternalGpuGetSmcMode => {
                kayfabe_abi::smcmode::encode_smc_mode(self.chip.smc_mode)
            }
            // ★★★ The second request-editing arm, and the first whose VALUE is derived
            // rather than transcribed. The table handed to it has one row, built here from
            // the chip's die generation — deliberately not a `&'static` table like
            // `forwarded_gpu_info`, because a `&'static [(u32, u32)]` is exactly the shape
            // that invites a measured word to be pasted into it — and the word MOVES:
            // `[measured 2026-08-08, real GA106 `GPU-d0913685`, R22 runs 1 and 2,
            // `traces/real_ga106/rmladder_r22_businfo_{sweep,loaded}_real_ga106.txt`]`
            // `0x00302000` with the link idle at 2.5 GT/s and `0x00322000` under load.
            //
            // ⊘ Every declared entry is filled with no forward-bit test, because
            // `kbusSendBusInfo_IMPL` puts ONE entry in a FRESH params struct per forwarded
            // index (`ogkm-580: kern_bus.c:1065-1101`): arriving here is the marker. An
            // index with no derivation is refused by name — never answered zero, which on
            // this control reads as a positive claim of `PCIE_LINK_CAP_GEN_GEN1`.
            WantedTable::BusGetInfoV2 => {
                let at = req.params_at;
                let answers = [(
                    kayfabe_abi::businfo::BUS_INFO_INDEX_PCIE_GEN_INFO,
                    kayfabe_abi::businfo::PcieGenInfo::fully_trained(self.chip.pcie_max_gen)
                        .encode(),
                )];
                match kayfabe_abi::businfo::answer_bus_get_info_v2(
                    &cmd.payload[at..at + kayfabe_abi::businfo::BUS_GET_INFO_V2_PARAMS_SIZE],
                    &answers,
                ) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ The third request-editing arm, and the first whose REFUSAL is itself a
            // measured hardware behaviour rather than an admission of ignorance.
            // `[measured 2026-08-08, real GA106 `GPU-d0913685`, R23,
            // `traces/real_ga106/rmladder_r23_atomics_real_ga106.txt`]` a real part answers
            // `capType = SYSMEM(0)` with thirteen `bSupported=FALSE, attributes=0` written
            // into a `0xCD`-seeded buffer, and refuses `_CAPTYPE_GPU(1)`, `_CAPTYPE_P2P(2)`
            // and every undeclared value with `0x56`.
            //
            // ⊘ So `answer_...` refuses everything but SYSMEM, and the refusal here is the
            // right answer rather than a gap: answering GPU/P2P "all unsupported" would
            // return `NV_OK` where hardware returns `NV_ERR_NOT_SUPPORTED`.
            //
            // ⊘ And the value takes no chip argument: PCIe atomics to coherent sysmem need
            // the ROOT COMPLEX to be an AtomicOp completer, so this is `PCIE_GEN_INFO`'s
            // species — a fact about a machine and a link, never about a die.
            WantedTable::BusGetPcieSupportedGpuAtomics => {
                let at = req.params_at;
                match kayfabe_abi::gpuatomics::answer_bus_get_pcie_supported_gpu_atomics(
                    &cmd.payload
                        [at..at + kayfabe_abi::gpuatomics::PCIE_SUPPORTED_GPU_ATOMICS_PARAMS_SIZE],
                    &kayfabe_abi::gpuatomics::GpuAtomicOp::none_supported(),
                ) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ The fourth request-editing arm, and the first that introduces NO NEW
            // NUMBER: all four values are projections of `self.chip.memory_system`, the row
            // this policy already serves to `0x20800a1c`. Two are that row's fields
            // verbatim (`l2CacheSize`, `ramType`); two are derived from `ltcCount` by
            // relations named with their architecture in `kayfabe_abi::fbinfo`. A second
            // table of measured words is exactly what would let this device tell RM its L2
            // is 2.25 MiB under one control id and something else under another.
            //
            // ⊘ Three of the seven indices in libcuda's ioctl are the guest kernel's own
            // (`ogkm-580: kern_mem_sys_ctrl.c:335, 711, 716`) and never arrive here — our
            // own boot already answers `TOTAL_RAM_SIZE` byte-identically to a real GA106
            // without this arm. Anything not in the derived set is refused BY NAME: on this
            // control zero means "no L2", "unknown RAM" or "no FB partitions", never blank.
            WantedTable::FbGetInfoV2 => {
                let at = req.params_at;
                let geometry = kayfabe_abi::fbinfo::FbGeometry {
                    l2_cache_size: self.chip.memory_system.l2_cache_size,
                    ram_type: self.chip.memory_system.ram_type,
                    ltc_count: self.chip.memory_system.ltc_count,
                };
                let Ok(answers) = geometry.forwarded_answers() else {
                    return refuse();
                };
                match kayfabe_abi::fbinfo::answer_fb_get_info_v2(
                    &cmd.payload[at..at + kayfabe_abi::fbinfo::FB_GET_INFO_V2_PARAMS_SIZE],
                    &answers,
                ) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ The FIRST arm that does not read the request at all. Every other reply
            // this policy builds is the guest's own buffer with fields overwritten;
            // `NV2080_CTRL_CE_GET_ALL_CAPS_PARAMS` documents both its members `[out]`
            // (`ogkm-580: ctrl2080ce.h:315-322`) and the guest kernel has already
            // `portMemSet` the whole 136 bytes to zero before forwarding
            // (`kernel_ce_shared.c:312`), so there is nothing to echo and `params_at` is
            // never dereferenced here.
            //
            // ⊘ And it states no new number either: `present` is the `DEV_TYPE_ENUM_LCE`
            // rows of `chip.engines` — the same slice `WantedTable::DeviceInfo` and
            // `WantedTable::InternalDeviceInfo` serve — and the one per-CE caps bit is
            // `NV_CE_GRCE_ALLOWED_LCE_MASK` intersected with them. A device that advertises
            // four copy engines to the guest's FIFO and five to its CE layer would be two
            // descriptions of one silicon.
            //
            // ⚠ The error arm is load-bearing in the usual direction and then some: the
            // caller wraps this control in `NV_ASSERT_OK_OR_RETURN`, so a refusal is the
            // whole of `CE_GET_ALL_CAPS` failing — which is exactly the `0x56` this rung
            // exists to remove. It is still right: every projection `from_engines` declines
            // is a chip row that would already have failed `encode_internal_device_info_table`
            // (`kgmmuInitCeMmuFaultIdRange_GA100` needs an LCE row to boot at all), and a
            // `present` of zero is a declared value meaning "this GPU has no copy engines".
            WantedTable::CeGetAllPhysicalCaps => {
                let Ok(geometry) = kayfabe_abi::cecaps::CeGeometry::from_engines(self.chip.engines)
                else {
                    return refuse();
                };
                kayfabe_abi::cecaps::encode_ce_get_all_physical_caps(&geometry)
            }
            // ★★★ §14.42, first of two. The PER-ENGINE door onto the table the arm above
            // emits whole — `geometry.caps_for(publicID)`, out of the SAME `CeGeometry`, so
            // this states no number the device does not already state. ⊘ The alternative
            // (a fresh per-engine table) is exactly how a device comes to say its CE2 is a
            // graphics copy engine under one control id and is not under another.
            //
            // ⊘⊘ And it is NOT the identity echo the three rungs before it used: `capsTbl`
            // is `[OUT]` and `kceGetDeviceCaps_IMPL` `portMemCopy`s our two bytes verbatim
            // into the guest's own caps (`ogkm-580: kernel_ce.c:557-560`). Echoing would
            // hand the guest back its own uninitialised buffer.
            //
            // ⚠ The error arm is load-bearing: the caller wraps this in
            // `NV_ASSERT_OK_OR_RETURN`, so a refusal fails the whole of `CE_GET_CAPS`. It is
            // still right — both refusals are RM's own `NV_ERR_NOT_SUPPORTED` arms
            // (`kernel_ce_shared.c:257-260` for a non-copy type, `:268-273` for an engine the
            // part does not advertise), and the second is corroborated at the boundary by
            // `0x20802a02` refusing LCE4 on a real GA106. ⊘ A zero caps row would NOT be the
            // safe fallback: `{0,0}` positively claims a copy engine that can do nothing.
            WantedTable::CeGetPhysicalCaps => {
                let Ok(geometry) = kayfabe_abi::cecaps::CeGeometry::from_engines(self.chip.engines)
                else {
                    return refuse();
                };
                let at = req.params_at;
                match kayfabe_abi::cecaps::answer_ce_get_physical_caps(
                    &cmd.payload[at..at + kayfabe_abi::cecaps::CE_GET_CAPS_V2_PARAMS_SIZE],
                    &geometry,
                ) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ §14.42, second of two — and the only arm in this rung whose value was
            // measured AT ITS OWN BOUNDARY rather than derived. `0x20802a02` carries
            // `NON_PRIVILEGED`, so unlike every other control on this plane a real GA106
            // could simply be asked; R24 did (`traces/real_ga106/rmladder_r24_pcemask_...`).
            //
            // ⊘ `present` is passed in from the SAME `CeGeometry` the caps arm uses, so the
            // engine list and the PCE table are checked against each other here rather than
            // allowed to drift into two answers about how many copy engines exist.
            //
            // ⚠ It is served in the same rung as its neighbour on purpose:
            // `queryCopyEngines` issues both per engine, six lines apart, each with a hard
            // `goto done`. Serving one alone moves the wall and buys nothing.
            WantedTable::CeGetCePceMask => {
                let Ok(geometry) = kayfabe_abi::cecaps::CeGeometry::from_engines(self.chip.engines)
                else {
                    return refuse();
                };
                let at = req.params_at;
                match kayfabe_abi::cepce::answer_ce_get_ce_pce_mask(
                    &cmd.payload[at..at + kayfabe_abi::cepce::CE_GET_CE_PCE_MASK_PARAMS_SIZE],
                    geometry.present,
                    self.chip.lce_pce_masks,
                ) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ The first arm whose reply carries PER-ITEM statuses. Every other control
            // here is served or refused as a whole; this one answers `NV_OK` with a status
            // word inside each query, because that is RM's own contract
            // (`ogkm-580: ctrl2080grmgr.h:42-50`) and refusing the batch is what a real
            // GA106 does NOT do.
            //
            // ⊘ And it states no new number: `gpc_mask` is the row already served to
            // `INTERNAL_STATIC_KGR_GET_FLOORSWEEPING_MASKS`. The one query `cuInit` asks —
            // `CHIPLET_GPC_MAP` — is the logical→physical GPC map, which is that mask's set
            // bits in order.
            //
            // ⚠ The error arm is the loud one BY DESIGN. A query type this port does not
            // model could have been answered with a per-query `NV_ERR_NOT_SUPPORTED`, which
            // would have been invisible: the command is served, the result is `0`, and
            // neither ledger would carry a word about it. `kayfabe_abi::grfsinfo` therefore
            // returns an error for those and this arm refuses the whole control, which
            // costs one boot and cannot be missed.
            WantedTable::GrmgrGetGrFsInfo => {
                let at = req.params_at;
                // ⊘ `gpc_mask()` off the chip's OWN GR rows, never `GA106_GPC_MASK`: the
                // constant is the same value today and would be a second statement of it.
                // `GrStaticProfile::gpc_mask` derives from `gpcs.len()`, which is the slice
                // `WantedTable::GrFloorsweepingMasks` encodes — one description of one
                // silicon, the `deviceinfo` rule applied to the GR plane.
                let Ok(gpc_mask) = self.chip.gr_static.gpc_mask() else {
                    return refuse();
                };
                let tpc_masks: Vec<u32> = self
                    .chip
                    .gr_static
                    .gpcs
                    .iter()
                    .map(|g| g.tpc_mask)
                    .collect();
                let geometry = kayfabe_abi::grfsinfo::GrFsGeometry {
                    gpc_mask,
                    // `physGfxGpcMask` — the same word `encode_floorsweeping_masks` writes
                    // for all three GPC masks, and for the same reason: they cannot drift.
                    gfx_gpc_mask: gpc_mask,
                    tpc_masks: &tpc_masks,
                };
                match kayfabe_abi::grfsinfo::answer_gr_fs_info(
                    &cmd.payload[at..at + kayfabe_abi::grfsinfo::GR_FS_INFO_PARAMS_SIZE],
                    &geometry,
                ) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★★ The first arm whose reply is a fact about the GUEST, not about the
            // silicon — and the first this port serves that the guest's own export table
            // marks `CACHEABLE`, i.e. the first `crate::sticky::BRANCH_A_CACHEABLE` row.
            //
            // ⊘ It states no new number in the strongest sense available: nothing here is
            // tabulated at all. Three fields are named constants out of `ctrl2080gsp.h`,
            // and the fourth is the string the guest itself sent at fn 1 — so the value
            // this port reports is MEASURED from this boot's guest rather than projected
            // from a build-time constant that could silently disagree with it. The two
            // constants that look right and are not (`host_driver`'s version, and
            // `self.driver.version()` = 580.65.06) are laid out in `kayfabe_abi::gspfeatures`.
            //
            // ⚠ The error arm is a whole-control refusal and that is the right size: the
            // field is report-only (no RM reader — see the abi module), so refusing costs
            // a `cuInit` rung and a loud ledger row, while answering with an unlatched or
            // unvalidated string would put a value this port invented in front of a user.
            WantedTable::GspGetFeatures => {
                let Some(firmware) = self.guest_firmware else {
                    return refuse();
                };
                kayfabe_abi::gspfeatures::encode_gsp_get_features(
                    // `NV2080_CTRL_GSP_GET_FEATURES_UVM_ENABLED_TRUE`, and bit 1 clear:
                    // `VGPU_GSP_MIG_REFACTORING` is a MIG feature and this is a GeForce
                    // part — the same fact `kayfabe_abi::smcmode` reports as `Unsupported`.
                    kayfabe_abi::gspfeatures::GspFeatures::GA106,
                    // `bValid` — truthful rather than copied. The header defines it as "RM
                    // is a GSP client with GPU support offloaded to GSP firmware", which is
                    // exactly what this port arranges.
                    true,
                    // `bDefaultGspRmGpu` — GSP-RM is on by default for Ampere GeForce.
                    true,
                    &firmware,
                )
            }
            // ★★★ The first GSS-legacy id this port answers, and the first arm whose reply
            // is the request VERBATIM. See `kayfabe_abi::gsslegacy` for why that is a
            // `[measured 2026-08-09, real GA106 on 580.159.04]` fact here — the path's
            // copy-out is unconditional on `NV_OK` and has no `SKIP_COPYOUT` to hide behind
            // — and an invention everywhere else.
            //
            // ⊘ The bytes are copied through this arm rather than left to the tail's
            // `copy_from_slice` no-op on purpose: the identity is then something the code
            // SAYS, and `answer_gss_legacy` is where the id and the length are checked.
            WantedTable::GssLegacy8159 | WantedTable::GssLegacy8162 => {
                let at = req.params_at;
                match kayfabe_abi::gsslegacy::answer_gss_legacy(
                    req.cmd,
                    &cmd.payload[at..at + want.params_size()],
                ) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
            // ★★ The one arm whose value is an ARGUMENT rather than a capture, and the flag
            // is passed rather than assumed: `c2c_absent` refuses for a part that HAS C2C,
            // so a future chip row cannot inherit "no links" by silence. GA106 has none.
            //
            // ⊘⊘ **This line shipped INVERTED (`!self.chip.has_c2c`) and a boot caught it, not
            // a test.** `kayfabe_abi::c2cinfo`'s own unit tests were green — both arms of
            // `c2c_absent` are covered there — because the defect was at the CALL SITE, in
            // the one place those tests cannot reach. `[measured 2026-08-09, boot `gf1437` at
            // `e7bb8c6`]` row 86 still answered `0x56` with the control fully "served".
            // ★ The lesson is `a_signature_is_not_the_dispatch` in its cheapest form: a
            // correct function reached with a negated argument is indistinguishable from an
            // unimplemented one at every boundary except the wire. `tests/bus_get_c2c_info.rs`
            // is the reply-plane test that now bites it.
            WantedTable::C2cInfo => match kayfabe_abi::c2cinfo::c2c_absent(self.chip.has_c2c) {
                Ok(p) => p,
                Err(_) => return refuse(),
            },
            // ★★★ §14.43 — the **acknowledgement** arm, and the only one in this table whose
            // reply contains no fact of this port's own. Every field of
            // `NVA06C_CTRL_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS_PARAMS` is `[input]`, so the
            // whole content of a correct answer is *"received, and here is what I read"*.
            //
            // ⊘ The re-encode is deliberately **not** `raw.to_vec()`, which is what
            // `WantedTable::RegisterAccessCntrBuffer` does one arm above. Copying the slice
            // would let a byte the validation rejected reach the guest anyway, by travelling
            // around the check inside the same buffer; re-encoding from the decoded view makes
            // the reply structurally incapable of carrying a record the decoder did not
            // accept. That is the same argument `encode_event_set_notification` makes for not
            // echoing, arrived at from the opposite direction.
            //
            // ⚠ `refuse()` here answers `NV_ERR_NOT_SUPPORTED`, which the caller turns into a
            // failed channel-group construct — the wall this arm exists to remove. So the
            // error branch is genuinely the *worse* outcome and is still right: each shape it
            // declines is one this port cannot name (a runqueue count past the array, an
            // aperture that is neither sysmem nor FB, a sized buffer at address zero), and
            // answering `NV_OK` to those would be claiming to have recorded something we
            // could not read.
            WantedTable::PromoteFaultMethodBuffers => {
                let at = req.params_at;
                let Some(raw) = cmd.payload.get(at..at + want.params_size()) else {
                    return refuse();
                };
                match kayfabe_abi::fmbpromote::decode_promote_fault_method_buffers(raw) {
                    Ok(req) => kayfabe_abi::fmbpromote::encode_promote_fault_method_buffers(&req),
                    Err(_) => return refuse(),
                }
            }
        };

        // ★★ The sticky-answer tripwire, at the serve site rather than in a comment. The
        // reply keeps the request's `rmctrlFlags`, and for a GSS-legacy control those flags
        // are what branch (b) reads to cache our answer PERMANENTLY
        // (`rmapiControlCacheSetUnchecked`, `ogkm-580: rpc.c:11096-11103`).
        //
        // ⚠⚠ **§14.36 NARROWED this, and the narrowing is the load-bearing part.** It used to
        // refuse every GSS-legacy id, on the stated ground that *"every id this port serves is
        // outside that mask today, so this is unreachable"*. That ceased to be true the moment
        // `GssLegacy8159` was served, so the tripwire had to become the statement it always
        // meant: **an id reaches here only by being in [`WantedTable::ALL`]**, i.e. only by a
        // deliberate, measured decision — so what this guards against is not "a GSS-legacy id"
        // but "a GSS-legacy id nobody chose", and `from_cmd` already makes that impossible.
        //
        // ⊘ Deleting it outright would have been wrong for a different reason than keeping it:
        // the cache is closed by `crate::sticky::StickyAnswerGuard` zeroing both flag words on
        // every accepted reply (its `Guarded` row in `POLICY_DISPOSITIONS`, executed by
        // `tests/tests/sticky_answer.rs`), which is a property of the CHAIN. This policy is
        // composable and can be seated without that link — `kayfabe-crec`'s replay does
        // exactly that. So the obligation is named here and discharged there, and the one
        // served id records that its answer is safe to cache anyway: it is the identity on the
        // guest's own buffer, so a cache that replayed it would replay what the guest sent.
        if is_gss_legacy(req.cmd)
            && !matches!(
                want,
                WantedTable::GssLegacy8159 | WantedTable::GssLegacy8162
            )
        {
            return refuse();
        }

        // Keep the guest's own control header — `hClient`/`hObject`/`cmd` are echoed, as
        // they are on every real reply — and overwrite only the two fields a GSP owns.
        let mut body = cmd.payload.clone();
        body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4].copy_from_slice(&NV_OK.to_le_bytes());
        let size = u32::try_from(params.len()).unwrap_or(u32::MAX);
        body[CONTROL_PARAMS_SIZE_OFF..CONTROL_PARAMS_SIZE_OFF + 4]
            .copy_from_slice(&size.to_le_bytes());
        body[req.params_at..req.params_at + params.len()].copy_from_slice(&params);

        Some(Reply {
            rpc_result: NV_OK,
            body,
        })
    }
}

kayfabe_util::assert_send_sync!(InitTablePolicy, WantedTable);
