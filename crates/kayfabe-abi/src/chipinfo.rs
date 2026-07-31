//! The `GSP_RM_CONTROL` reply body for `NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO` — the
//! control that tells the guest's RM **who this device is**, and where inside its register
//! aperture four register groups begin.
//!
//! ## Why the guest cannot start without it
//!
//! `_gpuInitChipInfo` allocates the params, zeroes them, issues this control under
//! `NV_ASSERT_OK_OR_GOTO`, and on success copies three of the six fields straight into
//! `pGpu->idInfo` (`ogkm-580: src/nvidia/src/kernel/gpu/gpu.c:874-901`). A refusal frees
//! `pGpu->pChipInfo`, leaves it `NULL`, and ends `RmInitNvDevice` — `[measured]` run
//! `t127c`, a stock 580.159.04 guest at `110c857`:
//!
//! ```text
//! NVRM: [NV_ERR_NOT_SUPPORTED] (0x56) from pRmApi->Control(…, NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO,
//!       pGpu->pChipInfo, paramSize) @ gpu.c:886
//! NVRM: … from _gpuInitChipInfo(pGpu) @ gpu.c:2124
//! NVRM: RmInitNvDevice: *** Cannot pre-initialize the device
//! ```
//!
//! ★★ It is **served**, not inert: the guest reads six `[OUT]` fields out of the reply and
//! builds on every one of them. `kayfabe_device::inert`'s own eligibility rule — *"the
//! guest reads nothing but the status"* — excludes it by inspection.
//!
//! ## ★★ What the oracle settled, and what it did not
//!
//! `C: src/qemu/mode2_initctrl_ga106.h:3355-3364` (`ctl_20800a36`, 88 bytes, registered at
//! `:6248` as `{0x20800a36u, 0x0u, 88u, 88u, ctl_20800a36}`) appears **verbatim, exactly
//! once** in the 360 725 records of
//! `traces/mode2_c_reference/cap1b_coldboot_hermetic_d6` — record **141991**, a 4096-byte
//! `GUEST_WR` to `0x1_2764_6000`, the blob at payload offset **120**, under an RPC envelope
//! reading `header_version=0x03000000 signature="VRPC" length=160 function=76
//! rpc_result=NV_OK sequence=3` and a control header carrying `hClient=0xc2000006
//! hObject=0xabcd2080 cmd=0x20800a36 status=NV_OK paramsSize=88 rmapiRpcFlags=0`.
//! `160 = 32 (envelope) + 40 (`RpcControlReq::HEADER`) + 88 (params)`, the same arithmetic
//! that made the three layouts agree for `kayfabe_abi::pcibars`.
//!
//! So the **layout** below — the pad byte after `chipSubRev`, the three pad bytes after
//! `isCmpSku`, and `regBases[]` starting at 24 — is answered by an RTX 3060 rather than by
//! a header read, and `crates/kayfabe-device/tests/chip_info.rs` pins this encoder against
//! those bytes.
//!
//! ⊘ It settles **nothing about which values to serve**. The oracle's board had silicon
//! behind every register base it named; ours has a register plane that decodes some
//! offsets and answers zero everywhere else (`kayfabe_device::plane`). That difference is
//! the whole content of [`RegBaseRow`]'s doc below.
//!
//! ## ★★★ `0xFFFFFFFF` is the protocol's OWN named refusal, and it is per-entry
//!
//! This is the property that makes it possible to *serve* this control honestly while
//! declining most of what it can carry. RM's physical side fills the array by asking its
//! own HAL and writing the sentinel wherever the HAL says no:
//!
//! ```text
//! for (i = 0; i < NV_ARRAY_ELEMENTS(pParams->regBases); i++)
//!     if (gpuGetRegBaseOffset_HAL(pGpu, i, &pParams->regBases[i]) != NV_OK)
//!         pParams->regBases[i] = 0xFFFFFFFF;
//! ```
//! (`ogkm-580: src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_internal_kernel.c:52-56`)
//!
//! and the guest's reader turns it straight back into an error rather than an address:
//!
//! ```text
//! if (pChipInfo->regBases[regBase] != 0xFFFFFFFF) { *pOffset = …; return NV_OK; }
//! return NV_ERR_NOT_SUPPORTED;
//! ```
//! (`ogkm-580: src/nvidia/src/kernel/gpu/gpu_gspclient.c:315-326`,
//! `gpuGetRegBaseOffset_FWCLIENT`)
//!
//! ⊘ So an unlisted index is **not** a zero-shaped lie of the #125 kind. It is a refusal in
//! the vocabulary the guest already speaks, delivered at the exact call site that wanted
//! the base — which is the same one-rung-per-boot property `kayfabe_device::unserviced`
//! relies on, moved one layer inward.
//!
//! ## ⚠ `emulationRev1` is served as zero, and that is a claim, not a gap
//!
//! `subdeviceCtrlCmdInternalGetChipInfo_IMPL` never writes the field
//! (`subdevice_ctrl_internal_kernel.c:44-58` — six assignments, none of them
//! `emulationRev1`), and `_gpuInitChipInfo` `portMemSet`s the params to zero before the
//! call (`gpu.c:884`), so **zero is what a real board's CPU-RM ends up holding** and the
//! oracle's captured bytes agree. Its one reader is `gpuGetEmulationRev1_FWCLIENT`
//! (`gpu_gspclient.c:195-203`); a non-zero value means *"this part is an emulation or FPGA
//! platform"*, which this port is not claiming to be. It is a constant here rather than a
//! chip row because no chip row could honestly vary it.

