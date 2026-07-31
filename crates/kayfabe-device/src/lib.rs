//! # `kayfabe-device` — the emulated device's **chip table** and its **register plane**
//!
//! Stage **Q4** of `l2_qemu_adapter.md`. Two things live here and nothing else:
//!
//! 1. [`ChipProfile`] — **one row per GPU generation**, carrying the facts a hypervisor
//!    shell must know before a guest driver will even bind: the PCI identity, the
//!    constructor for the generation's [`kayfabe_arch::GspModel`], the handful of
//!    silicon-constant registers, and the chip-adjacent windows (the VBIOS ROM aperture).
//! 2. [`RegPlane`] — the routing of one trapped register access into
//!    [`kayfabe_gsp::GspFsm`], and back out as a value.
//!
//! ## ★★★ Why this crate exists rather than the numbers going somewhere that already did
//!
//! Before stage Q4 the *only* consumer of a real register map was `kayfabe-crec`, the trace
//! differential — a harness. A shipped archive cannot depend on it (`kayfabe-crec` pulls in
//! `kayfabe-mocks`, whose own manifest says *"Test-only; never a production dependency"*),
//! so wiring a guest's accesses into the FSM meant either shipping the mocks or writing the
//! offsets out a second time. The second option is the failure this repository already
//! argued about at length for the VBIOS: **two descriptions of one chip that can disagree**.
//!
//! So the map moved here, `kayfabe_crec::ga10x` re-exports it, and the consequence is worth
//! stating plainly: the 359 062-record `cap1` replay now runs against **the same encoder
//! the guest reads through**.
//!
//! ## ★★ What is a table row, exactly
//!
//! Adding a GPU generation costs, in full:
//!
//! | thing | where | logic-crate edit? |
//! |---|---|---|
//! | the register map | a new module here, `impl GspModel` | no |
//! | the silicon constants ([`ChipProfile::boot_regs`]) | that module | no |
//! | the ROM the driver parses | one [`kayfabe_abi::vbios::VbiosProfile`] row | no |
//! | selecting all of it | one [`ChipProfile`] appended to [`CHIPS`] | no |
//!
//! `tests/chip_table.rs` proves the last row of that table by **doing it**: it declares a
//! chip whose register map is deliberately not GA10x, appends it to a table, and drives the
//! same [`RegPlane`] code through it. Nothing in `kayfabe-gsp`, `kayfabe-arch` or this
//! crate's `plane` module is touched to make that work.
//!
//! ## ★ The PCI identity has ONE source, and it is not this table
//!
//! [`ChipProfile`] names a `pci_device_id` and stops. The vendor id and the class code come
//! from [`kayfabe_abi::vbios::profile_for_device_id`] — the same row the ROM is generated
//! from — because a device that answers `10de:2504` through config space while serving a
//! ROM whose PCIR block says something else is precisely the host/guest disagreement
//! `kayfabe_abi::vbios`'s module docs exist to prevent. [`identity_for`] is the one
//! assembly point and [`ChipError::VbiosProfileMissing`] is what a chip row with no ROM row
//! gets: a named refusal, never a default.

#![doc(test(attr(deny(warnings))))]

pub mod abi;
pub mod ga10x;
pub mod guestsysinfo;
pub mod inert;
pub mod inittables;
pub mod plane;
pub mod staticinfo;
pub mod unserviced;

use kayfabe_abi::chipinfo::ChipInfoRow;
use kayfabe_abi::gspstaticinfo::FbRegion;
use kayfabe_abi::inittables::{FifoDeviceEntry, INTR_CATEGORY_COUNT, IntrTableEntry};
use kayfabe_abi::pcibars::{PciBarRow, bus_bar};
use kayfabe_abi::regaccessmap::RegisterAccessMapRow;
use kayfabe_abi::vbios::{VbiosError, VbiosWire, profile_for_device_id};
use kayfabe_arch::gsp::GspModel;

pub use plane::{
    Counters, NanoClock, ReadOutcome, RefusingRam, RegPlane, SteppingClock, WriteOutcome,
};

