//! The command policy that answers the six `GSP_RM_CONTROL`s the guest's RM cannot start
//! without, from the chip row's own tables.
//!
//! ⚠ The type names say *table* and three of the six are not one:
//! `NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO` is an identity,
//! `NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP` is a **permission policy**,
//! and `NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO` is an **instruction to construct**
//! — none of which is a list. The names are kept because what actually unifies the six is
//! the property the module is about — **`[OUT]`-only, and a pure function of the chip
//! row** — and that holds for all six.
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
//! Six controls, all `[OUT]`-only, all answered from the chip row. It touches no RM graph
//! state, allocates no handle, and remembers nothing between commands. Every other command
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
use kayfabe_abi::chipinfo::{
    self, CHIP_INFO_PARAMS_SIZE, ChipIdentity, NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO,
};
use kayfabe_abi::falconinfo::{
    self, FALCON_INFO_PARAMS_SIZE, NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO,
};
use kayfabe_abi::inittables::{
    self, DEVICE_INFO_PARAMS_SIZE, INTR_PARAMS_SIZE, NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE,
    NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE,
};
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
/// not FINN-serialized, and the flags say cacheable. Our replies **reflect the request's
/// `rmctrlFlags`**, because the whole control header is kept — so for a GSS-legacy control
/// the guest would cache whatever we answered and never ask again.
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

/// Answers the FIFO device-info table and the kernel interrupt table from a chip row.
///
/// Every other command gets `None`, i.e. the FSM's own acknowledgement.
#[derive(Debug, Clone, Copy)]
pub struct InitTablePolicy {
    chip: &'static ChipProfile,
    driver: DriverAbiTable,
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
}

impl WantedTable {
    /// ★★ **Every control this policy serves**, as a value a caller can quantify over.
    ///
    /// A test that wants to say *"the differential exercises every served control"* has to
    /// get the universe from somewhere, and a list written in the test is the defect shape
    /// this repository has been bitten by most: shortening it weakens the gate with zero
    /// red tests. The list lives here, next to the `match` that consumes it, and
    /// [`WantedTable::cmd_id`] is what keeps the two in step — it is exhaustive over
    /// `Self`, so a new variant does not compile until it has an id, and
    /// `tests/init_tables.rs` walks this array through `cmd_id` → [`WantedTable::from_cmd`]
    /// and back.
    pub const ALL: [WantedTable; 7] = [
        Self::DeviceInfo,
        Self::IntrKernelTable,
        Self::PciBarInfo,
        Self::ChipInfo,
        Self::UserRegisterAccessMap,
        Self::ConstructedFalconInfo,
        Self::MemorySystemStaticConfig,
    ];

    /// The control id this table answers — the inverse of [`WantedTable::from_cmd`].
    ///
    /// ★ An exhaustive `match`, which is the mechanism: adding a variant to this enum
    /// stops the crate compiling until the id is stated, and the round trip in
    /// `tests/init_tables.rs` then fails until it is in [`WantedTable::ALL`] too.
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
        }
    }

    /// Classify a control command, or `None` if this policy does not model it.
    #[must_use]
    pub fn from_cmd(cmd: u32) -> Option<WantedTable> {
        match cmd {
            NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE => Some(Self::DeviceInfo),
            NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE => Some(Self::IntrKernelTable),
            NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO => Some(Self::PciBarInfo),
            NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO => Some(Self::ChipInfo),
            NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP => {
                Some(Self::UserRegisterAccessMap)
            }
            NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO => Some(Self::ConstructedFalconInfo),
            NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG => {
                Some(Self::MemorySystemStaticConfig)
            }
            _ => None,
        }
    }
}

impl InitTablePolicy {
    /// Build the policy for one chip and one guest driver's wire table.
    #[must_use]
    pub fn new(chip: &'static ChipProfile, driver: DriverAbiTable) -> InitTablePolicy {
        InitTablePolicy { chip, driver }
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