/// `NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO` — the control `_gpuInitChipInfo` issues on
/// `hInternalClient`/`hInternalSubdevice`.
///
/// `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:571`.
pub const NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO: u32 = 0x2080_0a36;

/// `NV2080_CTRL_INTERNAL_GET_CHIP_INFO_REG_BASE_MAX` — how many register bases the reply
/// carries (`ogkm-580: ctrl2080internal.h:577`), and the bound
/// `gpuGetRegBaseOffset_FWCLIENT` asserts a caller's index against
/// (`ogkm-580: gpu_gspclient.c:317`).
pub const CHIP_INFO_REG_BASE_MAX: usize = 16;

/// Byte offset of `chipSubRev` — an `NvU8` at the head of the struct.
pub const CHIP_SUB_REV_OFF: usize = 0;

/// Byte offset of `emulationRev1`.
///
/// ★ `chipSubRev` is one byte and this is an `NvU32`, so the compiler inserts **three
/// bytes of padding no field name mentions**. Encoding this at 1 would shift every field
/// behind it.
pub const EMULATION_REV1_OFF: usize = 4;

/// Byte offset of `isCmpSku` — an `NvBool`, i.e. one byte
/// (`ogkm-580: src/common/sdk/nvidia/inc/nvtypes.h`, `NvBool` is `NvU8`).
pub const IS_CMP_SKU_OFF: usize = 8;

/// Byte offset of `pciDeviceId`. Three more bytes of padding precede it, after `isCmpSku`.
pub const PCI_DEVICE_ID_OFF: usize = 12;

/// Byte offset of `pciSubDeviceId`.
pub const PCI_SUB_DEVICE_ID_OFF: usize = 16;

/// Byte offset of `pciRevisionId`.
pub const PCI_REVISION_ID_OFF: usize = 20;

/// Byte offset of `regBases[0]`.
pub const REG_BASES_OFF: usize = 24;

/// `sizeof(NV2080_CTRL_INTERNAL_GPU_GET_CHIP_INFO_PARAMS)` — the `[OUT]` struct RM
/// allocates, and therefore the `paramSize` it declares (`ogkm-580: gpu.c:879`) and the
/// number of bytes it copies back (`ogkm-580: rpc.c:11083`).
pub const CHIP_INFO_PARAMS_SIZE: usize = REG_BASES_OFF + CHIP_INFO_REG_BASE_MAX * 4;

/// ★★★ RM's own spelling for *"this device does not have that register group"*.
///
/// Written by the physical side for every index its HAL declines
/// (`ogkm-580: subdevice_ctrl_internal_kernel.c:55`) and turned back into
/// `NV_ERR_NOT_SUPPORTED` by the client side (`ogkm-580: gpu_gspclient.c:319-325`). See
/// this module's docs: it is why an unlisted base is a refusal rather than a wrong address.
pub const REG_BASE_UNSUPPORTED: u32 = 0xFFFF_FFFF;

/// `emulationRev1`, as served. See this module's docs for why it is a constant.
pub const EMULATION_REV1_REAL_SILICON: u32 = 0;