/// ★ The guest-RAM port, re-exported — [`RegPlane::set_ram`]'s argument type.
///
/// A shell cannot install a port whose trait it cannot name, and before stage Q5 nobody
/// had tried: the only implementation was [`RefusingRam`], which lives here. Re-exporting
/// is deliberately preferred to making every shell depend on `kayfabe-gsp` directly —
/// `set_ram` is *this* crate's seam, so its vocabulary should be reachable from *this*
/// crate, and an adapter that had to name the state-machine crate to wire memory would be
/// reaching past the port it is plugging into.
///
/// ★ [`kayfabe_gsp::BootPhase`] and [`kayfabe_gsp::GspFsm`] are re-exported for the same
/// reason one step further out: [`plane::RegPlane::phase`] and [`plane::RegPlane::gsp_state`]
/// *return* them, and a shell that cannot name a method's return type cannot bind it.
pub use kayfabe_gsp::{BootPhase, GspFsm, GuestRam, RamRefused};

/// A register whose value is a constant of the silicon.
///
/// ★ Deliberately **not** part of [`kayfabe_arch::GspModel`]: that seam's rule is *"a
/// register whose served value is a function of the GSP boot FSM's state belongs there;
/// every other register does not"*, and a chip-identity register is a function of nothing.
/// Modelling it as a `(offset, value)` pair keeps the rule enforceable by inspection —
/// anything here that needed state would have to move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootReg {
    /// Byte offset within the register aperture (base-address register 0).
    pub off: u64,
    /// The value read back, in the low 32 bits.
    pub value: u32,
    /// The register's name, for diagnostics. Never branched on.
    pub name: &'static str,
}

/// Where this generation exposes its free-running nanosecond counter, as two 32-bit halves.
///
/// ★ A chip fact and nothing more — the *value* comes from [`plane::NanoClock`], which is
/// the shell's to provide. It is on the row rather than in the register model for the same
/// reason [`BootReg`] is: its answer is a function of no boot state at all, and a row that
/// needed state would have to move behind [`kayfabe_arch::gsp::GspModel`].
///
/// ★★ It is a REQUIRED field with no default, because a generation whose counter we forgot
/// to place is a generation whose driver hangs in kernel context with nothing printed. See
/// [`plane`]'s module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtimerRegs {
    /// Byte offset of the counter's low half within base-address register 0.
    pub lo_off: u64,
    /// Byte offset of its high half.
    pub hi_off: u64,
}

/// A guest-physical window inside the register aperture that is served from a byte image
/// rather than from a register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomWindow {
    /// Byte offset of the window's first byte within base-address register 0.
    pub base: u64,
    /// The window's length in bytes.
    pub len: u64,
}

impl RomWindow {
    /// Whether `off` lies in the window.
    #[must_use]
    pub fn contains(&self, off: u64) -> bool {
        off >= self.base && off - self.base < self.len
    }
}

/// ★★★ **A framebuffer window — an aperture whose accesses are DEVICE MEMORY, not
/// registers.** Re-exported from [`kayfabe_arch`], where it moved so that the address
/// plane can name a window without depending on the device shell (`kayfabe-trace`
/// depends on `kayfabe-mmu`, and this crate depends on `kayfabe-trace`, so the arrow
/// only goes one way). The classifier that produces it, [`ChipProfile::fb_window`],
/// stays here — it is a chip fact; the *name* is shared vocabulary.
pub use kayfabe_arch::FbWindow;

/// A contiguous byte span inside base-address register 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegSpan {
    /// Byte offset of the span's first byte.
    pub base: u64,
    /// The span's length in bytes.
    pub len: u64,
}

impl RegSpan {
    /// Whether `off` lies in the span.
    #[must_use]
    pub fn contains(&self, off: u64) -> bool {
        off >= self.base && off - self.base < self.len
    }
}

