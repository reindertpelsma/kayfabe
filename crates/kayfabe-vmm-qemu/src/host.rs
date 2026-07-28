//! ★★ `QemuHost` — the seam between the adapter's logic and the hypervisor's C API.
//!
//! `l2_qemu_adapter.md` §2.2's three-crate split, expressed as one trait:
//!
//! ```text
//!   hw/misc/nvkvm/  (C, QOM only)  ──▶  kayfabe-qemu-raw  ──▶  impl QemuHost
//!                                                                    ▲
//!                                            kayfabe-vmm-qemu ────────┘  (all the logic)
//! ```
//!
//! **The trait is defined by the CONSUMER**, in the safe crate, so the FFI crate has no
//! say in the shape of the port — the same direction `kayfabe_vmm::Vmm` points, one layer
//! down. It is also what makes stage **Q1** buildable with no QEMU at all: the mock in
//! [`crate::mock_host`] is the other implementor, and every test in this crate runs
//! against it.
//!
//! ## ★★ The three normative requirements on any implementor
//!
//! These are correctness, not style. Each is stated on the method it constrains, and
//! collected here because an implementor reads the trait before it reads the methods.
//!
//! 1. **[`QemuHost::read_region`] / [`QemuHost::write_region`] are a bounded memcpy
//!    against the named region's own backing.** They MUST NOT reach a general
//!    read/write-anywhere accessor, which takes the VMM's global lock whenever the target
//!    is not direct-access ([`kayfabe_vmm::VmmError::NonRamGpa`]'s rustdoc carries the
//!    citation). This is the whole reason [`kayfabe_vmm::GuestRamMap`] exists, and it is
//!    invisible to every gate in the tree, so it is written on the method an implementor
//!    would otherwise write the easy way.
//! 2. **Every topology-transaction method runs only from realize/unrealize** — the
//!    contexts the hypervisor already entered holding its global lock (§4.3: *"the adapter
//!    contains ZERO calls to `bql_lock`"*). The adapter does not merely promise this: it
//!    latches realize and **refuses** a topology call afterwards, so the discipline is a
//!    mechanism (see [`crate::QemuMachine::finish_realize`]).
//! 3. **[`QemuHost::signal_msix`] is one descriptor write and nothing else.** It is the
//!    single in-lock-legal foreign call (§7), and it must never become a notify call that
//!    takes the global lock.
//!
//! ## ★ What is deliberately NOT here
//!
//! No display/present sink (§9.5), no migration serialisation (§9.4 — migration is
//! blocked outright), no deferred-callback scheduling (§4.5, decision Q3: our threads
//! stay ours), and no interrupt *masking* callbacks — those arrive from above, holding
//! the VMM's global lock, and are latch-and-defer like every other such callback.

use core::ops::Range;

use kayfabe_vmm::BarId;

/// Backend-scoped identity of one foreign memory region we hold a counted reference to.
///
/// Opaque by contract: on a real host it names a `MemoryRegion *` the raw crate owns; in
/// the mock it indexes a `Vec`. Nothing above this seam may do arithmetic on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MrHandle(pub u64);

/// Backend-scoped identity of one migration blocker, so it can be withdrawn at
/// unrealize (§8.4 — the blocker is paired, or a device that failed to realize leaves
/// the machine permanently unmigratable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockerId(pub u64);

/// Why the hypervisor refused.
///
/// ★ [`HostError::Busy`] is a **named variant, not an errno**, because §8.5's
/// `-EBUSY` arm is the one refusal realize must report differently from every other:
/// it means a discard *requirer* (a memory-ballooning-adjacent device) is already
/// present in this machine, which is an operator's configuration mistake and not a bug
/// in us. `testing_doctrine.md` §2 rule 3 — the near neighbour it must never report as
/// is [`HostError::Refused`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    /// A conflicting requirer is already present — §8.5's `-EBUSY` arm.
    Busy {
        /// What conflicts, in operator-readable terms.
        what: &'static str,
    },
    /// The host refused, with its own error number when it reported one.
    Refused {
        /// Which operation was refused.
        what: &'static str,
        /// The OS error number, when the host reported one.
        errno: Option<i32>,
    },
    /// The host cannot express the request at all.
    Unsupported(&'static str),
}

