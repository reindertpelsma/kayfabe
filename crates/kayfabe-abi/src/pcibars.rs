//! The `GSP_RM_CONTROL` reply body for `NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO` — the one
//! control whose *echo* was measured to fault the guest's own kernel.
//!
//! ## ★★★ Why this control is different from every other unmodelled one
//!
//! `kbusInitBarsSize_KERNEL` declares its params on the stack with **no initialiser and no
//! `portMemSet`** (`ogkm-580: src/nvidia/src/kernel/gpu/bus/kern_bus.c:583-593`,
//! `ogkm-610: :583-593` — identical), RM serialises that struct into the RPC as-is, and the
//! caller then trusts the reply's `pciBarCount` as a loop bound over an eight-element array
//! (`ogkm-580: kern_bus.c:595-598`). A policy that reflects the request hands RM back its
//! own uninitialised stack and RM reads off the end of it. `[measured]` run `t126b`, a
//! stock 580.159.04 guest at `f2acb89` driven with `nvidia-smi`, twice on two fresh boots:
//!
//! ```text
//! BUG: unable to handle page fault for address: ffffd4a44035c000
//! RIP: 0010:kbusInitBarsSize_KERNEL+0x90/0x110 [nvidia]
//!  kbusStatePreInitLocked_GM107+0x73/0xe0 [nvidia]
//!  RmInitAdapter+0x12f2/0x1e80 [nvidia]
//! ```
//!
//! That measurement is why [`kayfabe_gsp::EchoOk`] is no longer a default anywhere: see its
//! docs, and `kayfabe_gsp::GspFsm::answer`'s refusal.
//!
//! ## ★★ What the oracle settled, and what it did not
//!
//! `C: src/qemu/mode2_initctrl_ga106.h:5396-5409` (`ctl_20801803`, 200 bytes, registered at
//! `:6256` as `{0x20801803u, 0x0u, 200u, 200u, ctl_20801803}`) appears **verbatim, exactly
//! once**, in `traces/mode2_c_reference/cap1b_coldboot_hermetic_d6` — record **142070**, a
//! 4096-byte `GUEST_WR` to `0x1_2765_7000`, the blob at payload offset 120, under an RPC
//! envelope reading `header_version=0x03000000 signature="VRPC" length=272 function=76
//! sequence=12` and a control header `cmd=0x20801803 status=NV_OK paramsSize=200`. `272 =
//! 32 (envelope) + 40 (`RpcControlReq::HEADER`) + 200 (params)`, which is the arithmetic
//! that makes the three layouts agree.
//!
//! So the **layout** is answered by an RTX 3060 rather than by a header read, and
//! `crates/kayfabe-device/tests/pci_bar_info.rs` pins this encoder against those bytes.
//!
//! ## ⚠ THIS REPLY HAS STILL NEVER BEEN EXERCISED BY A GUEST — and the reason is measured
//!
//! ★★★ **`[measured]` run `t134a`, a stock 580.159.04 guest at `1c79474`: a live guest has
//! now asked for this reply, and been served it.** Everything below used to be inference
//! about a code path no boot reached; the reach is now a measurement and the paragraph
//! after it records how it was read.
//!
//! `kbusInitBarsSize_KERNEL` is reached from `kbusStatePreInitLocked_GM107`, i.e. from the
//! engine-descriptor `engstateStatePreInit` loop that `gpuPreInit` runs at
//! `ogkm-580: src/nvidia/src/kernel/gpu/gpu.c:2146-2170`. `t133a` (at `c88f803`) stopped
//! four statements short of it, at `:2126`. `t134a` served that rung
//! ([`crate::falconinfo`]) and the boot ran **past the whole loop** — its dmesg reaches
//! `gpuStatePreInit_IMPL` at `:2204` and then `gpuStateInit_IMPL`, both of which are after
//! `:2170`.
//!
//! | line | statement | asks for | `t134a` |
//! |---|---|---|---|
//! | `:2126` | `gpuBuildGenericKernelFalconList` | `0x208001b0` | served |
//! | `:2129` | `gpuBuildKernelVideoEngineList` | `0x208001b0` again | served |
//! | `:2131-2140` | the `PDB_PROP_GPU_MIG_SUPPORTED` block | — | passed |
//! | `:2143` | `gpuRemoveMissingEngines` | — | passed |
//! | `:2145` | `pGpu->bFullyConstructed = NV_TRUE` | — | passed |
//! | `:2146-2170` | the `engstateStatePreInit` loop | **this reply** | **reached** |
//!
//! ★★ **How "served" was read, since the ledger only lists refusals.** `t134a`'s host-side
//! ledger names six distinct unserviced controls and `0x20801803` is **not among them**,
//! while `0x20800af3` and `0x20800aac` — which the oracle `cap1b` carries *after* it, at
//! `rpc.sequence` 13 and 14 against this reply's 12 — **are**. A control that was asked and
//! refused would appear; one that was never reached could not be followed by two that were.
//! Together with the dmesg reaching `:2204`, that is the reply being asked for and answered.
//!
//! ⊘ **What that does NOT establish.** The thing this module said to read then was *"whether
//! `pciBarCount = 4` survives into `pKernelBus->pciBars[]` without the page fault run
//! `t126b` recorded"*. The absence of a `kern_bus.c` line in `t134a`'s NVRM log is
//! consistent with that and is **not** the same as observing the array. `t134a` ended in a
//! guest-kernel `Oops` from an unrelated refusal (`kayfabe_device::unserviced` carries the
//! chain), so nothing downstream of the heap was exercised. `barOffset` is still served as
//! zero — see below — and no boot has yet shown a consumer reading it.
//!
//! ⊘ The oracle settles **nothing about the values**. Its `barOffset` fields are the *host*
//! board's physical addresses (`0xc000_0000`, `0x10_0000_0000`, `0x10_1000_0000`) and its
//! sizes are that board's; ours must be the ones **this** device presents, which is why the
//! rows live on `kayfabe_device::ChipProfile::pci_bars`.
//!
//! ## ★★ `barOffset` is served as zero, and that is a stated limit, not an oversight
//!
//! This port has no source for it. The device's guest-physical base addresses are assigned
//! by the guest's own PCI enumeration and are known to the *memory* plane
//! (`kayfabe_shim_note_bar_mapping`), not to the chip row this encoder reads.
//!
//! What makes the zero safe rather than the #125 shape of lie — where a zero *count* made
//! RM read "this device has nothing" and refuse with `NV_ERR_NO_MEMORY` — is that
//! **nothing reads it**. `[measured]` grep for `pciBarInfo` and for the reply's `barOffset`
//! across all of `src/` and `kernel-open/` in both vendored trees: the only readers of a
//! `NV2080_CTRL_BUS_GET_PCI_BAR_INFO_PARAMS` field are `kern_bus.c:597`, which copies
//! `barSizeBytes` and nothing else, and `kern_bus_ctrl.c:646-651`, which is the *writer*.
//! RM's own `pKernelBus->pciBars[]` is filled from `pGpu->busInfo` by
//! `kbusInitPciBars_GM107` (`ogkm-580: kern_bus_gm107.c:4702-4724`), i.e. from what the
//! guest's OS layer read out of Linux's PCI resources — the guest already knows where its
//! BARs are and never learns it from us.
//!
//! ⚠ That is a statement about the two **open** trees. A closed userspace client
//! (NVML/`nvidia-smi`) could read the field, and this port cannot grep it. If one is ever
//! observed to care, the fix is a real source — the memory plane's mapping — not a guess.