/// RM's own `NV_REG_BASE_*` indices into `regBases[]`, named so a chip row and the guest
/// cannot silently mean different slots.
///
/// `ogkm-580: src/nvidia/generated/g_gpu_nvoc.h:5828-5834`, which also carries
/// `NV_REG_BASE_LAST = NV_REG_BASE_USERMODE` and a `ct_assert` that it stays below
/// [`CHIP_INFO_REG_BASE_MAX`].
pub mod reg_base {
    /// `NV_REG_BASE_GR` (`g_gpu_nvoc.h:5829`).
    ///
    /// ⊘ `[inferred]` — a grep of a source tree is a read, not a run: **no reader at all**
    /// in `ogkm-580`. The identifier appears exactly once in the whole tree, at
    /// `ogkm-580: src/nvidia/generated/g_gpu_nvoc.h:5829`, its own definition.
    pub const GR: usize = 1;
    /// `NV_REG_BASE_TIMER` (`:5830`). Read by `tmrapiGetRegBaseOffsetAndSize_IMPL`
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/timer/timer.c:1712-1734`) so a client can
    /// map the timer block, and by `subdeviceCtrlCmdTimerGetRegisterOffset`
    /// (`ogkm-580: subdevice_ctrl_timer_kernel.c:287`).
    pub const TIMER: usize = 2;
    /// `NV_REG_BASE_MASTER` (`:5831`). Read only by `GF100_SUBDEVICE_MASTER`'s
    /// read-only client mapping (`ogkm-580: generic_engine.c:118-135`).
    pub const MASTER: usize = 3;
    /// `NV_REG_BASE_USERMODE` (`:5832`).
    ///
    /// ★★ The one on the **initialisation** path: `kfifoConstructUsermodeMemdescs_GV100`
    /// asks for it under `NV_ASSERT_OK_OR_RETURN` and builds `pKernelFifo->pRegVF` from
    /// the answer (`ogkm-580: kernel_fifo_gv100.c:353-378`), and its caller
    /// `kfifoStateInit` asks under `NV_ASSERT_OK_OR_RETURN` too
    /// (`ogkm-580: kernel_fifo_init.c:232`). A device that declines this one cannot
    /// finish `RmInitAdapter`.
    pub const USERMODE: usize = 4;
    /// `NV_REG_BASE_LAST` (`:5833`) — the largest index RM's own HAL can be asked for.
    pub const LAST: usize = USERMODE;
}

/// ★★ **One register group this device is willing to name — an aperture the guest will go
/// on to MAP, not a number it merely stores.**
///
/// Every consumer of a base turns it into a mapping: `kfifoConstructUsermodeMemdescs_GV100`
/// builds an `ADDR_REGMEM` memdesc at it (`ogkm-580: kernel_fifo_gv100.c:371-374`),
/// `generic_engine.c` maps it into a client's address space (`:132-146`), and
/// `tmrapiGetRegBaseOffsetAndSize_IMPL` hands it to `rmapiMapToCpu`. So a row here is a
/// promise that base-address register 0 plus `offset` is a range this device's register
/// plane really answers.
///
/// ⊘ An index with no row is **not** a defaulted zero: it is
/// [`REG_BASE_UNSUPPORTED`], which the guest's own reader converts to
/// `NV_ERR_NOT_SUPPORTED` at the call site that wanted it. Leaving a group out is
/// therefore a *diagnosis*, and adding one we cannot back is the failure that lands far
/// from here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegBaseRow {
    /// RM's own index — one of [`reg_base`]'s constants.
    pub index: usize,
    /// Byte offset of the group's first register within base-address register 0.
    pub offset: u32,
    /// What the group is, for diagnostics. Never branched on, never sent.
    pub name: &'static str,
}

/// ★ The per-chip half of the reply: the two silicon facts that are not already stated by
/// the device's PCI identity, plus the register groups the chip declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChipInfoRow {
    /// `chipSubRev`.
    ///
    /// Its only reader is `NV2080_CTRL_CMD_MC_GET_ARCH_INFO`'s `subRevision`
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mc/kernel_mc.c:66`), which reports it to a
    /// client and branches on nothing.
    pub chip_sub_rev: u8,
    /// `isCmpSku` — whether this part is one of the compute-only mining SKUs.
    ///
    /// Its only reader is `NV2080_CTRL_GPU_INFO_INDEX_CMP_SKU`
    /// (`ogkm-580: subdevice_ctrl_gpu_kernel.c:491-501`), which turns it into a
    /// `_YES`/`_NO` a client reads.
    pub is_cmp_sku: bool,
    /// The register groups this chip names. See [`RegBaseRow`] — an omission is a
    /// refusal, not a gap.
    pub reg_bases: &'static [RegBaseRow],
}

