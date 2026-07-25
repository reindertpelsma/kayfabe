//! # kayfabe-vmm — the hypervisor-adapter boundary (port, not implementation)
//!
//! `mode2_rust_rewrite_architecture.md` §4.1: the core needs a **small, fixed set of
//! capabilities** from a hypervisor — the eight methods groups of [`Vmm`] — and nothing
//! else. Notably absent, by proven design (experiment E0): **vCPU register access** —
//! doorbell demux is vChid-keyed (GPU-side identity), so no backend ever needs CPU-state
//! introspection.
//!
//! This crate defines the **traits and value types only**. Real backends live in future
//! adapter crates (`kayfabe-vmm-qemu`: thin C-shell glue over the staticlib;
//! `kayfabe-vmm-ch`: cloud-hypervisor/rust-vmm). Tests use `kayfabe-mocks::MockVmm`.
//! The core is *deterministically testable without a GPU or a hypervisor* precisely
//! because every effect crosses this seam.
//!
//! **Threading contract** (§4.1 + decision #17): the adapter provides the
//! synchronization; the core provides safety under *any* strategy. All [`Device`]
//! entry points take `&mut self`, so the adapter must give the core exclusive
//! access per call — a whole-device serialization (QEMU's BQL, a per-device mutex)
//! is the simplest valid strategy, and per-`Proc` sharding is equally sound
//! (core types are `Send + Sync`, mutation is `&mut`-exclusive, and the per-proc
//! planes are disjoint — see `kayfabe-core`'s concurrency contract). Isolate I/O
//! completes via [`CoreEvent`]s on the adapter's executor, never by re-entry from
//! an isolate thread. [`Vmm`] itself is a *passed argument*, not core-owned state:
//! it is `Send` (handed between adapter threads) but carries no `Sync` bound — the
//! adapter owns its synchronization.

use core::ops::Range;
use core::time::Duration;
use kayfabe_util::Instant;

/// Error from a VMM capability call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmmError {
    /// The GPA range is not backed by guest memory.
    BadGpa {
        /// Offending guest-physical address.
        gpa: u64,
    },
    /// The slot/region handle is unknown or already removed.
    BadSlot(SlotId),
    /// The backend cannot satisfy the request (resource exhaustion, unsupported mode).
    Unsupported(&'static str),
}

/// Identifies an installed guest-physical mapping (memslot) for later unmap/lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotId(pub u64);

/// Which PCI BAR an MMIO range/trap refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BarId {
    /// Register BAR (boot, IRQ, doorbell).
    Bar0,
    /// GMMU-walked aperture BAR (USERD/GPFIFO CPU access).
    Bar1,
    /// Second GMMU-walked aperture BAR.
    Bar2,
}

/// Host memory to back a guest-physical range with. Abstract: the *backend* knows
/// what a shareable region is on its OS; the core only names one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostRegion {
    /// Backend-scoped opaque region id (a mock index, an fd+offset on Linux, …).
    pub id: u64,
    /// Byte offset into the region.
    pub offset: u64,
}

/// Protection for an installed mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Prot {
    /// Guest may read and write.
    ReadWrite,
    /// Guest may only read (writes fault to the trap dispatcher).
    ReadOnly,
}

/// Trap mode for a registered MMIO range (§4.4 page-policy classes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrapMode {
    /// Reads and writes both dispatch to [`Device::mmio_read`]/[`Device::mmio_write`].
    ReadWrite,
    /// Reads are served natively (RAM-backed); only writes trap — the
    /// `gsp_falcon` rom-device overlay pattern (lesson L12).
    WriteOnly,
}

/// An interrupt to inject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IrqSpec {
    /// MSI-X vector index.
    Msix(u16),
    /// Legacy INTx line level.
    IntxLevel(bool),
}

/// A shareable handle over (a slice of) guest RAM, for mapping into isolates
/// (the `m2_stub_ram_base` MAP_FIXED share; Mode-1's double-mmap). Opaque to the
/// core; the isolate adapter consumes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RamHandle {
    /// Backend-scoped opaque token.
    pub token: u64,
    /// The guest-physical range this handle covers (`None` = all of guest RAM).
    pub covers: Option<Range<u64>>,
}

/// An opaque **host-surface** token: a presentable render-target surface minted by
/// the owning isolate's `RmBackend::export_surface` (the `PRIME_HANDLE_TO_FD`
/// dma-buf export, C-proven — `present_path_b_done`). Deliberately DISTINCT from
/// [`RamHandle`]: the proven present path scans out **host VRAM**, not a slice of
/// guest RAM, so a guest-RAM handle must never typecheck into [`Present::present`]
/// (seam audit GR-2a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceHandle(pub u64);