/// The raw per-section facts the topology listener reports, **before** classification.
///
/// ★★ Five fields, because §5.3's finding is that `is_ram` alone is wrong in three
/// independent directions and the complete test is not expressible as one predicate at
/// the pinned version. The listener hands these across unclassified precisely so
/// [`crate::classify`] stays the **one** place the rule lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SectionFacts {
    /// The region reports itself as RAM. **Necessary, nowhere near sufficient.**
    pub is_ram: bool,
    /// The region is a *device* RAM region — direct-access-looking, possibly real MMIO.
    /// A pass-through-mapped BAR is exactly this shape and `is_ram` is **true** for one.
    pub is_ram_device: bool,
    /// The region is a ROM-device region: reads are direct, **writes go to callbacks**.
    /// Memcpy-ing a guest write into one bypasses the owning device's write path.
    pub is_rom_device: bool,
    /// The *section* is read-only. A first-class property of the section, not a hint.
    pub readonly: bool,
    /// The section is non-volatile (persistent-memory-shaped).
    pub nonvolatile: bool,
}

impl SectionFacts {
    /// Plain host RAM: the only shape [`crate::classify::is_ram`] admits.
    #[must_use]
    pub fn plain_ram() -> Self {
        SectionFacts {
            is_ram: true,
            is_ram_device: false,
            is_rom_device: false,
            readonly: false,
            nonvolatile: false,
        }
    }

    /// An ordinary device register window: not RAM by any measure.
    #[must_use]
    pub fn device() -> Self {
        SectionFacts {
            is_ram: false,
            is_ram_device: false,
            is_rom_device: false,
            readonly: false,
            nonvolatile: false,
        }
    }
}

/// One section of the guest-physical topology, as the listener reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionDesc {
    /// The region this section is a slice of.
    pub mr: MrHandle,
    /// Guest-physical base of the section.
    pub gpa: u64,
    /// Length in bytes.
    pub len: u64,
    /// Byte offset of the section's first byte within `mr`'s backing.
    pub offset_within_region: u64,
    /// The unclassified facts (§5.3).
    pub facts: SectionFacts,
}

/// Where a region we publish is placed, so the caller never has to name a raw address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarPlacement {
    /// Which BAR the region is a subregion of.
    pub bar: BarId,
    /// Guest-physical base the BAR is programmed at.
    pub base: u64,
    /// Length of the BAR in bytes.
    pub len: u64,
}

/// # The hypervisor capabilities the QEMU adapter needs, and no others
///
/// `&self` throughout: the hypervisor owns its own synchronisation, and several of these
/// are called from threads that hold one of *our* leaf locks. Nothing here may take a
/// lock of ours, and nothing here may block.
pub trait QemuHost: Send + Sync + core::fmt::Debug {
    // --- realize-time facts (§3.4, §3.5) -----------------------------------------

    /// The **binary's** version as `(major, minor)`.
    ///
    /// ★ Not a substitute for the compile-time floor and not substituted by it (§3.5):
    /// the build-time check is a claim about the headers, this is a claim about the
    /// binary, and a header-only mismatch or an ABI-compatible relink separates them.
    fn version(&self) -> (u32, u32);

    /// Whether the machine is running on the hardware accelerator.
    ///
    /// ★ The lockless-IO opt-out is honoured only on the accelerator's dispatch path, so
    /// an interpreted machine would run this device holding the VMM's global lock on
    /// **every** access — the measured 5.3× amplification, invisible from inside (§3.4).
    /// Realize refuses rather than offering a silent slow mode.
    fn kvm_enabled(&self) -> bool;

    // --- realize/unrealize only: the topology transactions (§4.3) ------------------