/// ★★ **One GPU generation, as a row.**
///
/// Every field is either an identity the device reports through configuration space, a
/// constructor for something that is genuinely code (a register decoder), or a
/// chip-adjacent geometry. There is no behaviour here, which is the property that makes
/// "adding a generation is a table row" checkable rather than aspirational.
pub struct ChipProfile {
    /// Chip name, for diagnostics. Never branched on.
    pub name: &'static str,
    /// PCI device id. **The key** — into [`CHIPS`] and into
    /// [`kayfabe_abi::vbios::VBIOS_PROFILES`], which is what stops the two disagreeing.
    pub pci_device_id: u16,
    /// PCI revision id.
    pub pci_revision: u8,
    /// Subsystem vendor id.
    pub pci_subsystem_vendor_id: u16,
    /// Subsystem device id.
    pub pci_subsystem_id: u16,
    /// The register aperture's size in bytes (base-address register 0).
    pub regs_aperture_len: u64,
    /// Registers that are constants of the silicon.
    pub boot_regs: &'static [BootReg],
    /// Where the driver reads this generation's free-running nanosecond counter.
    pub ptimer: PtimerRegs,
    /// Where the driver reads the VBIOS through the register aperture.
    pub rom_window: RomWindow,
    /// ★★ Where this generation's `PRAMIN` framebuffer window sits in the register
    /// aperture — see [`FbWindow::Pramin`] for why a framebuffer window is not a register.
    ///
    /// On the row rather than in [`plane`] for the reason every other geometry is: the
    /// window moved between generations, and a plane that hard-codes it is a plane the
    /// second generation edits.
    pub pramin_window: RegSpan,
    /// The VBIOS parse path this generation's driver speaks.
    pub vbios_wire: VbiosWire,
    /// How many message-signalled interrupt vectors the device offers.
    ///
    /// ★ A chip fact, and a small one, but it belongs on the row for the same reason the
    /// aperture size does: a shell that hard-codes it is a shell a second generation edits.
    pub msix_vectors: u16,
    /// Build this generation's GSP register map.
    ///
    /// A constructor rather than a value because [`GspModel`] is a trait object and a
    /// `static` table cannot own one without a lifetime that outlives every reader.
    pub gsp_model: fn() -> Box<dyn GspModel>,
    /// ★★ **The engines this chip advertises to the guest's RM.**
    ///
    /// Rows, not a blob, and on the *chip* row rather than in a logic crate, because an
    /// engine list is a fact about silicon. `kayfabe_abi::inittables` owns only the wire
    /// layout; [`inittables::InitTablePolicy`] owns only the decision to answer.
    ///
    /// ⊘ An engine listed here is an engine the driver will go on to **use**. Padding this
    /// to look complete moves the failure later and deeper — see [`ga10x::GA106_ENGINES`],
    /// which names the four it leaves out.
    pub engines: &'static [FifoDeviceEntry],
    /// This chip's kernel interrupt table — the `MC_ENGINE_IDX` → vector map.
    pub intr_table: &'static [IntrTableEntry],
    /// `subtreeMap[]`, which travels with [`ChipProfile::intr_table`] because RM copies it
    /// out of the same reply and asserts on one of its entries.
    pub intr_subtree_map: [u64; INTR_CATEGORY_COUNT],
    /// ★★★ **The framebuffer regions this chip advertises — a promise, not a
    /// description.**
    ///
    /// Every byte in a region with `reserved == 0` is memory the guest's heap will hand
    /// out. On the row rather than in a logic crate for the usual reason, and with a
    /// sharper edge than the engine list has: an invented region is not answered later
    /// with `NV_ERR_NOT_SUPPORTED`, it is answered with a fault at an address that names
    /// nothing. See [`ga10x::GA106_FB_REGIONS`], which states what backs each of its two
    /// and why the oracle's other three are not here.
    pub fb_regions: &'static [FbRegion],
    /// ★★ **The base-address registers this device presents, in RM's own index order**
    /// ([`kayfabe_abi::pcibars::bus_bar`]).
    ///
    /// A row here is an aperture the hypervisor shell really decodes: [`identity_for`]
    /// hands the two window lengths to the shell, which refuses to realize if its own
    /// registration disagrees. That is what makes this table an *advertisement of what we
    /// serve* rather than a second, drifting description of the device.
    ///
    /// ⊘ Sizes only. `barOffset` is the guest's PCI enumeration's business and this port
    /// has no source for it. `[inferred]` from the two vendored trees: the only reader of
    /// a `NV2080_CTRL_BUS_GET_PCI_BAR_INFO_PARAMS` field is
    /// `ogkm-580: src/nvidia/src/kernel/gpu/bus/kern_bus.c:597`, which copies
    /// `barSizeBytes` and nothing else, and RM fills its own `pciBars[]` from
    /// `pGpu->busInfo` instead (`ogkm-580: kern_bus_gm107.c:4709-4720`). ⚠ That covers the
    /// open trees only; a closed userspace client is not greppable — see
    /// [`kayfabe_abi::pcibars`] for what would have to change if one is ever observed to
    /// care.
    pub pci_bars: &'static [PciBarRow],
    /// ★★ **The chip-identity reply's per-chip half** — the two silicon facts the PCI
    /// identity does not already state, and the register groups this chip *names*.
    ///
    /// The identity fields of `NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO` are not here on
    /// purpose: they are assembled from [`identity_for`], the same call configuration space
    /// is served from, because `_gpuInitChipInfo` **overwrites** `pGpu->idInfo` with what
    /// this reply carries (`ogkm-580: src/nvidia/src/kernel/gpu/gpu.c:891-893`) and a
    /// second, drifting statement of the device's identity is exactly what that would be.
    ///
    /// ⊘ A register group listed in [`kayfabe_abi::chipinfo::ChipInfoRow::reg_bases`] is a
    /// window the guest will **map and read**. See [`ga10x::GA106_CHIP_INFO`], which names
    /// one and states, per omission, what leaving the other three out costs.
    pub chip_info: ChipInfoRow,
    /// ★★★ **What this device tells the guest unprivileged userspace may reach with
    /// `NV2080_CTRL_CMD_GPU_EXEC_REG_OPS` — a policy, not a description.**
    ///
    /// A chip row because the reply is a bitmap with one bit per 32-bit register of *this
    /// chip's* BAR0, and because whether a chip publishes one at all is a fact about the
    /// chip (`ogkm-580: gpu_register_access_map.c:261-267` is RM's own *"unsupported for
    /// this chip"*).
    ///
    /// ⊘ A set bit is a promise that this device answers that offset meaningfully. See
    /// [`ga10x::GA106_USER_REGISTER_ACCESS_MAP`], which publishes none and says why, and
    /// [`kayfabe_abi::regaccessmap`] for the one encoding that means the opposite of it.
    pub user_register_access_map: RegisterAccessMapRow,
    /// `fb_length` — the same framebuffer, in bytes.
    ///
    /// ⚠ **The third statement of one fact.** `NV_USABLE_FB_SIZE_IN_MB` is the first and
    /// [`ChipProfile::fb_regions`]' last limit is the second; RM reads all three and
    /// believes each (`ogkm-580: mem_mgr_gsp_client.c:104-120`).
    /// `kayfabe_abi::gspstaticinfo::encode_gsp_static_info` refuses a row whose regions
    /// and `fb_length` disagree, and `tests/gsp_static_info.rs` pins it against the
    /// register.
    pub fb_length: u64,
}

