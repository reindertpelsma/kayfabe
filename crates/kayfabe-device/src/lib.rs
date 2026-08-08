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
pub mod bar2;
pub mod census;
pub mod ceresolve;
pub mod cpuintr;
pub mod doorbell;
pub mod faultbuffer;
pub mod fbwin;
pub mod ga10x;
pub mod guestsysinfo;
pub mod gvaspub;
pub mod inert;
pub mod inittables;
pub mod plane;
pub mod staticinfo;
pub mod sticky;
pub mod sweep;
pub mod unserviced;

use kayfabe_abi::bifstatic::BifStaticRow;
use kayfabe_abi::chipinfo::ChipInfoRow;
use kayfabe_abi::confcompute::ConfComputeRow;
use kayfabe_abi::deviceinfo::DeviceInfoRow;
use kayfabe_abi::falconinfo::FalconInventoryRow;
use kayfabe_abi::fifochannels::FifoChannelsRow;
use kayfabe_abi::gmmustatic::GmmuStaticRow;
use kayfabe_abi::grinfo::GrInfoProfile;
use kayfabe_abi::grstatic::{CONTEXT_BUFFER_ID_COUNT, ContextBuffer, GrStaticProfile};
use kayfabe_abi::gspstaticinfo::FbRegion;
use kayfabe_abi::inittables::{FifoDeviceEntry, INTR_CATEGORY_COUNT, IntrTableEntry};
use kayfabe_abi::memsysconfig::MemorySystemRow;
use kayfabe_abi::pcibars::{PciBarRow, bus_bar};
use kayfabe_abi::regaccessmap::RegisterAccessMapRow;
use kayfabe_abi::vbios::{VbiosError, VbiosWire, profile_for_device_id};
use kayfabe_arch::gsp::GspModel;

pub use plane::{
    Counters, DoorbellLog, NanoClock, PlaneResidue, ReadOutcome, RefusingRam, RegPlane,
    SteppingClock, WriteOutcome,
};

/// ★ The fault vocabulary a [`DoorbellReport`] speaks, re-exported.
///
/// [`DoorbellRefused::kind`] is a [`FaultTag`], and the composition root that fills one in
/// needs [`Faulted`] in scope to ask a fault for its own. Same argument as [`FbStore`]'s
/// re-export below: this is *this* crate's seam, and a shell plugging into it should not
/// have to name a further crate to spell the type its answer already carries.
pub use kayfabe_trace::{FaultTag, Faulted};

/// ★★★ The framebuffer port and the BAR0 moving window's register, re-exported —
/// [`RegPlane::set_fb`]'s argument type and the vocabulary a shell needs to install it.
///
/// Same argument as [`GuestRam`]'s re-export one paragraph down: `set_fb` is *this* crate's
/// seam, so a shell plugging into it should not have to name a third crate to do so.
pub use fbwin::{Bar0Window, FbRefused, FbStore, RefusingFb, SparseFb};