    /// Block migration and checkpoint-restart, naming our reason (§8.4).
    ///
    /// # Errors
    /// [`HostError::Refused`] if the host will not take the blocker.
    fn migrate_add_blocker(&self, reason: &'static str) -> Result<BlockerId, HostError>;

    /// Withdraw a blocker at unrealize.
    fn migrate_del_blocker(&self, id: BlockerId);

    /// Refuse guest-driven discard of RAM ranges machine-wide (§8.5, decision Q4).
    ///
    /// ★ Mandatory, not tidy: the balloon path accepts any section that reports itself
    /// RAM and calls a discard helper that has **no check for the preallocated flag**, so
    /// a guest that hands it a guest-physical address inside our reservation reaches a
    /// `madvise`-shaped zeroing underneath live placements.
    ///
    /// # Errors
    /// [`HostError::Busy`] if a discard *requirer* is already present — realize must
    /// refuse and name the conflict, never proceed.
    fn ram_block_discard_disable(&self, disable: bool) -> Result<(), HostError>;

    /// Start receiving topology callbacks for the address space this device does DMA in
    /// (§5.2).
    ///
    /// # Errors
    /// [`HostError::Refused`].
    fn register_listener(&self) -> Result<(), HostError>;

    /// Publish one of **our** reservations as a RAM region and add it as a subregion of
    /// `at.bar` (§5.1, §5.4's coarse tier).
    ///
    /// The host takes the mapping by pointer and never frees it — teardown is entirely
    /// ours (§8.3 step 8).
    ///
    /// # Errors
    /// [`HostError::Refused`] / [`HostError::Unsupported`].
    fn publish_window(
        &self,
        name: &'static str,
        at: BarPlacement,
        bar_offset: u64,
        len: u64,
    ) -> Result<MrHandle, HostError>;

    /// Publish a **host-owned** read-native overlay: reads are served from RAM the host
    /// allocated, writes go to our callbacks (§5.4).
    ///
    /// ★ The host allocates that RAM. There is no pointer-taking variant of this
    /// constructor at the pinned version, so the backing is **not** inside our
    /// reservation and nothing may ever be placed into it.
    ///
    /// # Errors
    /// [`HostError::Refused`] / [`HostError::Unsupported`].
    fn publish_rom_overlay(
        &self,
        name: &'static str,
        at: BarPlacement,
        bar_offset: u64,
        len: u64,
    ) -> Result<MrHandle, HostError>;

    /// Remove a region we published and drop our reference to it.
    ///
    /// ★ This is the destructor-shaped foreign call §0 item 1 is about, and it is
    /// confined to unrealize for exactly that reason: it may run a finalizer, and a
    /// finalizer wants the VMM's global lock — which unrealize is already holding and a
    /// deferred-collection thread would not be. See [`crate::QemuMachine::unrealize`].
    fn unpublish(&self, mr: MrHandle);

    // --- the listener's own reference counting (§5.2) ------------------------------

    /// Take a counted reference to a foreign region the listener reported.
    ///
    /// # Errors
    /// [`HostError::Refused`] if the region is unknown to the host.
    fn ref_region(&self, mr: MrHandle) -> Result<(), HostError>;

    /// Release a counted reference. Called from the topology callback itself, which
    /// arrives holding the VMM's global lock — the one context where a finalizer is
    /// legal (§5.2).
    fn unref_region(&self, mr: MrHandle);

    // --- the hot path: no global lock, no topology, no blocking --------------------

    /// Copy `dst.len()` bytes out of `mr` starting at `off`.
    ///
    /// # ★★★ NORMATIVE
    ///
    /// A bounded memcpy against **this region's own backing**. It MUST NOT be spelled as
    /// a general read-anywhere accessor: that entry point takes the VMM's global lock
    /// whenever the target is not direct-access, which would put a foreign lock beneath
    /// one of our ranked locks — the inversion [`kayfabe_vmm::GuestRamMap`] exists to
    /// make unconstructible.
    ///
    /// # Errors
    /// [`HostError::Refused`] if `off + dst.len()` leaves the region.
    fn read_region(&self, mr: MrHandle, off: u64, dst: &mut [u8]) -> Result<(), HostError>;

    /// Copy `src` into `mr` at `off`. **The same normative contract**, and the write
    /// direction is the sharper one — a stray write into a device register window is a
    /// side effect on hardware, not merely a bad read.
    ///
    /// # Errors
    /// [`HostError::Refused`] if `off + src.len()` leaves the region.
    fn write_region(&self, mr: MrHandle, off: u64, src: &[u8]) -> Result<(), HostError>;

    /// Raise an interrupt vector.
    ///
    /// ★ **One descriptor write** (§7). This is the single in-lock-legal foreign call in
    /// the whole adapter, and its legality is a property of the *implementation*, not of
    /// the name: an implementor that spells it as a notify call taking the VMM's global
    /// lock has silently made every completion delivery an inversion.
    ///
    /// # Errors
    /// [`HostError::Refused`] / [`HostError::Unsupported`] (a vector the device was not
    /// realized with).
    fn signal_msix(&self, vector: u16) -> Result<(), HostError>;
}

/// A read-native overlay the device is realized with (§5.4).
///
/// ★★ **Why this is realize-time configuration and not a runtime argument.** Publishing
/// an overlay is a topology transaction, and §4.3 confines those to realize/unrealize.
/// [`kayfabe_vmm::Vmm::map_read_native`] is a *runtime* call, so on this backend it can
/// only **claim** an overlay that already exists, never create one. That is consistent
/// with §5.4's own scope for the class — *"small and static (faked registers)"* — and it
/// is the reason the refusal for an unmatched request names the constraint rather than
/// reporting a generic fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlaySpec {
    /// Which BAR the overlay lives in.
    pub bar: BarId,
    /// Guest-physical base.
    pub gpa: u64,
    /// Length in bytes.
    pub len: u64,
    /// The write-trap sub-range, in guest-physical addresses, or `None` for "the whole
    /// overlay traps writes" — which is what a ROM-device region does natively.
    pub write_trap: Option<Range<u64>>,
}