/// `NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO` — the control `kbusInitBarsSize_KERNEL` issues on
/// `hInternalClient`/`hInternalSubdevice`.
///
/// ogkm `src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080bus.h:638`.
pub const NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO: u32 = 0x2080_1803;

/// `NV2080_CTRL_BUS_MAX_PCI_BARS` — the array bound RM's loop runs against
/// (`ogkm-580: ctrl2080bus.h:641`).
pub const PCI_BAR_MAX: usize = 8;

/// `sizeof(NV2080_CTRL_BUS_PCI_BAR_INFO)` — `flags`, `barSize`, then two
/// `NV_DECLARE_ALIGNED(NvU64, 8)` (`ogkm-580: ctrl2080bus.h:612-617`).
pub const PCI_BAR_ENTRY_SIZE: usize = 24;

/// Byte offset of `pciBarInfo[]` inside the params.
///
/// ★ `pciBarCount` is one `NvU32` and the array is `NV_DECLARE_ALIGNED(..., 8)`, so the
/// compiler inserts **four bytes of padding no field name mentions**. Encoding the array at
/// 4 would put every entry one word low and RM would read a size out of a `flags` slot.
pub const PCI_BAR_INFO_OFF: usize = 8;

/// `sizeof(NV2080_CTRL_BUS_GET_PCI_BAR_INFO_PARAMS)` — the `[OUT]` struct RM allocates,
/// and therefore the `paramsSize` it declares and the number of bytes it copies back
/// (`ogkm-580: rpc.c:11083`).
pub const PCI_BAR_INFO_PARAMS_SIZE: usize = PCI_BAR_INFO_OFF + PCI_BAR_MAX * PCI_BAR_ENTRY_SIZE;