/// Metadata describing a scanout framebuffer to present (`execution_plane.md` §2.6).
/// Abstract: the core names the geometry; the concrete adapter (QEMU/PRIME) knows how
/// to turn `buffer` into a real dma-buf and scan it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FbMeta {
    /// Framebuffer width in pixels.
    pub width: u32,
    /// Framebuffer height in pixels.
    pub height: u32,
    /// Row stride in bytes.
    pub stride: u32,
    /// Abstract pixel format tag (the adapter maps it to a concrete fourcc).
    pub format: u32,
}

/// A completed present, fed back into the core as a synthetic vblank
/// (`execution_plane.md` §2.6). The GR-graphics completion tie-in observes this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vblank {
    /// Monotonic present/frame sequence number.
    pub seq: u64,
}

/// Error from a [`Present`] sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentError {
    /// The buffer handle is unknown / not a valid scanout source (its token).
    BadBuffer(u64),
    /// The sink cannot present (unsupported format, no display, …).
    Unsupported(&'static str),
}

/// # The abstract present / display sink — GR-graphics's home
///
/// The seam that keeps display **hypervisor/host-agnostic** (`execution_plane.md`
/// §2.6/§3.3): a GR-graphics context routes its scanout buffer here; the concrete
/// adapter turns it into a `PRIME_HANDLE_TO_FD` dma-buf → VMM present → host
/// present-complete, fed back as a synthetic [`Vblank`]. **NEVER NVKMS forwarding.**
///
/// This crate defines only the trait; the QEMU/PRIME adapter is a LATER concrete impl.
/// `kayfabe-mocks::MockPresent` is the test impl so the seam exists and is exercised now.
pub trait Present {
    /// Present the scanout `buffer` — a [`SurfaceHandle`] minted by the owning
    /// isolate's `RmBackend::export_surface` (host VRAM, never guest RAM) — with
    /// geometry `meta`. Returns the [`Vblank`] the present completed on — the core
    /// feeds it back as a synthetic vblank via the completion tie-in.
    fn present(&mut self, buffer: SurfaceHandle, meta: FbMeta) -> Result<Vblank, PresentError>;
}

/// Discriminates deferred-work callbacks so `Vmm::defer`/`lock_region` can name
/// which core path to re-enter without a closure crossing the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CoreEventKind {
    /// Completion re-delivery sweep (per-proc, §4.3.2).
    CompletionRedeliver,
    /// Deferred heavy-state reap at a quiesce point (lesson L10).
    DeferredReap,
    /// Poll-kick budget expiry.
    PollKickBudget,
    /// A locked region faulted (memory-lock primitive, capability 8).
    RegionFault,
}

/// An event delivered back into the core on the device's serialized executor.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum CoreEvent {
    /// A timer/deferred-work callback scheduled via [`Vmm::defer`].
    Deferred(CoreEventKind),
    /// A guest access faulted on a locked region ([`Vmm::lock_region`]).
    LockedRegionFault {
        /// The slot that faulted.
        slot: SlotId,
        /// Guest-physical address of the faulting access.
        gpa: u64,
    },
    /// An isolate completed an asynchronous host operation.
    IsolateComplete {
        /// The isolate's session id (== ProcId by construction, §4.3.4).
        session: u64,
        /// Opaque per-op cookie the core supplied when issuing the op.
        cookie: u64,
    },
}

/// # The hypervisor adapter — everything the Mode-2 core may ask of a VMM
///
/// One instance per emulated GPU device; object-safe. Eight capability groups
/// (arch doc §4.1 — *"count is not the invariant; hypervisor-agnosticism is"*):
///
/// 1. GPA read/write — the DMA plane.
/// 2. Memslot map/unmap — guest-physical mapping management.
/// 3. MMIO trap registration — the VMM routes trapped accesses to [`Device`].
/// 4. Interrupt injection.
/// 5. Guest-RAM export for isolates.
/// 6. Deferred work + virtual time.
/// 7. Read-native overlay (RAM-backed reads, write sub-range traps).
/// 8. The memory-lock primitive (revoke → fault+wait → update → restore).
///
/// The core never calls the OS or the VMM except through this trait (and
/// `RmBackend` inside isolates). That sentence is the invariant the whole rewrite
/// is judged on.
pub trait Vmm: Send {
    // --- 1. Guest-physical memory access -----------------------------------------

