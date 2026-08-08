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
    pub const ALL: [WantedTable; 27] = [
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
        }
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
                    &cmd.payload
                        [at..at + kayfabe_abi::businfo::BUS_GET_INFO_V2_PARAMS_SIZE],
                    &answers,
                ) {
                    Ok(p) => p,
                    Err(_) => return refuse(),
                }
            }
        };

        // ★★ The sticky-answer guard, at the serve site rather than in a comment. The reply
        // keeps the request's `rmctrlFlags`, and for a GSS-legacy control those flags let
        // the guest cache our answer PERMANENTLY (`rmapiControlCacheSetUnchecked`,
        // `ogkm-580: rpc.c:11096-11103`). Every id this port serves is outside that mask
        // today, so this is unreachable — and it is here precisely because nothing else
        // would notice the day it stops being. A refusal, not a panic: an id is data.
        if is_gss_legacy(req.cmd) {
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