/// RM's own indices into `pciBars[]`/`pciBarSizes[]`, named so a chip row and a consumer
/// cannot silently mean different slots.
///
/// `ogkm-580: src/nvidia/generated/g_kern_bus_nvoc.h:172-176` (`BUS_BAR_0..3`,
/// `BUS_NUM_BARS = 4`), filled in that order by `kbusInitPciBars_GM107`
/// (`ogkm-580: kern_bus_gm107.c:4709-4720`).
pub mod bus_bar {
    /// `BUS_BAR_0` — the register aperture (`gpuPhysAddr`).
    pub const REGS: usize = 0;
    /// `BUS_BAR_1` — the framebuffer window (`gpuPhysFbAddr`).
    pub const FB: usize = 1;
    /// `BUS_BAR_2` — the instance/`BAR2` GMMU window (`gpuPhysInstAddr`).
    pub const INST: usize = 2;
    /// `BUS_BAR_3` — the I/O BAR (`gpuPhysIoAddr`).
    pub const IO: usize = 3;
    /// `BUS_NUM_BARS` — how many a classic dGPU reports, and therefore the
    /// `pciBarCount` its physical RM sends (`kern_bus_gm107.c:4717`).
    pub const NUM: usize = 4;
}

/// One base-address register, as the guest's RM enumerates them.
///
/// ⚠ These are **RM's** four logical BARs, not the six PCI slots: index 0 is the register
/// aperture, 1 the framebuffer window, 2 the instance/`BAR2` window and 3 the I/O BAR
/// (`ogkm-580: kern_bus_gm107.c:4709-4720`, which fills `pciBars[0..3]` from
/// `gpuPhysAddr`/`gpuPhysFbAddr`/`gpuPhysInstAddr`/`gpuPhysIoAddr`). A 64-bit window eats
/// two PCI slots and still counts once here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciBarRow {
    /// What this BAR is, for diagnostics. Never branched on, never sent.
    pub name: &'static str,
    /// `barSizeBytes` — the size of the aperture the device actually decodes.
    ///
    /// ⊘ Zero means **this BAR is not present**, which is a thing RM's own physical side
    /// encodes the same way (`kern_bus_gm107.c:407, 416` zero the entry for a disabled
    /// BAR1/BAR2) and which the oracle's row 3 is: an RTX 3060 has no I/O BAR.
    pub size_bytes: u64,
}