    /// Read guest-physical memory into `buf` (RPC queue, FB shadow, pushbuffer,
    /// page-table reads).
    fn gpa_read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), VmmError>;

    /// Write `buf` to guest-physical memory (semaphore/status writes into guest RAM).
    fn gpa_write(&mut self, gpa: u64, buf: &[u8]) -> Result<(), VmmError>;

    // --- 2. Guest-physical mapping management (memslots) -------------------------

    /// Install host memory into guest-physical space: BAR backings, shared
    /// USERD/GPFIFO pages, per-process GPA-arena slices.
    fn map_guest(
        &mut self,
        gpa: u64,
        len: u64,
        backing: HostRegion,
        prot: Prot,
    ) -> Result<SlotId, VmmError>;

    /// Remove a previously installed mapping.
    fn unmap_guest(&mut self, slot: SlotId) -> Result<(), VmmError>;

    // --- 3. MMIO/PIO trap registration -------------------------------------------

    /// Register a trapped MMIO range on `bar`. The VMM dispatches trapped accesses
    /// back into the core via [`Device::mmio_read`]/[`Device::mmio_write`]; the
    /// adapter only routes.
    fn set_trap(&mut self, bar: BarId, range: Range<u64>, mode: TrapMode) -> Result<(), VmmError>;

    // --- 4. Interrupt injection ---------------------------------------------------

    /// Inject an interrupt into the guest.
    fn raise_irq(&mut self, irq: IrqSpec) -> Result<(), VmmError>;

    // --- 5. Guest-RAM export ------------------------------------------------------

    /// Export (a slice of) guest RAM as a shareable handle for isolate mapping.
    /// Per-slice export supports least-privilege sharing (§4.3.4).
    fn export_ram(&mut self, slice: Option<Range<u64>>) -> Result<RamHandle, VmmError>;

    // --- 6. Deferred work + time --------------------------------------------------

    /// Schedule `event` for delivery to [`Device::event`] after `after`, on the
    /// device's serialized executor (bottom-half equivalent).
    fn defer(&mut self, after: Duration, event: CoreEvent);

    /// Current virtual time. Deterministic under test (the mock advances it
    /// explicitly); never a wall-clock read in the core.
    fn now(&self) -> Instant;

    // --- 7. Read-native overlay ---------------------------------------------------

    /// Back `gpa..gpa+len` with RAM the core keeps current so guest READS are served
    /// without a VMM op, while `write_trap` (if any) still dispatches writes to
    /// [`Device::mmio_write`] — the rom-device pattern that killed the nested-virt
    /// poll storm (faked-reg class iv-a, decision #12).
    fn map_read_native(
        &mut self,
        gpa: u64,
        len: u64,
        backing: HostRegion,
        write_trap: Option<Range<u64>>,
    ) -> Result<SlotId, VmmError>;

    // --- 8. Memory-lock primitive -------------------------------------------------

    /// Revoke access to a live untrapped mapping: the next guest access faults and
    /// blocks; the fault is delivered as [`CoreEvent::LockedRegionFault`] tagged
    /// `on_fault`. The core then updates the backing atomically and calls
    /// [`Vmm::unlock_region`]. Hypervisor-agnostic (userfaultfd / memslot
    /// revoke-restore), never host `mprotect` (decision #6).
    fn lock_region(&mut self, slot: SlotId, on_fault: CoreEventKind) -> Result<(), VmmError>;

    /// Restore access to a locked region and release any blocked guest access.
    fn unlock_region(&mut self, slot: SlotId) -> Result<(), VmmError>;
}

/// # The core as seen by the adapter
///
/// The composition root (`kayfabe-core`'s `Gpu`) implements this; a backend routes
/// trapped MMIO and deferred events here. All entry points are serialized per
/// device by the adapter (threading contract, crate docs).
pub trait Device {
    /// A trapped MMIO read of `size` bytes at `off` in `bar`.
    fn mmio_read(&mut self, vmm: &mut dyn Vmm, bar: BarId, off: u64, size: u8) -> u64;

    /// A trapped MMIO write of `size` bytes at `off` in `bar`.
    fn mmio_write(&mut self, vmm: &mut dyn Vmm, bar: BarId, off: u64, size: u8, val: u64);

    /// A deferred callback, lock fault, or isolate completion.
    fn event(&mut self, vmm: &mut dyn Vmm, ev: CoreEvent);
}

// The concurrency contract, compile-time-asserted (decision #17): every value type
// crossing the seam is `Send + Sync`; `dyn Vmm` is `Send` by supertrait (the
// threading contract in the crate docs explains why `Sync` is not required).
kayfabe_util::assert_send_sync!(
    VmmError,
    SlotId,
    BarId,
    HostRegion,
    Prot,
    TrapMode,
    IrqSpec,
    RamHandle,
    SurfaceHandle,
    FbMeta,
    Vblank,
    PresentError,
    CoreEventKind,
    CoreEvent,
);
kayfabe_util::assert_send!(dyn Vmm);