/// The device's PCI identity, in the packing this reply uses.
///
/// ★★ Assembled by the caller from the **same** source configuration space is served from,
/// so the reply and the config header cannot drift: `_gpuInitChipInfo` **overwrites**
/// `pGpu->idInfo` with these three fields (`ogkm-580: gpu.c:891-893`), discarding whatever
/// the guest's own PCI enumeration had put there. A reply that disagreed with the header
/// would leave RM believing a device that is not the one it enumerated, and nothing logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChipIdentity {
    /// PCI vendor id — the **low** half of `pciDeviceId`.
    pub pci_vendor_id: u16,
    /// PCI device id — the **high** half of `pciDeviceId`.
    pub pci_device_id: u16,
    /// Subsystem vendor id — the low half of `pciSubDeviceId`.
    pub pci_subsystem_vendor_id: u16,
    /// Subsystem device id — the high half of `pciSubDeviceId`.
    pub pci_subsystem_id: u16,
    /// PCI revision id.
    pub pci_revision: u8,
}

impl ChipIdentity {
    /// `pciDeviceId`, as RM packs it: **device in the high half, vendor in the low**.
    ///
    /// ★ Not a guess and not the obvious order. `ogkm-580: gpu.c:6297-6299` says so in a
    /// comment — *"Upper half of `pGpu->idInfo.PCIDeviceID` is devid"* — and then reads it
    /// with `DRF_VAL(_PCI, _SUBID, _DEVICE, …)`, i.e. bits 31:16;
    /// `ogkm-580: src/nvidia/src/kernel/gpu/external_device/arch/kepler/kern_gsync_p2060.c:5506`
    /// spells the same shift out longhand. The oracle's captured `0x250410de` for an RTX
    /// 3060 (device `0x2504`, vendor `0x10de`) is the third witness.
    #[must_use]
    pub fn packed_device_id(&self) -> u32 {
        u32::from(self.pci_device_id) << 16 | u32::from(self.pci_vendor_id)
    }

    /// `pciSubDeviceId`, packed the same way — subsystem device high, subsystem vendor
    /// low. The oracle's `0x397d1462` is an MSI (`0x1462`) board `0x397d`.
    #[must_use]
    pub fn packed_sub_device_id(&self) -> u32 {
        u32::from(self.pci_subsystem_id) << 16 | u32::from(self.pci_subsystem_vendor_id)
    }
}

/// Why the reply could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipInfoError {
    /// A row names an index `regBases[]` does not hold. `gpuGetRegBaseOffset_FWCLIENT`
    /// `NV_ASSERT`s the bound before indexing (`ogkm-580: gpu_gspclient.c:317`), so such a
    /// row could never be read anyway — and writing it would run off the params.
    RegBaseIndexOutOfRange {
        /// The row that named it.
        name: &'static str,
        /// The index it named.
        index: usize,
        /// The array bound.
        max: usize,
    },
    /// Two rows name one index. Refused rather than resolved by order: the loser would
    /// simply never be encoded, and which of two disagreeing rows an operator meant is not
    /// ours to decide.
    DuplicateRegBase {
        /// The index both rows named.
        index: usize,
        /// The row encoded first.
        first: &'static str,
        /// The row that collided with it.
        second: &'static str,
    },
    /// A row's offset is [`REG_BASE_UNSUPPORTED`].
    ///
    /// ★ That value is the protocol's own *"not present"*, so a row claiming it would
    /// advertise a group and simultaneously deny it — the guest would read the denial and
    /// the operator would read the row.
    RegBaseIsSentinel {
        /// The row that named it.
        name: &'static str,
    },
    /// A row's offset lies outside base-address register 0.
    ///
    /// ★★ This is [`RegBaseRow`]'s promise made mechanical. The guest maps
    /// `BAR0 + offset`; a base past the aperture is a mapping its own MMU cannot make, and
    /// the failure would surface as a client mapping error with no mention of this table.
    RegBaseOutsideAperture {
        /// The row that named it.
        name: &'static str,
        /// The offset it named.
        offset: u32,
        /// The aperture's length.
        aperture: u64,
    },
}