/// ★★★ **E2** — the usermode doorbell port, re-exported: [`RegPlane::set_doorbell`]'s
/// argument type and the vocabulary a shell needs to read its answer.
///
/// Same argument as [`FbStore`]'s re-export above. `set_doorbell` is *this* crate's seam,
/// and the one composition root that installs it already holds four other crates; it
/// should not have to name a fifth to spell the trait.
pub use doorbell::{
    DoorbellPort, DoorbellRefused, DoorbellReport, NO_DOORBELL_PORT, NO_DOORBELL_PORT_KIND,
    RefusingDoorbell, USERMODE_DOORBELL_OFF, doorbell_reg,
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
///
/// ★ `Copy` because every field already is — refs, arrays, a fn pointer and nested `Copy`
/// rows — and because the alternative is what the test tree had grown: hand-written
/// `copy_of_ga106()` literals restating the whole row, which is precisely the "second,
/// drifting description" this type's own field docs argue against.
#[derive(Clone, Copy)]
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
    /// ★★★ **Where `NV_PBUS_BAR0_WINDOW` — the register that POSITIONS
    /// [`ChipProfile::pramin_window`] — sits in the register aperture.**
    ///
    /// On the row for the same reason the window it moves is, and with a sharper edge: the
    /// two are one mechanism, and a port that had the aperture without the register would
    /// serve a *fixed* view of framebuffer address zero while the guest believed it had
    /// moved the window. See [`crate::fbwin::Bar0Window`] for the field layout and for why
    /// the register must be a real read-write latch rather than a decoded write sink.
    ///
    /// ⊘ **Zero means this chip has no BAR0 moving window**, the same spelling
    /// [`ChipProfile::pci_bar_len`] and [`ChipProfile::fb_window`] already use for *"this
    /// aperture is not present"*. A chip declaring a `pramin_window` and no register to
    /// move it is refused at realize ([`ChipError::WindowWithoutItsRegister`]) — the two
    /// halves are useless apart and a row that carried only one is a row somebody stopped
    /// editing halfway.
    pub bar0_window_reg: u64,
    /// The VBIOS parse path this generation's driver speaks.
    pub vbios_wire: VbiosWire,
    /// How many message-signalled interrupt vectors the device offers.
    ///
    /// ★ A chip fact, and a small one, but it belongs on the row for the same reason the
    /// aperture size does: a shell that hard-codes it is a shell a second generation edits.
    pub msix_vectors: u16,
    /// ★★★ **The copy-engine fault method buffer's size in bytes — the first field on this
    /// row whose value was measured on real silicon rather than read out of a tree.**
    ///
    /// A chip row, and emphatically not a constant, for a reason the module that encodes it
    /// spells out at length ([`kayfabe_abi::fmbsize`]): the number is produced inside GSP
    /// firmware, appears in no open source, and is refused to every usermode client — so the
    /// only way to hold it is to have asked a part of *this* generation. GA106 was asked;
    /// AD10x and GH100 were not. A shared constant would silently answer them with GA106's
    /// number, which is the flattering direction.
    ///
    /// ⊘ **Zero means the chip has not stated one**, and a chip row that reaches realize with
    /// zero is refused ([`ChipError::NoFaultMethodBufferSize`]) rather than served — because
    /// serving zero rebuilds the exact `RmInitAdapter failed! (0x25:0x1f:1249)` this field
    /// removes (`ogkm-580: mem_desc.c:239-241`), while presenting as an answered control.
    pub ce_fault_method_buffer_size: u32,
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
    /// ★★★ **Which engines this device tells the guest to construct falcons for.**
    ///
    /// A chip row because the falcon inventory is a property of silicon — and, on this
    /// port, of how much of that silicon's register map [`plane`] actually decodes. Every
    /// entry is an *instruction to construct*: RM turns each into a `GenericKernelFalcon`
    /// whose `registerBase` becomes a raw BAR0 base for register reads and writes it never
    /// validates (`ogkm-580: kernel_falcon_tu102.c:45-76`).
    ///
    /// ⊘ Naming an engine whose register window this device does not model would hand RM a
    /// microcontroller reporting a defaulted zero. See
    /// [`ga10x::GA106_CONSTRUCTED_FALCONS`], which names none and says what that costs,
    /// and [`kayfabe_abi::falconinfo`] for the count RM's own bounds check lets through.
    pub constructed_falcons: FalconInventoryRow,
    /// ★★★ **What this chip says about its memory system — the first row whose *refusal*
    /// is not survivable.**
    ///
    /// A chip row because L2 geometry, comptag page size and RAM type are facts about
    /// silicon. It is called out from the others because of *where* the guest asks for it:
    /// `KernelMemorySystem`'s `StatePreInit`, inside `gpuStatePreInit_IMPL`'s engine sweep,
    /// which reads `NV_ERR_NOT_SUPPORTED` as *"this engine is absent — delete it"* and
    /// carries on (`ogkm-580: gpu.c:2170-2214`). RM then dereferences the pointer it just
    /// NULLed, in a different subsystem, thousands of lines later.
    ///
    /// ⊘ So this is not a row that can be omitted and refused like
    /// [`ChipProfile::constructed_falcons`] can. `[measured]` run `t134a`, a stock
    /// 580.159.04 guest at `1c79474`: leaving it unserved is a guest-kernel `NULL`
    /// dereference in
    /// `memmgrGetBlackListPagesForHeap_GM107`, and `nvidia-smi` hangs rather than fails.
    /// See [`kayfabe_abi::memsysconfig`] for the full chain and for why an all-zero answer
    /// is a *different* crash rather than a safe default.
    pub memory_system: MemorySystemRow,
    /// ★★★ **Where each advertised engine's own PRI register block starts** — the one fact
    /// the DEVICE_INFO2 reply needs that [`ChipProfile::engines`] does not already carry.
    ///
    /// ⚠ **Deliberately not a second engine table.** Every other field of
    /// `NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE` (`0x20800a40`) is *projected* from
    /// [`ChipProfile::engines`] by [`kayfabe_abi::deviceinfo`], because that slice already
    /// states this silicon for the FIFO device-info control (`0x20801112`). Two
    /// independently written descriptions of one chip is the drift this row exists to
    /// avoid, so it holds `devicePriBase` and the not-a-device marking and nothing else —
    /// and an engine it says nothing about is a refusal
    /// ([`kayfabe_abi::deviceinfo::DeviceInfoError::NoPriBaseForEngine`]), never a silent
    /// omission.
    ///
    /// ⊘ Like [`ChipProfile::memory_system`] this is not a row that can be left out and
    /// refused. `[measured]` run `t135a`, a stock 580.159.04 guest at `c84ef52`:
    /// `gpuConstructDeviceInfoTable_HAL @ kernel_fifo.c:2208` was the guest's **first and
    /// only** `LEVEL_ERROR`, twenty times, followed by `kfifoConstructEngineList_HAL` and
    /// then a guest-kernel `NULL` dereference in `memmgrCalcReservedFbSpaceHal_GM107`
    /// (`CR2=0x201`) — the heap sizing a reservation from a `KernelFifo` with no engine
    /// list. See [`crate::sweep`] for why this refusal is worse than a refusal inside
    /// `gpuPreInit`.
    pub device_info: DeviceInfoRow,
    /// ★★ **What this device says about its confidential-compute plane** — two `NvBool`s,
    /// and the only thing standing between CPU-RM and a BAR1 mapping of protected vidmem
    /// (`ogkm-580: src/nvidia/src/kernel/rmapi/mapping_cpu.c:227-235`).
    ///
    /// ⊘ A chip row rather than a constant because *whether a part has a CC plane at all*
    /// is a fact about silicon. `kayfabe_abi::confcompute` makes the widening direction —
    /// claiming trust this port cannot back — unencodable; it cannot make the fail-open
    /// direction unencodable, because here the all-zero answer is the **right** one. See
    /// that module for why refusing and serving the truth are byte-identical to the guest.
    pub conf_compute: ConfComputeRow,
    /// ★★ **What this device says about its bus interface** — four `NvBool`s that become
    /// four `KernelBif` PDB properties.
    ///
    /// ⊘ Two of them point RM at hardware: `bIsC2CLinkUp` at a coherent chip-to-chip
    /// mapping of framebuffer, and `bIsDeviceMultiFunction` at configuration space for a
    /// PCI function 1. `kayfabe_abi::bifstatic` makes both unencodable for a device that
    /// presents neither.
    pub bif_static: BifStaticRow,
    /// ★★★ **How many channel ids this chip's FIFO has per runlist** — the first control in
    /// the sweep whose refusal **halts** the boot instead of amputating something.
    ///
    /// `kfifoRunlistQueryNumChannels_KERNEL` returns zero on any failure
    /// (`ogkm-580: kernel_fifo.c:1330-1336`) and `kfifoChidMgrConstruct` turns that zero
    /// into `NV_ERR_INVALID_STATE` (`:300-308`) — which `gpuStateInit_IMPL` does **not**
    /// map to `NV_OK`, so the boot aborts at a named statement rather than continuing with
    /// an amputated engine. ⊘ That makes refusing safe and makes serving the thing that
    /// buys the next boot any reach at all: every engine after `KernelFifo` in
    /// `gpuChildOrderList_GM200` is unreachable while this answers zero.
    pub fifo_channels: FifoChannelsRow,
    /// ★★★ **The two MMU fault buffers' sizes** — and the row whose absence is a
    /// guest-kernel **use-after-free** rather than an amputation.
    ///
    /// `_kgmmuInitStaticInfo` allocates `pKernelGmmu->pStaticInfo`, and on a failed control
    /// `portMemFree`s it **without NULLing the field**
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:139-166`).
    /// `gpuStateInit_IMPL` then maps the `NV_ERR_NOT_SUPPORTED` to `NV_OK` and carries on
    /// (`ogkm-580: gpu.c:2286-2287`), leaving `KernelGmmu` alive with a dangling pointer that
    /// every `NULL` check passes and that guest-reachable control handlers read
    /// (`mmu_fault_buffer_ctrl.c:84, 176`).
    ///
    /// ⊘ `[inferred]` from source, not from a boot — see [`kayfabe_abi::gmmustatic`], which
    /// states the boundary rather than eliding it.
    pub gmmu_static: GmmuStaticRow,
    /// ★★★ **This chip's GR geometry** — GPCs, TPCs, the global SM order, the FECS record
    /// size and the per-subcontext-header property, as one value.
    ///
    /// One field rather than five, because RM reads all five replies and **cross-checks
    /// them**: `numTpc` comes out of the SM order and `tpcCount` out of the floorsweeping
    /// masks, and a row that could state them separately is a row that could state them
    /// differently. [`kayfabe_abi::grstatic::GrStaticProfile::validate`] is what makes the
    /// disagreement unencodable rather than merely unlikely.
    ///
    /// ⚠ Its `gpcMask` is the field a shortcut would have zeroed to skip the golden-image
    /// channel entirely (`ogkm-580: kernel_graphics.c:486`). See
    /// [`kayfabe_abi::grstatic`]'s header for why this port does not.
    pub gr_static: GrStaticProfile,
    /// ★★★ GR's **legacy info list** — the 58 `(index, data)` pairs `0x20800a2a` answers,
    /// and the row whose absence ended run `fmb1` with `RmInitAdapter failed!
    /// (0x25:0x40:1249)`.
    ///
    /// ⊘ A separate field from [`ChipProfile::gr_static`] even though six of its 58 entries
    /// restate that geometry: the other 52 are **litter constants** — crossbar port counts,
    /// LTC slice counts — that are not derivable from anything this port models. What ties
    /// the two together is [`kayfabe_abi::grinfo::GrInfoProfile::validate_against`], which
    /// refuses a pair that disagrees about the six they share, rather than a fold that would
    /// have implied a relationship for all 58.
    pub gr_info: GrInfoProfile,
    /// ★ This chip's twenty-six context-buffer sizes and alignments, indexed by
    /// `NV0080_CTRL_FIFO_GET_ENGINE_CONTEXT_PROPERTIES_ENGINE_ID_*`.
    ///
    /// ⊘ A **separate** field from [`ChipProfile::gr_static`] on purpose: the other five GR
    /// replies are views of one geometry that RM cross-checks, and these are not derivable
    /// from it. Folding them in would have implied a relationship this port has not
    /// established. See [`kayfabe_abi::grstatic::CONTEXT_BUFFER_ABSENT`] — `NV_U32_MAX`
    /// means *absent*, and two of GA106's rows are genuinely `0`.
    pub gr_context_buffers: [ContextBuffer; CONTEXT_BUFFER_ID_COUNT],
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
    /// ★★★ The chip row states no copy-engine fault method buffer size.
    ///
    /// See [`ChipProfile::ce_fault_method_buffer_size`]. Refused at realize because the
    /// alternative — serving the control with a zero — is not a weaker answer but the
    /// original bug wearing an answer's clothes: `memdescCreate` turns a zero length into
    /// `NV_ERR_INVALID_ARGUMENT` (`ogkm-580: mem_desc.c:239-241`) and the guest's
    /// `RmInitAdapter` fails `0x25:0x1f:1249` with nothing naming this row.
    NoFaultMethodBufferSize {
        /// The chip that has not stated one.
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
    /// ★★★ The chip declares a `PRAMIN` window and **no `NV_PBUS_BAR0_WINDOW` register to
    /// move it**, or the register and no window.
    ///
    /// The two halves are one mechanism: the register selects which framebuffer page the
    /// aperture shows. A row with only the aperture would serve a *fixed* view of
    /// framebuffer address zero while the guest believed it had moved the window — every
    /// access mis-addressed, nothing logged, and the first symptom hundreds of operations
    /// later. A row with only the register would latch a value nothing reads. Refused at
    /// realize, because a half-filled row is what an interrupted edit leaves behind.
    WindowWithoutItsRegister {
        /// The `PRAMIN` window's length, `0` if the row declares none.
        window_len: u64,
        /// The `NV_PBUS_BAR0_WINDOW` offset, `0` if the row declares none.
        reg_off: u64,
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
            Self::NoFaultMethodBufferSize { device_id } => write!(
                f,
                "chip {device_id:#06x} states no copy-engine fault method buffer size; \
                 serving zero is not a weaker answer, it is RmInitAdapter 0x25:0x1f:1249"
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
            Self::WindowWithoutItsRegister {
                window_len,
                reg_off,
            } => write!(
                f,
                "this chip declares a PRAMIN window of {window_len:#x} bytes and a \
                 NV_PBUS_BAR0_WINDOW register at {reg_off:#x}; exactly one of them is zero, \
                 and the two are one mechanism — an aperture nothing can move shows \
                 framebuffer address zero forever, and says nothing while it does"
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
///
/// [`ChipError::NoFaultMethodBufferSize`] if the chip states no
/// [`ChipProfile::ce_fault_method_buffer_size`]. ★ Checked *here*, at the same assembly
/// point as the ROM row, and for the same reason: a chip that cannot answer a control the
/// guest's state load requires is a configuration mistake an operator can see at realize,
/// and discovering it as `0x25:0x1f:1249` inside a guest costs a bench cycle to attribute.
pub fn identity_for(chip: &ChipProfile) -> Result<DeviceIdentity, ChipError> {
    if chip.ce_fault_method_buffer_size == 0 {
        return Err(ChipError::NoFaultMethodBufferSize {
            device_id: chip.pci_device_id,
        });
    }
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

/// ★★★ **The observation latches a served chain writes into**, as one value.
///
/// ⊘ A struct rather than four more parameters, and the reason is not tidiness: every one
/// of these is a log that the **plane also holds**, so the composition root must hand the
/// *same* handle to both. Four positional `Log::new()`s at a call site is exactly the shape
/// that silently acquires a fifth built fresh in the wrong place — the aperture would go on
/// being published, nothing would receive it, and every later access would refuse with a
/// diagnosis pointing at the guest ([`plane::RegPlane::bar_pde_log`] argues the same case
/// for the same reason). Named fields make that a compile error at the construction site.
///
/// ★ It is `Clone` because every field is a shared handle (`Arc` inside): cloning it hands
/// out the same latches, never copies of them.
#[derive(Debug, Clone, Default)]
pub struct ChainLogs {
    /// The commands **nothing answered** ([`unserviced`]).
    pub unserviced: unserviced::UnservicedLog,
    /// The replayable-fault-buffer registrations this port declines ([`faultbuffer`]).
    pub fault_buffer: faultbuffer::FaultBufferLog,
    /// The bus apertures' published root page-directory entries ([`bar2`]).
    pub bar_pdes: bar2::BarPdeLog,
    /// The VA spaces' published page-directory levels ([`gvaspub`]).
    pub gvas_pub: gvaspub::GvasPubLog,
}

/// ★★ **The one command-policy chain this device answers with** — built here rather than
/// inside [`plane::RegPlane::new`] so that the *differential* drives the same chain a
/// guest does.
///
/// The chain used to be an expression inside `RegPlane::new`, which meant the 359 062-record
/// `cap1`/`cap1b` replay could only ever exercise `kayfabe_gsp::EchoOk` — the C baseline —
/// and **not one** of the served controls. Four of the five [`inittables::InitTablePolicy`]
/// replies that existed then had no reply-plane coverage at all, which is why a defect in
/// [`staticinfo::StaticInfoPolicy`] could sit unnoticed. It is **six of six** now, and the
/// coverage assertion derives its universe from `WantedTable::ALL` rather than restating a
/// number here. One chain, two consumers: change an order or a link here and the replay
/// changes with the device.
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
///
/// ## ★★★ The chain is WRAPPED, and the wrapper is what makes the sticky rule a property
///
/// [`sticky::StickyAnswerGuard`] sits outside the whole chain rather than inside any link.
/// Every accepted `GSP_RM_CONTROL` reply — from a link that exists today, from one added
/// tomorrow, from a link that never heard of the rule — crosses it, and it writes the two
/// reply fields that decide whether the guest keeps our answer **forever** back to zero
/// (`rmapiControlCacheSetUnchecked`, `ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11098-11103`).
/// So *"installed in this port"* and *"cannot hand out a sticky answer"* are **one fact**,
/// not two lists that agree today; [`sticky`]'s module docs carry the derivation, including
/// the branch this cannot reach and why that one is tolerable.
///
/// ## ★★★ And the guard is itself wrapped by the CENSUS, which answers nothing
///
/// [`census::ControlCensus`] is the outermost layer, outside even the sticky guard, so the
/// `rpc_result` it records is the one the guest reads. It exists because the unserviced
/// ledger's list was twice misread as a map of the boot: a **served-but-refused** control
/// (`refuse()` returns `Some(Reply)`) never reaches the ledger, and a **served** control is
/// also absent — so "id absent" discriminated nothing. The census records the two positive
/// states so absence finally means *never seen*. See [`census`]'s module docs; it returns
/// the inner reply unchanged, byte for byte, and `tests/control_census.rs` pins that.
#[must_use]
pub fn served_policy(
    chip: &'static ChipProfile,
    driver: kayfabe_abi::versions::DriverAbiTable,
    logs: ChainLogs,
    census: census::ControlCensusLog,
    objects: Option<Box<dyn kayfabe_gsp::CommandPolicy>>,
) -> Box<dyn kayfabe_gsp::CommandPolicy> {
    // ★ The notifier PROBE SET comes out of the census log — deliberately, and not as a
    // convenience: the census is what the end-of-run report prints, so reading the set
    // from it makes "the set the report states" and "the set the event-plane arm
    // consults" ONE stored value rather than two values a construction site promises to
    // keep equal. A caller that never called `ControlCensusLog::set_probe_arm` gets the
    // empty set — the shipping configuration, and the safe direction.
    let probe_arm = census.snapshot().probe_arm;
    Box::new(census::ControlCensus::new(
        driver,
        census,
        sticky::StickyAnswerGuard::new(
            driver,
            served_chain(chip, driver, logs, probe_arm, objects),
        ),
    ))
}

/// The chain [`served_policy`] wraps — exposed so a test can drive the links **without**
/// the guard and see the difference the guard makes.
///
/// ⊘ Nothing in the port calls this. A caller that wants the port's behaviour wants
/// [`served_policy`]; this exists because *"the guard changes the bytes"* is only
/// assertable against the unguarded bytes, and a test that could not build them would be
/// asserting the guard against itself.
#[must_use]
pub fn served_chain(
    chip: &'static ChipProfile,
    driver: kayfabe_abi::versions::DriverAbiTable,
    logs: ChainLogs,
    probe_arm: kayfabe_abi::eventnotify::ProbeArmSet,
    objects: Option<Box<dyn kayfabe_gsp::CommandPolicy>>,
) -> Box<dyn kayfabe_gsp::CommandPolicy> {
    // ★★★ EXHAUSTIVE, and the missing `..` is load-bearing for [`Shim::audit`]'s reason
    // one crate over: a latch added to `ChainLogs` and not seated in a link below is a fact
    // the report can never carry, and nothing would go red. `rustc` refuses the pattern
    // instead.
    let ChainLogs {
        unserviced,
        fault_buffer,
        bar_pdes,
        gvas_pub,
    } = logs;
    let mut links: Vec<Box<dyn kayfabe_gsp::CommandPolicy>> = vec![
        // ★★★ FIRST, and the position is the whole of it — see `gvaspub`'s module docs.
        // This link records the guest's page-directory publication (`0x90f10106` /
        // `0x20800a9f`) and **declines**, always. `InitTablePolicy` below TERMINATES the
        // chain for those two ids — it decodes, re-encodes and returns `Some` — so a
        // recorder seated with the other two recorders at the tail could never see one.
        //
        // ⊘ The alternative was re-routing who ANSWERS, and it is forbidden:
        // `tests/gvaspace_pdes.rs` and `tests/cap1b_differential.rs` pin
        // `InitTablePolicy` as the answerer, and this link must not change a reply byte.
        // Seating an always-`None` observer ahead of it is what keeps those two facts
        // compatible (`execution_plane_increments.md` §14.8).
        Box::new(gvaspub::GvasPubRecorder::new(driver, gvas_pub)),
        // ★ The probe set rides into exactly one link: the event-plane arm. It is empty
        // unless the `probe-arm-notifier` device property named indices, and the set in
        // effect is recorded in the census so the boot's own report proves what it ran
        // with (`RegPlane::with_objects` records the SAME value it passes here).
        Box::new(inittables::InitTablePolicy::with_probe_arm(
            chip, driver, probe_arm,
        )),
        Box::new(staticinfo::StaticInfoPolicy::new(chip, driver)),
        Box::new(guestsysinfo::GuestSystemInfoPolicy::new(driver)),
        // ★★★ `#149`. Position: among the ANSWERING links, before the recorders, and
        // before `InertPolicy` would have a chance to grow an arm for it. It answers
        // exactly one function and declines everything else, so it can sit anywhere among
        // them; it is written here, beside the other links that latch a guest fact, so a
        // reader meets "the guest publishes its bus-aperture root" next to "the guest
        // publishes its system info".
        Box::new(bar2::BarPdePolicy::new(driver, bar_pdes)),
        Box::new(inert::InertPolicy::new()),
    ];
    // ★★★ The object-model link, if the composition root has one. Its POSITION is the
    // decision, and it is between the answering links and the recorders:
    //
    // - **After** them, so a link that already answers a function keeps answering it. The
    //   four above are disjoint from `kayfabe_rmrpc::OBJECT_VERBS` today, so this changes
    //   no byte; it is written this way so that the day they overlap, the *specific*
    //   answer wins over the general one rather than by accident of vec order.
    // - **Before** the recorders, because they are terminal-shaped (`respond` never
    //   answers) and anything below them is unreachable.
    //
    // ⚠ And the link must not claim more than it serves. `kayfabe_rmrpc::GraphPolicy`
    // answers EVERY command, so installing that one here would silence
    // `unserviced::UnservicedLedger` permanently — the port would lose the only instrument
    // that can say what it has not built. `kayfabe_rmrpc::ObjectPolicy` is the composable
    // form and declines by default; this crate cannot name either type (it has no
    // `kayfabe-core` dependency, deliberately), so the obligation is stated here and
    // `tests/served_chain_objects.rs` is where it is checked.
    links.extend(objects);
    links.extend::<Vec<Box<dyn kayfabe_gsp::CommandPolicy>>>(vec![
        // ★ Two recorders, both terminal-shaped (`respond` never answers), so precedence
        // between them is irrelevant and the pair cannot change what the guest sees. The
        // fault-buffer recorder is FIRST only so a reader meets the specific one before
        // the catch-all; see `faultbuffer`'s docs for why it declines on purpose.
        Box::new(faultbuffer::FaultBufferRecorder::new(driver, fault_buffer)),
        Box::new(unserviced::UnservicedLedger::new(driver, unserviced)),
    ]);
    Box::new(kayfabe_gsp::PolicyChain::new(links))
}
