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
//! 2. ★★★ **There is no topology-transaction method left.** Under
//!    `host_execution_plane.md` §1 the hypervisor **reserves** the guest-physical window
//!    and does not back it; we install our own memslots. So this trait no longer has a
//!    `publish_window`, a `publish_rom_overlay` or an `unpublish` — nothing on it mutates
//!    the hypervisor's region tree at all, in realize or out of it, and §4.3's *"the
//!    adapter contains ZERO calls to `bql_lock`"* is discharged by there being nothing to
//!    call it for. What is left is three realize-time *facts* (version, accelerator,
//!    reservation shape), two paired lifecycle calls (the migration blocker and the
//!    discard refusal), and the hot path.
//! 3. **[`QemuHost::signal_msix`] and [`QemuHost::bar_base`] are one field access each.**
//!    They are the only in-lock-legal foreign calls (§7), and it must never become a notify
//!    call that takes the global lock.
//!
//! ## ★ What is deliberately NOT here
//!
//! No display/present sink (§9.5), no migration serialisation (§9.4 — migration is
//! blocked outright), no deferred-callback scheduling (§4.5, decision Q3: our threads
//! stay ours), and no interrupt *masking* callbacks — those arrive from above, holding
//! the VMM's global lock, and are latch-and-defer like every other such callback.
//!
//! ★★ And, since 2026-07-29, **no memslot method either**. Installing a memslot is a call
//! to the *kernel*, not to the hypervisor, so it does not belong on a trait whose subject
//! is the hypervisor's C API. It crosses [`crate::slots::SlotPlane`] instead — a separate
//! seam with a real implementation (`kayfabe_linux_raw::KvmVm`) and a mock, so the tiering
//! can be tested against a real kernel rather than against a double that cannot refuse.

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

    /// ★★★ Refuse guest-driven discard of RAM ranges machine-wide (decision Q4, task #97).
    ///
    /// # ★★ §8.5's reasoning is VOID; this is the argument that replaces it
    ///
    /// §8.5 justified this by the balloon reaching *our reservation*. Under
    /// `host_execution_plane.md` §1 the reservation is no longer a hypervisor RAM block at
    /// all, so the balloon skips it **trivially** — its inflate path only walks the
    /// machine's own RAM blocks (`qemu: hw/virtio/virtio-balloon.c:425-435`) and ours is
    /// not one. The old argument does not survive the decision it was written under.
    ///
    /// **The real hazard is guest RAM exported to isolates.** That backing is a shared
    /// `memfd`, and the discard helper punches the **file** with `FALLOC_FL_PUNCH_HOLE`
    /// (`qemu: system/physmem.c:3629-3692`) — which destroys the file's backing pages and
    /// therefore reaches **every** mapping of it: the hypervisor's, the accelerator's
    /// second-stage tables, and the isolate's own `MAP_SHARED` one. The hypervisor's own
    /// comment warns the call works *"as long as nobody else uses that file"*; an isolate
    /// holding a mapping of exported guest RAM is precisely somebody else. This is the same
    /// rationale a pass-through device driver has, and it is why the call is mandatory
    /// rather than tidy.
    ///
    /// # Errors
    /// [`HostError::Busy`] if a discard *requirer* is already present — a device that needs
    /// guest-driven discard to function. Realize must refuse **naming the conflict**, never
    /// proceed: the two devices cannot coexist, and an operator who is told only
    /// *"refused"* has to bisect their command line to find out which one to remove. See
    /// [`HostError::Busy::what`], which is the name, and `crate::qemu_refused`, which is
    /// what carries it out to the caller instead of flattening it to a class.
    fn ram_block_discard_disable(&self, disable: bool) -> Result<(), HostError>;

    /// Start receiving topology callbacks for the address space this device does DMA in
    /// (§5.2).
    ///
    /// # Errors
    /// [`HostError::Refused`].
    fn register_listener(&self) -> Result<(), HostError>;

    // --- the reservation BAR (host_execution_plane.md §1, §1.5) --------------------

    /// ★★★ Whether `bar` is a **pure-MMIO reservation** the hypervisor does not back.
    ///
    /// `host_execution_plane.md` §1.5, resolved against the hypervisor's own source: the
    /// reservation object is `memory_region_init_io` + a 64-bit prefetchable BAR
    /// (`C: src/qemu/virtio_nvgpu_pci.c:108-114`), whose stub ops are never reached because
    /// our memslot shadows it. That construction *never sets the region's RAM flag*
    /// (`qemu: system/memory.c:1568-1579`), so the accelerator's listener takes an
    /// unconditional early return for it (`qemu: accel/kvm/kvm-all.c:1449-1457`) — **before**
    /// allocating a slot, in both directions, so a BAR remap produces exactly zero slot
    /// operations.
    ///
    /// **That early return is the entire safety argument for installing foreign memslots,
    /// and it is a property of how the BAR was constructed.** A BAR the shim registered
    /// with a RAM-backed constructor instead would get a hypervisor-managed slot of its
    /// own, over the same guest-physical range as ours — and only one of the two can win.
    /// So the adapter refuses to place a window in a BAR that does not report this, rather
    /// than trusting that whoever wrote the shim read §1.5.
    fn bar_is_unbacked_reservation(&self, bar: BarId) -> bool;

    /// ★★★ Where `bar` is **currently** programmed, or `None` while it is unmapped.
    ///
    /// # NORMATIVE — this is a field read, and it is on the hot path
    ///
    /// Called on **every** guest-physical access that lands in a window, because
    /// `host_execution_plane.md` §1.5's *"the C has a LATENT BUG here"* finding is that a
    /// base cached once and never re-checked is silently wrong the moment the guest moves
    /// the BAR: the hypervisor's own region follows, our memslot does not, the old
    /// guest-physical range keeps working **and becomes reassignable to another BAR**, and
    /// the new one reads zeros. The C caches it once (`C: nvkvm_mmap_host.c:189-193`) and
    /// has no BAR-change hook anywhere.
    ///
    /// So an implementor must spell this as **one read of the device's own PCI bookkeeping**
    /// — `C: virtio_nvgpu_pci.c:71-76` is the whole function — and never as a call that
    /// takes the VMM's global lock. It is in the same in-lock-legal class as
    /// [`QemuHost::signal_msix`], and for the same reason: its legality is a property of
    /// the implementation, not of the name.
    fn bar_base(&self, bar: BarId) -> Option<u64>;

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