impl core::fmt::Display for ChipInfoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::RegBaseIndexOutOfRange { name, index, max } => write!(
                f,
                "register base {name:?} claims index {index}; regBases[] holds {max}"
            ),
            Self::DuplicateRegBase {
                index,
                first,
                second,
            } => write!(
                f,
                "register bases {first:?} and {second:?} both claim index {index}"
            ),
            Self::RegBaseIsSentinel { name } => write!(
                f,
                "register base {name:?} is {REG_BASE_UNSUPPORTED:#010x}, which is RM's own \
                 spelling for \"this device has no such group\""
            ),
            Self::RegBaseOutsideAperture {
                name,
                offset,
                aperture,
            } => write!(
                f,
                "register base {name:?} is at {offset:#x}, outside the {aperture:#x}-byte \
                 register aperture the guest maps it through"
            ),
        }
    }
}

impl core::error::Error for ChipInfoError {}

/// Encode `NV2080_CTRL_INTERNAL_GPU_GET_CHIP_INFO_PARAMS` from a chip's row and identity.
///
/// The whole [`CHIP_INFO_PARAMS_SIZE`]-byte struct is returned. `regBases[]` is filled with
/// [`REG_BASE_UNSUPPORTED`] **first** and the declared rows are written over it, so an
/// index nobody declared is RM's own refusal rather than the zero a `vec![0u8; …]` would
/// leave — and `regBases[0]` is a real example: RM's `NV_REG_BASE_*` numbering starts at 1,
/// so slot 0 has no meaning at all and the oracle's board sent the sentinel there.
///
/// # Errors
///
/// [`ChipInfoError::RegBaseIndexOutOfRange`], [`ChipInfoError::DuplicateRegBase`],
/// [`ChipInfoError::RegBaseIsSentinel`] or [`ChipInfoError::RegBaseOutsideAperture`].
pub fn encode_chip_info(
    row: &ChipInfoRow,
    id: &ChipIdentity,
    regs_aperture_len: u64,
) -> Result<Vec<u8>, ChipInfoError> {
    let mut params = vec![0u8; CHIP_INFO_PARAMS_SIZE];

    params[CHIP_SUB_REV_OFF] = row.chip_sub_rev;
    params[EMULATION_REV1_OFF..EMULATION_REV1_OFF + 4]
        .copy_from_slice(&EMULATION_REV1_REAL_SILICON.to_le_bytes());
    params[IS_CMP_SKU_OFF] = u8::from(row.is_cmp_sku);
    params[PCI_DEVICE_ID_OFF..PCI_DEVICE_ID_OFF + 4]
        .copy_from_slice(&id.packed_device_id().to_le_bytes());
    params[PCI_SUB_DEVICE_ID_OFF..PCI_SUB_DEVICE_ID_OFF + 4]
        .copy_from_slice(&id.packed_sub_device_id().to_le_bytes());
    params[PCI_REVISION_ID_OFF..PCI_REVISION_ID_OFF + 4]
        .copy_from_slice(&u32::from(id.pci_revision).to_le_bytes());

    // The refusal is the DEFAULT, and it is written before anything is served.
    for i in 0..CHIP_INFO_REG_BASE_MAX {
        let o = REG_BASES_OFF + i * 4;
        params[o..o + 4].copy_from_slice(&REG_BASE_UNSUPPORTED.to_le_bytes());
    }

    let mut claimed: [Option<&'static str>; CHIP_INFO_REG_BASE_MAX] =
        [None; CHIP_INFO_REG_BASE_MAX];
    for b in row.reg_bases {
        if b.index >= CHIP_INFO_REG_BASE_MAX {
            return Err(ChipInfoError::RegBaseIndexOutOfRange {
                name: b.name,
                index: b.index,
                max: CHIP_INFO_REG_BASE_MAX,
            });
        }
        if let Some(first) = claimed[b.index] {
            return Err(ChipInfoError::DuplicateRegBase {
                index: b.index,
                first,
                second: b.name,
            });
        }
        if b.offset == REG_BASE_UNSUPPORTED {
            return Err(ChipInfoError::RegBaseIsSentinel { name: b.name });
        }
        if u64::from(b.offset) >= regs_aperture_len {
            return Err(ChipInfoError::RegBaseOutsideAperture {
                name: b.name,
                offset: b.offset,
                aperture: regs_aperture_len,
            });
        }
        claimed[b.index] = Some(b.name);
        let o = REG_BASES_OFF + b.index * 4;
        params[o..o + 4].copy_from_slice(&b.offset.to_le_bytes());
    }

    Ok(params)
}