/// Why the table could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PciBarError {
    /// More rows than `pciBarInfo[]` holds. RM `NV_ASSERT`s `pciBarCount <=
    /// NV2080_CTRL_BUS_MAX_PCI_BARS` (`ogkm-580: kern_bus_ctrl.c:642`) and then loops to
    /// `pciBarCount` regardless, so an over-long table is the read-off-the-end this
    /// encoder exists to prevent.
    TooManyBars {
        /// How many rows were offered.
        len: usize,
        /// The array bound.
        max: usize,
    },
    /// A size that is not a whole number of megabytes.
    ///
    /// ★ Refused rather than truncated. `barSize` is the deprecated megabyte spelling of
    /// the same fact (`barSizeBytes >> 20`, `ogkm-580: kern_bus_ctrl.c:648-649`), so a
    /// row that does not divide leaves the reply stating **two different sizes** for one
    /// BAR, and which one a reader believes is not ours to decide.
    SizeNotWholeMegabytes {
        /// The BAR that declared it.
        name: &'static str,
        /// What it declared.
        size_bytes: u64,
    },
    /// A size that is not a power of two. A base-address register cannot have one — the
    /// hypervisor's own `pci_register_bar` would round it — so a row like this describes an
    /// aperture the guest does not see.
    SizeNotPowerOfTwo {
        /// The BAR that declared it.
        name: &'static str,
        /// What it declared.
        size_bytes: u64,
    },
}

impl core::fmt::Display for PciBarError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooManyBars { len, max } => write!(
                f,
                "{len} base-address registers; pciBarInfo[] holds {max}, and RM loops to \
                 the count we send"
            ),
            Self::SizeNotWholeMegabytes { name, size_bytes } => write!(
                f,
                "BAR {name:?} is {size_bytes} bytes, which is not a whole number of \
                 megabytes; barSize and barSizeBytes would disagree"
            ),
            Self::SizeNotPowerOfTwo { name, size_bytes } => write!(
                f,
                "BAR {name:?} is {size_bytes} bytes, which is not a power of two; a \
                 base-address register cannot have that size"
            ),
        }
    }
}

impl core::error::Error for PciBarError {}

/// Encode `NV2080_CTRL_BUS_GET_PCI_BAR_INFO_PARAMS` from a chip's BAR rows.
///
/// The whole [`PCI_BAR_INFO_PARAMS_SIZE`]-byte struct is returned, zero-filled, for the
/// same reason the init tables are: RM copies back exactly `paramsSize` bytes, so a reply
/// that encodes only the populated prefix leaves the tail holding whatever the request had
/// — and for *this* control the request is uninitialised stack.
///
/// # Errors
///
/// [`PciBarError::TooManyBars`], [`PciBarError::SizeNotWholeMegabytes`] or
/// [`PciBarError::SizeNotPowerOfTwo`].
pub fn encode_pci_bar_info(bars: &[PciBarRow]) -> Result<Vec<u8>, PciBarError> {
    if bars.len() > PCI_BAR_MAX {
        return Err(PciBarError::TooManyBars {
            len: bars.len(),
            max: PCI_BAR_MAX,
        });
    }
    for b in bars {
        if !b.size_bytes.is_multiple_of(1 << 20) {
            return Err(PciBarError::SizeNotWholeMegabytes {
                name: b.name,
                size_bytes: b.size_bytes,
            });
        }
        if b.size_bytes != 0 && !b.size_bytes.is_power_of_two() {
            return Err(PciBarError::SizeNotPowerOfTwo {
                name: b.name,
                size_bytes: b.size_bytes,
            });
        }
    }

    let mut params = vec![0u8; PCI_BAR_INFO_PARAMS_SIZE];
    let count = u32::try_from(bars.len()).unwrap_or(u32::MAX);
    params[0..4].copy_from_slice(&count.to_le_bytes());

    for (i, b) in bars.iter().enumerate() {
        let o = PCI_BAR_INFO_OFF + i * PCI_BAR_ENTRY_SIZE;
        // `flags` — the header defines no bit and the oracle sends none, so it stays the
        // zero the whole buffer already is. Spelled out so the omission is deliberate.
        let flags: u32 = 0;
        params[o..o + 4].copy_from_slice(&flags.to_le_bytes());
        let megabytes = u32::try_from(b.size_bytes >> 20).unwrap_or(u32::MAX);
        params[o + 4..o + 8].copy_from_slice(&megabytes.to_le_bytes());
        params[o + 8..o + 16].copy_from_slice(&b.size_bytes.to_le_bytes());
        // `barOffset` stays zero. `[inferred]` from `ogkm-580: kern_bus.c:597` and
        // `kern_bus_ctrl.c:646-651`: no reader in either vendored tree consumes the
        // reply's copy. This module's docs carry the grep and what would have to change.
    }
    Ok(params)
}