impl ChipProfile {
    /// The length of one of RM's logical base-address registers, by
    /// [`kayfabe_abi::pcibars::bus_bar`] index.
    ///
    /// ★ An index this chip does not declare is **zero**, and that is the protocol's own
    /// spelling for *"this BAR is not present"* rather than a defaulted answer: RM's
    /// physical side encodes a disabled BAR1/BAR2 identically
    /// (`ogkm-580: kern_bus_gm107.c:407, 416`), and rows past `pciBarCount` are cleared to
    /// zero before the reply is sent (`ogkm-580: kern_bus_ctrl.c:654-660`).
    #[must_use]
    pub fn pci_bar_len(&self, index: usize) -> u64 {
        self.pci_bars.get(index).map_or(0, |b| b.size_bytes)
    }

    /// ★★★ **Which framebuffer window, if any, an access lands in.**
    ///
    /// `bar` is RM's own logical index ([`kayfabe_abi::pcibars::bus_bar`]), which is what
    /// the hypervisor shim already hands [`plane::RegPlane`].
    ///
    /// ★ A window this chip declares with a **zero length is not a window** — the same
    /// spelling [`ChipProfile::pci_bar_len`] uses for *"this BAR is not present"*
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:407, 416`).
    /// A chip with no framebuffer aperture must not have its accesses attributed to one.
    #[must_use]
    pub fn fb_window(&self, bar: u8, off: u64) -> Option<FbWindow> {
        let len = self.pci_bar_len(usize::from(bar));
        match usize::from(bar) {
            kayfabe_abi::pcibars::bus_bar::REGS if self.pramin_window.contains(off) => {
                Some(FbWindow::Pramin)
            }
            kayfabe_abi::pcibars::bus_bar::FB if len != 0 => Some(FbWindow::FbAperture),
            kayfabe_abi::pcibars::bus_bar::INST if len != 0 => Some(FbWindow::InstanceWindow),
            _ => None,
        }
    }
}

impl core::fmt::Debug for ChipProfile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChipProfile")
            .field("name", &self.name)
            .field(
                "pci_device_id",
                &format_args!("{:#06x}", self.pci_device_id),
            )
            .field("boot_regs", &self.boot_regs.len())
            .field("rom_window", &self.rom_window)
            .finish_non_exhaustive()
    }
}

/// The known chips.
///
/// ★ **Adding a generation is appending a row here** — plus the module its `gsp_model`
/// names, plus the [`kayfabe_abi::vbios::VBIOS_PROFILES`] row its `pci_device_id` keys.
/// No logic crate changes; `tests/chip_table.rs` proves it by building a chip that is not
/// in this table at all and driving the same plane through it.
pub static CHIPS: &[&ChipProfile] = &[&ga10x::GA106];

/// Why a chip could not be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipError {
    /// No [`ChipProfile`] in [`CHIPS`] has this PCI device id. There is deliberately no
    /// nearest-neighbour fallback: answering a driver as a chip we do not model is how a
    /// failure surfaces a thousand registers later instead of here.
    NoChipForDevice {
        /// The device id that was asked for.
        device_id: u16,
    },
    /// A chip row exists but [`kayfabe_abi::vbios::VBIOS_PROFILES`] has no row for the same
    /// device id, so the identity this device would claim has no ROM behind it.
    VbiosProfileMissing {
        /// The device id that was asked for.
        device_id: u16,
    },
    /// The ROM for this chip could not be generated.
    Vbios(VbiosError),
    /// The generated ROM does not fit the window the chip declares.
    RomTooLargeForWindow {
        /// The generated image's length.
        len: usize,
        /// The window's length.
        window: u64,
    },
    /// ★★ Two of the chip's declared read sources cover the same offset.
    ///
    /// The read path asks them in a fixed order, so an overlap would be resolved silently
    /// by that order and the loser would simply never be consulted. Refused at realize
    /// instead — see [`plane::RegPlane::new`].
    OverlappingSources {
        /// The offset both sources claim.
        off: u64,
        /// The source the read path would ask first.
        a: &'static str,
        /// The source it would therefore never reach.
        b: &'static str,
    },
    /// ★★ The chip states the register aperture's size twice — once as
    /// [`ChipProfile::regs_aperture_len`], once as row
    /// [`kayfabe_abi::pcibars::bus_bar::REGS`] of [`ChipProfile::pci_bars`] — and the two
    /// do not agree.
    ///
    /// One is what the hypervisor registers and the guest's MMU decodes; the other is what
    /// we tell the guest's RM through `NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO`. A guest told
    /// the wrong one sizes its own mappings against an aperture that is not there, and
    /// nothing logs. Refused at realize.
    BarTableDisagreesWithAperture {
        /// What the BAR table says.
        bar_len: u64,
        /// What [`ChipProfile::regs_aperture_len`] says.
        aperture: u64,
    },
    /// A declared register or window lies outside the register aperture the chip states,
    /// so the guest could never address it.
    OutsideAperture {
        /// The offending offset.
        off: u64,
        /// The aperture's length.
        aperture: u64,
    },
}

impl core::fmt::Display for ChipError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoChipForDevice { device_id } => {
                write!(
                    f,
                    "no emulated-chip profile for PCI device {device_id:#06x}"
                )
            }
            Self::VbiosProfileMissing { device_id } => write!(
                f,
                "chip {device_id:#06x} has no synthetic-VBIOS row; its identity would have \
                 no ROM behind it"
            ),
            Self::Vbios(e) => write!(f, "the synthetic VBIOS could not be built: {e}"),
            Self::RomTooLargeForWindow { len, window } => write!(
                f,
                "the generated ROM is {len} bytes and the chip's ROM window is {window}"
            ),
            Self::OverlappingSources { off, a, b } => write!(
                f,
                "offset {off:#x} is claimed by both {a} and {b}; the read path asks {a} \
                 first, so {b} would never be reached there"
            ),
            Self::OutsideAperture { off, aperture } => write!(
                f,
                "offset {off:#x} lies outside the {aperture:#x}-byte register aperture"
            ),
            Self::BarTableDisagreesWithAperture { bar_len, aperture } => write!(
                f,
                "the chip's BAR table says the register aperture is {bar_len:#x} bytes and \
                 its regs_aperture_len says {aperture:#x}; the guest would be told one and \
                 decode the other"
            ),
        }
    }
}

impl core::error::Error for ChipError {}

/// Look a chip up by PCI device id.
///
/// # Errors
///
/// [`ChipError::NoChipForDevice`] if no row matches.
pub fn chip_for_device_id(device_id: u16) -> Result<&'static ChipProfile, ChipError> {
    CHIPS
        .iter()
        .copied()
        .find(|c| c.pci_device_id == device_id)
        .ok_or(ChipError::NoChipForDevice { device_id })
}

/// The default chip a shell realizes when its operator names none.
///
/// ★ Named here rather than in a shell, so that "which chip does the device claim to be?"
/// has one answer in one place. A shell that wants a different one asks by device id.
#[must_use]
pub fn default_chip() -> &'static ChipProfile {
    CHIPS[0]
}

/// What a hypervisor shell must put in configuration space before a stock driver will bind.
///
/// ★★ **Not a lie, and this is the whole argument for it.** `nv_pci_table`
/// (`ogkm-580: kernel-open/nvidia/nv-pci-table.c:39`) matches vendor `0x10DE` with class
/// `0300xx`/`0302xx` and the module unloads itself when nothing matches, so there is no
/// force-bind fallback to fall back to. Presenting a neutral identity is not a safer
/// version of this device; it is a device no NVIDIA driver can ever reach. We *are*
/// emulating an NVIDIA GPU, so saying so is the accurate statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceIdentity {
    /// PCI vendor id — from the VBIOS row, not from the chip row.
    pub vendor_id: u16,
    /// PCI device id.
    pub device_id: u16,
    /// PCI revision id.
    pub revision: u8,
    /// PCI class code as a 24-bit value, `(base << 16) | (sub << 8) | prog_if`.
    pub class_code: u32,
    /// Subsystem vendor id.
    pub subsystem_vendor_id: u16,
    /// Subsystem device id.
    pub subsystem_id: u16,
    /// How many message-signalled vectors to offer.
    pub msix_vectors: u16,
    /// The framebuffer window's length, from [`ChipProfile::pci_bars`].
    ///
    /// ★ Here rather than left to the shell's own property because the *same* number is
    /// what [`inittables::InitTablePolicy`] tells the guest's RM through
    /// `NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO`. A shell that registers a different aperture
    /// than the one we advertise is the two-descriptions-of-one-range failure, and the
    /// only moment an operator can act on it is realize.
    pub fb_window_len: u64,
    /// The instance/`BAR2` window's length, likewise.
    pub inst_window_len: u64,
}

/// Assemble the identity a chip's device reports.
///
/// # Errors
///
/// [`ChipError::VbiosProfileMissing`] if the chip's device id has no
/// [`kayfabe_abi::vbios::VbiosProfile`]. This is a refusal rather than a default because
/// the vendor id and the class code are read *out of the ROM row on purpose* — a default
/// here would put the two back in a position to disagree.
///
/// [`ChipError::BarTableDisagreesWithAperture`] if the chip's own two statements of the
/// register aperture's size are not the same statement.
pub fn identity_for(chip: &ChipProfile) -> Result<DeviceIdentity, ChipError> {
    let regs = chip.pci_bar_len(bus_bar::REGS);
    if regs != chip.regs_aperture_len {
        return Err(ChipError::BarTableDisagreesWithAperture {
            bar_len: regs,
            aperture: chip.regs_aperture_len,
        });
    }
    let p =
        profile_for_device_id(chip.pci_device_id).map_err(|_| ChipError::VbiosProfileMissing {
            device_id: chip.pci_device_id,
        })?;
    // The PCIR structure stores the class code low byte first; configuration space wants it
    // as one 24-bit value. One conversion, in one place.
    let class_code = u32::from(p.pci_class_code[2]) << 16
        | u32::from(p.pci_class_code[1]) << 8
        | u32::from(p.pci_class_code[0]);
    Ok(DeviceIdentity {
        vendor_id: p.pci_vendor_id,
        device_id: p.pci_device_id,
        revision: chip.pci_revision,
        class_code,
        subsystem_vendor_id: chip.pci_subsystem_vendor_id,
        subsystem_id: chip.pci_subsystem_id,
        msix_vectors: chip.msix_vectors,
        fb_window_len: chip.pci_bar_len(bus_bar::FB),
        inst_window_len: chip.pci_bar_len(bus_bar::INST),
    })
}

/// Build the ROM image this chip's device serves through its [`ChipProfile::rom_window`].
///
/// # Errors
///
/// [`ChipError::VbiosProfileMissing`], [`ChipError::Vbios`] or
/// [`ChipError::RomTooLargeForWindow`]. ★ The last one is checked *here* rather than at the
/// window's read path: a ROM that does not fit is a configuration mistake an operator can
/// see at realize, and discovering it as a truncated parse inside the guest is the shape of
/// bug that costs a bench cycle to attribute.
pub fn rom_for(chip: &ChipProfile) -> Result<Vec<u8>, ChipError> {
    let p =
        profile_for_device_id(chip.pci_device_id).map_err(|_| ChipError::VbiosProfileMissing {
            device_id: chip.pci_device_id,
        })?;
    let image = kayfabe_abi::vbios::build(p, chip.vbios_wire).map_err(ChipError::Vbios)?;
    if image.len() as u64 > chip.rom_window.len {
        return Err(ChipError::RomTooLargeForWindow {
            len: image.len(),
            window: chip.rom_window.len,
        });
    }
    Ok(image)
}

/// ★★ **The one command-policy chain this device answers with** — built here rather than
/// inside [`plane::RegPlane::new`] so that the *differential* drives the same chain a
/// guest does.
///
/// The chain used to be an expression inside `RegPlane::new`, which meant the 359 062-record
/// `cap1`/`cap1b` replay could only ever exercise `kayfabe_gsp::EchoOk` — the C baseline —
/// and **not one** of the served controls. Three of the four [`inittables::InitTablePolicy`]
/// replies therefore had no reply-plane coverage at all, which is why a defect in
/// [`staticinfo::StaticInfoPolicy`] could sit unnoticed. One chain, two consumers: change
/// an order or a link here and the replay changes with the device.
///
/// ★ Order is precedence and the three answering links are disjoint by function code:
/// [`inittables::InitTablePolicy`] claims only `GSP_RM_CONTROL`,
/// [`staticinfo::StaticInfoPolicy`] only `GET_GSP_STATIC_INFO`,
/// [`guestsysinfo::GuestSystemInfoPolicy`] only fn 1 and fn 64.
/// `crates/kayfabe-device/tests/gsp_static_info.rs` pins the disjointness rather than
/// trusting the reading.
///
/// ★ The LEDGER is last, and it answers nothing — it writes down exactly what the links
/// above declined and lets the FSM refuse it by name. See [`unserviced`] for why a
/// host-side list is the only way to know how long that list is.
#[must_use]
pub fn served_policy(
    chip: &'static ChipProfile,
    driver: kayfabe_abi::versions::DriverAbiTable,
    unserviced: unserviced::UnservicedLog,
) -> Box<dyn kayfabe_gsp::CommandPolicy> {
    Box::new(kayfabe_gsp::PolicyChain::new(vec![
        Box::new(inittables::InitTablePolicy::new(chip, driver)),
        Box::new(staticinfo::StaticInfoPolicy::new(chip, driver)),
        Box::new(guestsysinfo::GuestSystemInfoPolicy::new(driver)),
        Box::new(inert::InertPolicy::new()),
        Box::new(unserviced::UnservicedLedger::new(driver, unserviced)),
    ]))
}
