//! # `kayfabe-vmm-qemu` — the QEMU adapter's **logic half**, and all of it
//!
//! `l2_qemu_adapter.md` stage **Q1**: *"the pure half — the biggest stage, and it needs
//! nothing."* No hypervisor, no guest, no GPU. Every foreign effect crosses
//! [`host::QemuHost`], and [`mock_host::MockQemuHost`] is the implementor the whole suite
//! runs against. `forbid(unsafe_code)`, like [`kayfabe_vmm_kvm`], and for the same reason:
//! the unsound surface belongs in one auditable crate per host axis, never in the logic.
//!
//! [`kayfabe_vmm_kvm`]: https://docs.rs/kayfabe-vmm-kvm
//!
//! ## ★★ The four places this crate departs from the design it implements
//!
//! Each is a **finding**, each is load-bearing, and each has a mechanism here rather than
//! a note. They are collected at the top because a reader who takes §5 at face value will
//! not otherwise find them.
//!
//! ### 1. Topology is realize-only, so the coarse memory tier is too
//!
//! §4.3 is absolute — *"the adapter contains ZERO calls to `bql_lock`"*, and every
//! topology mutation is confined to realize/unrealize, which the hypervisor already
//! entered holding its global lock. §5.4 then spells the coarse tier as *"one region
//! added as a BAR subregion"* and the read-native overlay as *"a higher-priority
//! subregion added with an overlap-adding call"*. **Both of those are topology
//! transactions**, and [`kayfabe_vmm::Vmm::map_read_native`] is a **runtime** method.
//!
//! The two rules cannot both hold with a runtime constructor, so this crate takes §4.3 as
//! the stronger one and makes it a **mechanism**: [`QemuMachine::realize`] creates every
//! window and every overlay from configuration and then **latches**. Afterwards
//! [`QemuMachine::install_ram_window`] refuses by name, and
//! [`kayfabe_vmm::Vmm::map_read_native`] can only **claim** an overlay that already
//! exists. That is consistent with §5.4's own scope for the read-native class — *"small
//! and static"* — and it means §4.3 is enforced by a latch with a negative test instead
//! of by a discipline with a comment.
//!
//! The consequence §6.7's per-window frequency rule cares about is unchanged: one
//! reservation, and per-process arenas are `MAP_FIXED` placements **inside** it, which
//! §5.4's fine tier already says perform no hypervisor call at all.
//!
//! ### 2. ★★ A foreign region is copied **inside** the view lock, and that DELETES the
//! retirement obligation §0 item 1 tries to create for it
//!
//! §0 item 1 reasons: releasing our last reference to a foreign region is a `Drop`-shaped
//! call site, therefore it is subject to #57's rule, therefore the answer is the
//! retire/collect machinery `kayfabe_vmm_kvm::Plane::retire` already implements.
//!
//! **The first two steps are right and the conclusion is backwards.** #57's fix moves a
//! release *off* the accessor's thread and onto a thread that has been proved lock-free.
//! For a foreign region that is the **wrong destination**: releasing the last reference
//! can run the owning object's finalizer, and a finalizer wants the VMM's global lock —
//! which the topology callback is already holding and a deferred-collection thread of
//! ours is not. Retiring a foreign region therefore takes a release that was legal where
//! it stood and moves it somewhere it is not.
//!
//! So this adapter splits the two backings by **who frees them**:
//!
//! | backing | released by | where | mechanism |
//! |---|---|---|---|
//! | our reservation (`Backing::Ours`) | us, an unmap | a door proved lock-free | retire + collect, exactly as #57 |
//! | a foreign region (`Backing::HostOwned`) | the owner's finalizer | the topology callback, global lock held | **no retirement**: the copy runs *inside* the view lock, so no accessor can hold the region across the callback |
//!
//! The cost is stated rather than hidden: a foreign copy serialises against another
//! foreign copy, which the reservation's copies do not. The benefit is that §5.5's second
//! R5 invalidator — *"a topology callback can retire a cached foreign pointer between our
//! plan and our commit"* — **stops existing**, by the same move
//! [`kayfabe_vmm::GuestRamMap`] uses to delete the `RamGpa` token: resolution and copy are
//! indivisible, so there is no gap to revalidate. The topology generation is still
//! counted, as an instrument (see [`AuditReport::topology_generation`]).
//!
//! ### 3. A read-native overlay is declared **RAM**, not a device
//!
//! §5.4 says to classify the overlay `Device` *"so the core can never `gpa_write` through
//! it"*. But [`kayfabe_vmm::Vmm::map_read_native`]'s own contract is *"RAM the core keeps
//! current"*, and the only way the core has to keep anything current is
//! [`kayfabe_vmm::Vmm::gpa_write`] — which a `Device` declaration refuses. The two cannot
//! both hold, and the existing suite settles it: the mean run writes into a read-native
//! overlay immediately after installing one, so a `Device` declaration would make this
//! backend refuse what the other backend serves, breaking stage Q1's own acceptance that
//! the two produce *identical* operation logs.
//!
//! The hazard §5.4 is defending against is real but is not the classification: it is
//! reaching a **foreign** region through a general read/write-anywhere accessor. Our
//! overlay is not foreign in that sense — we published it, we hold its handle, and
//! [`host::QemuHost::write_region`] is normatively a bounded memcpy against that one
//! region. So the overlay is `Ram` with a `Backing::HostOwned`, and the prove-RAM rule
//! is satisfied by an install we performed, exactly as [`kayfabe_vmm::GuestRamMap`]'s
//! rustdoc requires.
//!
//! ### 4. A caller-supplied host backing under a read-native overlay is **refused**
//!
//! §12 item 7 records that there is no pointer-taking constructor for a ROM-device region
//! and that the backing is therefore hypervisor-owned. It does not say what
//! [`kayfabe_vmm::Vmm::map_read_native`]'s `backing` argument then means. It cannot mean
//! what it means on the other backend — that spelling is a `MAP_FIXED` placement, and
//! §5.4 forbids placing anything into an overlay. So a non-sentinel backing is a
//! **backend-conditional refusal**, in the shape [`kayfabe_vmm::IrqSpec::IntxLevel`]
//! establishes: named, exact, and never a silent difference.
//!
//! ## The lock ladder
//!
//! ```text
//!   the VMM's global lock   (foreign, unrankable)  ── outermost, on the paths that have it
//!     │
//!   rank 0 / rank 1         the core's ranked locks
//!     │
//!   leaf(view) · leaf(installer)                   ── ours, unranked, leafwitness
//! ```
//!
//! Never the reverse. No leaf critical section calls the hypervisor **except**
//! [`host::QemuHost::read_region`]/[`host::QemuHost::write_region`], which are normatively
//! a bounded memcpy and take nothing — that exception is finding 2 above and is the one
//! place this adapter differs from the other backend's ladder.

#![allow(clippy::module_name_repetitions)]

pub mod classify;
pub mod host;
pub mod mock_host;

use core::ops::Range;
use core::time::Duration;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use kayfabe_linux_raw::{
    Backing as RawBacking, GuestWindow, HostOffset, HostPageSize, RawError, SharedRam, geometry,
};
use kayfabe_util::Instant;
use kayfabe_util::leafwitness::{self, assert_leaf_free};
use kayfabe_vmm::{
    BarId, CoreEvent, DeferQueue, GuestRamMap, HostRegion, IrqSpec, Prot, RamHandle, RamRegionId,
    RegionKind, SlotId, TrapMode, Vmm, VmmError,
};

use host::{BarPlacement, BlockerId, HostError, MrHandle, OverlaySpec, QemuHost, SectionDesc};

// =====================================================================================
// The refusal vocabulary — constants, because every one of them is asserted by content
// =====================================================================================

/// The minimum hypervisor version this adapter is written against (§3.5). Below it the
/// lockless-IO opt-out does not exist and the whole threading story of §4 is false.
pub const VERSION_FLOOR: (u32, u32) = (10, 2);

/// Realize-time refusal: the binary is older than [`VERSION_FLOOR`].
pub const BELOW_FLOOR: &str = "the hypervisor binary is below the 10.2 floor; the lockless-IO opt-out this device's \
     entire threading design rests on does not exist there";

/// Realize-time refusal: not running on the hardware accelerator (§3.4, decision Q6).
pub const NOT_ACCELERATED: &str = "this device requires the hardware accelerator; without it every trapped access runs \
     under the VMM's global lock, which is a measured 5.3x amplification and not a slow mode";

/// Realize-time refusal: a discard requirer is already present (§8.5's `-EBUSY` arm).
pub const DISCARD_REQUIRER_PRESENT: &str = "another device in this machine requires guest-driven RAM discard, which would let the \
     guest zero the backing under our live placements";

/// §4.3's refusal: a topology transaction outside realize/unrealize.
pub const TOPOLOGY_AFTER_REALIZE: &str = "a topology transaction after realize; on this backend that would need the VMM's global \
     lock, and the adapter takes it in no context but realize/unrealize";

/// `map_read_native` refusal: nothing was realized at that address.
pub const NO_SUCH_OVERLAY: &str = "no read-native overlay was realized at that guest-physical range; on this backend an \
     overlay is a topology transaction and therefore realize-time configuration";

/// `map_read_native` refusal: an overlay is there, but not of the requested shape.
pub const OVERLAY_SHAPE_MISMATCH: &str = "a read-native overlay exists at that guest-physical range but with a different length \
     or write-trap sub-range than the one requested";

/// `map_read_native` refusal: the overlay is already claimed by a live slot.
pub const OVERLAY_ALREADY_CLAIMED: &str = "that read-native overlay is already claimed by a live slot; unmap it before claiming it \
     again";

/// `map_read_native` refusal: finding 4 in the crate docs.
pub const OVERLAY_BACKING_UNPLACEABLE: &str = "a caller-supplied host backing under a read-native overlay; the overlay's RAM is the \
     hypervisor's, there is no pointer-taking constructor for it, and nothing may be placed \
     into it";

/// `map_guest` refusal: per-object protection (§6.7 item 4).
pub const PER_OBJECT_PROTECTION: &str = "per-object read-only protection inside a read-write window; protection is a WINDOW \
     property, so place the object in a read-only window instead";

/// `export_ram` refusal: the deployment fact no code gate can observe (§8.1 step 9).
pub const NO_SHARED_BACKING: &str =
    "guest RAM was not created with a shareable backing; an isolate cannot map it";

/// `raise_irq` refusal: the backend-conditional variant.
pub const NO_LEGACY_INTX: &str =
    "legacy line interrupts are not modelled by this device; only message-signalled vectors are";

/// `set_trap` refusal: the Rust half of §3.3's coverage clause.
pub const TRAP_OUTSIDE_THE_REALIZED_TABLE: &str = "a trap registration outside the realize-time region table; every trapped region of this \
     device is enumerated there and marked there, and a range the table does not cover is a \
     region nobody marked";

/// Topology-callback refusal: a reported section overlaps a region we published.
pub const FOREIGN_OVERLAPS_OURS: &str = "a reported topology section overlaps a region this device published itself; the two \
     sources would race to declare the same guest-physical range";

// =====================================================================================
// Configuration
// =====================================================================================

/// One reservation the device is realized with (§5.1, §5.4's coarse tier).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowSpec {
    /// Which BAR the reservation is published inside.
    pub bar: BarId,
    /// Guest-physical base. Must lie inside the named BAR and be page-aligned.
    pub gpa: u64,
    /// Length in bytes. Must be a whole number of host pages.
    pub len: u64,
}

/// One trapped region of the device, as the realize-time table names it.
///
/// ★ This is the Rust-side half of §3.3's clause (c). The C shim owns the literal region
/// table and the realize-time self-check over it; **this** list is what
/// [`kayfabe_vmm::Vmm::set_trap`] is validated against, so a core that registers a trap
/// the table does not cover is refused rather than silently bookkept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrapSpec {
    /// Which BAR.
    pub bar: BarId,
    /// The BAR-relative range the realized region covers.
    pub range: Range<u64>,
    /// How the realized region dispatches.
    pub mode: TrapMode,
}

/// What to realize.
#[derive(Debug, Clone, Default)]
pub struct MachineConfig {
    /// Whether guest RAM is created from a shareable backing. `false` models a machine
    /// launched without it: everything works until the first
    /// [`kayfabe_vmm::Vmm::export_ram`], which then refuses loudly.
    pub shareable_ram: bool,
    /// The BARs, whose ranges are declared [`RegionKind::Device`] at realize.
    pub bars: Vec<BarPlacement>,
    /// The reservations (§5.4's coarse tier), all of them realize-time.
    pub windows: Vec<WindowSpec>,
    /// The read-native overlays (§5.4), all of them realize-time.
    pub overlays: Vec<OverlaySpec>,
    /// The trapped regions [`kayfabe_vmm::Vmm::set_trap`] is validated against.
    pub traps: Vec<TrapSpec>,
}

impl MachineConfig {
    /// A machine with a shareable RAM backing and nothing else.
    #[must_use]
    pub fn shareable() -> Self {
        MachineConfig {
            shareable_ram: true,
            ..MachineConfig::default()
        }
    }
}

// =====================================================================================
// The audit
// =====================================================================================

/// What the adapter observed about itself. Every field is an instrument and every
/// instrument has a documented non-vacuity condition — a witness that cannot fail is the
/// failure mode this project has caught most often.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditReport {
    /// Reservations currently mapped.
    pub live_windows: u64,
    /// `MAP_FIXED` placements currently live inside reservations.
    pub live_placements: u64,
    /// Total bytes of reservation address space currently mapped.
    pub window_bytes: u64,
    /// High-water mark of live reservations — the non-vacuity half of `live_windows`.
    pub peak_windows: u64,
    /// High-water mark of live placements.
    pub peak_placements: u64,
    /// Cumulative `MAP_FIXED` placements — §9.3's memslot-frequency denominator. The
    /// numerator on this backend is a **constant**, because publication performs no
    /// hypervisor call at all: see `regions_published`.
    pub placements_made: u64,
    /// ★ Cumulative regions handed to the hypervisor. The frequency gate's numerator, and
    /// on this backend it must stop moving the instant realize returns.
    pub regions_published: u64,
    /// `(min, max)` of the core's ranked-lock depth observed at `gpa_read`/`gpa_write`.
    /// `max == 0` means the in-lock hazard was never exercised; `min == u32::MAX` means
    /// no access happened at all. Both halves are load-bearing.
    pub accessor_ranked_depth: (u32, u32),
    /// `(min, max)` of the core's ranked-lock depth observed at a syscall-shaped method.
    /// R1 says this must be `(0, 0)`, and `min == u32::MAX` is the vacuous case.
    pub syscall_ranked_depth: (u32, u32),
    /// ★ Max leaf depth observed at a copy out of **our own reservation**. Must be `0`:
    /// that copy runs outside the view lock, held alive by an `Arc`.
    pub own_copy_leaf_depth_max: u32,
    /// ★★ Min leaf depth observed at a copy out of a **host-owned** region. Must be
    /// `>= 1` once one has happened, because finding 2 keeps that copy *inside* the view
    /// lock. `u32::MAX` means no such copy ever ran — the vacuous case, and the reason
    /// this is a minimum rather than a maximum.
    pub host_copy_leaf_depth_min: u32,
    /// Max leaf depth observed while the view lock was held. Must be `>= 1`, or the leaf
    /// witness is measuring nothing.
    pub view_leaf_depth_max: u32,
    /// Guest-physical accesses served.
    pub accesses_served: u64,
    /// Guest-physical accesses refused by the region map.
    pub accesses_refused: u64,
    /// Memory-plane ops that failed in the execute phase — a real host refusal.
    pub host_refusals: u64,
    /// Ops that reached commit and were undone by the R5 re-validation — a placement
    /// whose reservation was torn down in the gap between plan and commit.
    ///
    /// ★★ **NOT YET WITNESSED, and said so rather than left to be discovered.** The arm is
    /// genuinely reachable — a teardown on another thread between this op's execute and
    /// commit phases — but no test in this suite has been *seen* to reach it, because the
    /// gap is a few instructions wide and there is no place to rendezvous inside it (the
    /// fine tier deliberately calls the hypervisor zero times, so there is no seam to
    /// interpose on). A test that retried until it landed would be a flake dressed as a
    /// gate, and one asserting a conditional invariant that may never fire would be the
    /// green-instrument-on-an-unexercised-path failure this project names. So this is a
    /// **named residual**: the arm is correct by inspection and unexercised by measurement,
    /// and closing it needs a seam that does not exist yet, not a longer loop.
    pub r5_revalidation_failures: u64,
    /// ★ §4.3's counter: topology transactions refused because realize had already
    /// latched. Non-vacuity for [`TOPOLOGY_AFTER_REALIZE`], which is otherwise a string
    /// nothing proves is reachable.
    pub topology_ops_refused_after_realize: u64,
    /// Topology sections added by the listener.
    pub topology_adds: u64,
    /// Topology sections removed by the listener.
    pub topology_dels: u64,
    /// ★ §5.5's generation. Not an R5 token here — finding 2 dissolves that obligation —
    /// but the instrument that says the listener really ran.
    pub topology_generation: u64,
    /// Read-native overlays currently claimed by a live slot.
    pub overlays_claimed: u64,
    /// Interrupt vectors raised.
    pub irqs_raised: u64,
    /// ★★ Reservation releases that found an accessor still holding the mapping and so
    /// handed the release to the machine instead of performing it inline. The non-vacuity
    /// half of #57's fix: with no live accessor the deferral never happens.
    pub window_releases_deferred: u64,
    /// Retired reservation mappings actually released. At quiescence this must equal the
    /// number removed, or a mapping is parked forever.
    pub window_mappings_released: u64,
}

#[derive(Debug, Default)]
struct Audit {
    live_windows: AtomicU64,
    live_placements: AtomicU64,
    window_bytes: AtomicU64,
    peak_windows: AtomicU64,
    peak_placements: AtomicU64,
    placements_made: AtomicU64,
    regions_published: AtomicU64,
    accessor_depth_min: AtomicU32,
    accessor_depth_max: AtomicU32,
    syscall_depth_min: AtomicU32,
    syscall_depth_max: AtomicU32,
    own_copy_leaf_max: AtomicU32,
    host_copy_leaf_min: AtomicU32,
    view_leaf_max: AtomicU32,
    accesses_served: AtomicU64,
    accesses_refused: AtomicU64,
    host_refusals: AtomicU64,
    r5_failures: AtomicU64,
    topology_refused: AtomicU64,
    topology_adds: AtomicU64,
    topology_dels: AtomicU64,
    topology_generation: AtomicU64,
    overlays_claimed: AtomicU64,
    irqs_raised: AtomicU64,
    window_releases_deferred: AtomicU64,
    window_mappings_released: AtomicU64,
}

impl Audit {
    /// ★ Hand-written for the reason the other backend's is: a derived `Default` gives
    /// every minimum `0`, which makes the lower half of every span assertion vacuously
    /// true — a witness reporting a constant would still pass. "Never observed" must be
    /// distinguishable from "observed zero".
    fn new() -> Self {
        Audit {
            accessor_depth_min: AtomicU32::new(u32::MAX),
            syscall_depth_min: AtomicU32::new(u32::MAX),
            host_copy_leaf_min: AtomicU32::new(u32::MAX),
            ..Audit::default()
        }
    }

    fn bump(count: &AtomicU64, peak: &AtomicU64, delta: i64) {
        let now = if delta >= 0 {
            count.fetch_add(delta.unsigned_abs(), Ordering::SeqCst) + delta.unsigned_abs()
        } else {
            count.fetch_sub(delta.unsigned_abs(), Ordering::SeqCst) - delta.unsigned_abs()
        };
        peak.fetch_max(now, Ordering::SeqCst);
    }

    fn note_ranked(min: &AtomicU32, max: &AtomicU32, d: u32) {
        min.fetch_min(d, Ordering::SeqCst);
        max.fetch_max(d, Ordering::SeqCst);
    }

    fn report(&self) -> AuditReport {
        let g = |a: &AtomicU64| a.load(Ordering::SeqCst);
        let h = |a: &AtomicU32| a.load(Ordering::SeqCst);
        AuditReport {
            live_windows: g(&self.live_windows),
            live_placements: g(&self.live_placements),
            window_bytes: g(&self.window_bytes),
            peak_windows: g(&self.peak_windows),
            peak_placements: g(&self.peak_placements),
            placements_made: g(&self.placements_made),
            regions_published: g(&self.regions_published),
            accessor_ranked_depth: (h(&self.accessor_depth_min), h(&self.accessor_depth_max)),
            syscall_ranked_depth: (h(&self.syscall_depth_min), h(&self.syscall_depth_max)),
            own_copy_leaf_depth_max: h(&self.own_copy_leaf_max),
            host_copy_leaf_depth_min: h(&self.host_copy_leaf_min),
            view_leaf_depth_max: h(&self.view_leaf_max),
            accesses_served: g(&self.accesses_served),
            accesses_refused: g(&self.accesses_refused),
            host_refusals: g(&self.host_refusals),
            r5_revalidation_failures: g(&self.r5_failures),
            topology_ops_refused_after_realize: g(&self.topology_refused),
            topology_adds: g(&self.topology_adds),
            topology_dels: g(&self.topology_dels),
            topology_generation: g(&self.topology_generation),
            overlays_claimed: g(&self.overlays_claimed),
            irqs_raised: g(&self.irqs_raised),
            window_releases_deferred: g(&self.window_releases_deferred),
            window_mappings_released: g(&self.window_mappings_released),
        }
    }
}

// =====================================================================================
// The plane
// =====================================================================================

/// What backs one declared guest-physical region.
///
/// ★★ The split is by **who frees it**, which is finding 2 in the crate docs.
#[derive(Debug, Clone)]
pub enum Backing {
    /// Inside our own reservation. The copy runs **outside** the view lock, held alive by
    /// this `Arc` — and the release is ours, so it is retired rather than dropped.
    Ours(
        /// The reservation the range lives in.
        Arc<GuestWindow>,
    ),
    /// A region the hypervisor owns: a section the topology listener reported, or one of
    /// our read-native overlays. The copy runs **inside** the view lock, so the owner's
    /// release can never race an accessor and never has to leave the context that is
    /// allowed to run a finalizer.
    HostOwned {
        /// The region the hypervisor owns.
        mr: MrHandle,
        /// Offset of this region's first byte within `mr`'s backing.
        region_off: u64,
    },
}

/// What `gpa_read`/`gpa_write` and the topology listener share, under the **view** leaf.
#[derive(Debug, Default)]
pub(crate) struct View {
    regions: GuestRamMap,
    backings: BTreeMap<RamRegionId, Backing>,
    /// Foreign sections the listener declared, keyed by guest-physical base, so
    /// `region_del` can undeclare exactly what `region_add` declared.
    foreign: BTreeMap<u64, (RamRegionId, u64, Option<MrHandle>)>,
    /// ★ The guest-physical ranges **this device published itself**, so a reported
    /// topology section that lands on one can be refused rather than silently replacing
    /// a declaration we own. §5.2 never says which source wins; here neither does,
    /// because the collision is a refusal.
    ours: Vec<(u64, u64)>,
    /// §5.5's counter. Bumped by every listener callback.
    topology_generation: u64,
}

/// One realize-time reservation, as the **installer** knows it.
#[derive(Debug)]
struct Window {
    gpa: u64,
    len: u64,
    window: Arc<GuestWindow>,
    placements: BTreeMap<SlotId, (u64, u64)>,
}

/// Everything one reservation's execute phase created, so a failure anywhere inside it
/// drops the whole lot before a single field has been recorded anywhere.
#[derive(Debug)]
struct Installed {
    window: Arc<GuestWindow>,
    ram: Option<Arc<SharedRam>>,
    mr: MrHandle,
}

/// One realize-time read-native overlay.
#[derive(Debug)]
struct Overlay {
    spec: OverlaySpec,
    mr: MrHandle,
    region: RamRegionId,
    claimed_by: Option<SlotId>,
}

/// The **installer**'s state: everything authoritative about the memory plane.
#[derive(Debug)]
struct Installer {
    windows: BTreeMap<RamRegionId, Window>,
    placement_owner: BTreeMap<SlotId, RamRegionId>,
    overlays: Vec<Overlay>,
    overlay_slot: BTreeMap<SlotId, usize>,
    traps: Vec<TrapSpec>,
    registered_traps: Vec<(BarId, Range<u64>, TrapMode)>,
    exports: Vec<std::os::fd::OwnedFd>,
    rams: BTreeMap<RamRegionId, Arc<SharedRam>>,
    published: Vec<MrHandle>,
    blocker: Option<BlockerId>,
    next_region: u64,
    next_slot_id: u64,
    /// Removed reservations whose mapping has not been released yet — #57's mechanism.
    retired: Vec<Arc<GuestWindow>>,
}

#[derive(Debug)]
pub(crate) struct Plane {
    host: Arc<dyn QemuHost>,
    page: HostPageSize,
    shareable_ram: bool,
    bars: Vec<BarPlacement>,
    /// ★★ §4.3 as a mechanism. Set at the end of realize, cleared at the start of
    /// unrealize; every topology transaction asserts it is clear.
    realized: AtomicBool,
    view: Mutex<View>,
    installer: Mutex<Installer>,
    clock: Mutex<(Instant, DeferQueue)>,
    audit: Audit,
}

impl Plane {
    fn view(&self) -> (MutexGuard<'_, View>, leafwitness::Held) {
        let g = self
            .view
            .lock()
            .expect("the view leaf lock is never poisoned");
        let held = leafwitness::Held::enter();
        self.audit
            .view_leaf_max
            .fetch_max(leafwitness::depth(), Ordering::SeqCst);
        (g, held)
    }

    fn installer(&self) -> (MutexGuard<'_, Installer>, leafwitness::Held) {
        let g = self
            .installer
            .lock()
            .expect("the installer leaf lock is never poisoned");
        let held = leafwitness::Held::enter();
        self.audit
            .view_leaf_max
            .fetch_max(leafwitness::depth(), Ordering::SeqCst);
        (g, held)
    }

    /// Every syscall-shaped entry point starts here — both halves of R1, at the door of
    /// the phase that performs the syscall.
    fn about_to_syscall(&self, what: &str) {
        Audit::note_ranked(
            &self.audit.syscall_depth_min,
            &self.audit.syscall_depth_max,
            kayfabe_util::lockwitness::held_depth(),
        );
        assert_leaf_free(what);
        // Every door that reaches this line has just been proved lock-free on both
        // halves, which makes it the legal place to release a reservation an accessor was
        // still reading when it was removed.
        self.collect_retired();
    }

    /// ★ §4.3's gate. A topology transaction is legal only before realize latches (or
    /// after unrealize clears it), because those are the contexts the hypervisor entered
    /// holding its own global lock.
    fn assert_realize_context(&self) -> Result<(), VmmError> {
        if self.realized.load(Ordering::SeqCst) {
            self.audit.topology_refused.fetch_add(1, Ordering::SeqCst);
            return Err(VmmError::Unsupported(TOPOLOGY_AFTER_REALIZE));
        }
        Ok(())
    }

    /// Park a removed reservation's mapping where no accessor's clone can be the last
    /// one — #57's ownership fix, verbatim, and it applies to [`Backing::Ours`] only.
    fn retire(&self, window: Arc<GuestWindow>) {
        if Arc::strong_count(&window) > 1 {
            self.audit
                .window_releases_deferred
                .fetch_add(1, Ordering::SeqCst);
        }
        // ★ Push only. An inline collection here would be **unfalsifiable** on this
        // backend and a bite-check said so: `retire` has exactly one caller
        // (`remove_window`), which has exactly one caller (`unrealize`), which collects at
        // its own tail — so a collection at this line can never be the one that releases
        // anything, and a mutation deleting it survived the whole suite. Collection
        // happens where it is provably legal instead: at `about_to_syscall`, and at
        // unrealize's tail. If a runtime removal path is ever added, this is the line to
        // reconsider — with a test that can tell the difference.
        let (mut ins, _h) = self.installer();
        ins.retired.push(window);
    }

    /// Release every retired mapping no accessor still holds, with every lock dropped.
    fn collect_retired(&self) {
        let dead: Vec<Arc<GuestWindow>> = {
            let (mut ins, _h) = self.installer();
            if ins.retired.is_empty() {
                return;
            }
            let (dead, keep): (Vec<_>, Vec<_>) = core::mem::take(&mut ins.retired)
                .into_iter()
                .partition(|w| Arc::strong_count(w) == 1);
            ins.retired = keep;
            dead
        };
        assert_leaf_free("releasing a retired reservation's mapping");
        self.audit
            .window_mappings_released
            .fetch_add(dead.len() as u64, Ordering::SeqCst);
        drop(dead);
    }

    fn bump_topology(&self, v: &mut View) {
        v.topology_generation += 1;
        self.audit
            .topology_generation
            .store(v.topology_generation, Ordering::SeqCst);
    }
}

/// Map a raw-OS refusal onto the port's vocabulary, keeping the error number.
fn host_refused(what: &'static str, e: &RawError) -> VmmError {
    match e {
        RawError::Syscall { errno, .. } => VmmError::HostRefused {
            what,
            errno: *errno,
        },
        _ => VmmError::Unsupported(what),
    }
}

/// Map a hypervisor refusal onto the port's vocabulary.
fn qemu_refused(what: &'static str, e: &HostError) -> VmmError {
    match e {
        HostError::Busy { .. } => VmmError::Unsupported(DISCARD_REQUIRER_PRESENT),
        HostError::Refused { errno, .. } => VmmError::HostRefused {
            what,
            errno: *errno,
        },
        HostError::Unsupported(s) => VmmError::Unsupported(s),
    }
}

// =====================================================================================
// The machine
// =====================================================================================

/// A realized device: the owner of the memory plane and of every region we published.
///
/// Hand out [`QemuMachine::vmm`] handles to threads; keep this to drive the realize-time
/// tier, to feed the topology listener, and to read the [`AuditReport`].
#[derive(Debug)]
pub struct QemuMachine {
    plane: Arc<Plane>,
}

impl QemuMachine {
    /// ★★ §8.1's realize, in its order, with every failure arm unwinding what it created
    /// **before** recording it anywhere.
    ///
    /// The steps that are ours: the runtime floor assertion, the accelerator refusal, the
    /// migration blocker, the discard refusal, the reservations, the overlays, the
    /// listener registration, and the latch. The QOM half — the device type, the BAR
    /// registration, the message-signalled interrupt table — belongs to the C shim and is
    /// stage Q2.
    ///
    /// # Errors
    /// - [`VmmError::Unsupported`] naming [`BELOW_FLOOR`], [`NOT_ACCELERATED`] or
    ///   [`DISCARD_REQUIRER_PRESENT`] for the three realize-time refusals;
    /// - [`VmmError::HostRefused`] if the hypervisor or the OS refused something.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1).
    pub fn realize(cfg: MachineConfig, host: Arc<dyn QemuHost>) -> Result<Self, VmmError> {
        // 1. The runtime floor. The compile-time check is a claim about the headers; this
        //    is a claim about the binary, and neither substitutes for the other (§3.5).
        let (major, minor) = host.version();
        if (major, minor) < VERSION_FLOOR {
            return Err(VmmError::Unsupported(BELOW_FLOOR));
        }
        // 2. Refuse a machine with no hardware accelerator, loudly (§3.4).
        if !host.kvm_enabled() {
            return Err(VmmError::Unsupported(NOT_ACCELERATED));
        }
        // 3. Block migration and checkpoint-restart, before anything is mapped (§8.4).
        let blocker = host
            .migrate_add_blocker("this device forwards to a host GPU through process-local state")
            .map_err(|e| qemu_refused("adding a migration blocker", &e))?;
        // 4. Refuse guest-driven discard, or unwind the blocker (§8.5).
        if let Err(e) = host.ram_block_discard_disable(true) {
            host.migrate_del_blocker(blocker);
            return Err(qemu_refused("disabling guest-driven RAM discard", &e));
        }

        let page = HostPageSize::query();
        let plane = Arc::new(Plane {
            host: Arc::clone(&host),
            page,
            shareable_ram: cfg.shareable_ram,
            bars: cfg.bars.clone(),
            realized: AtomicBool::new(false),
            view: Mutex::new(View::default()),
            installer: Mutex::new(Installer {
                windows: BTreeMap::new(),
                placement_owner: BTreeMap::new(),
                overlays: Vec::new(),
                overlay_slot: BTreeMap::new(),
                traps: cfg.traps.clone(),
                registered_traps: Vec::new(),
                exports: Vec::new(),
                rams: BTreeMap::new(),
                published: Vec::new(),
                blocker: Some(blocker),
                next_region: 1,
                next_slot_id: 1,
                retired: Vec::new(),
            }),
            clock: Mutex::new((Instant::ZERO, DeferQueue::new())),
            audit: Audit::new(),
        });
        let machine = QemuMachine { plane };

        // 5-8. Everything that can fail from here unwinds through `unrealize`, which is
        //      the same teardown the ordinary path takes. A partial realize must leave
        //      the host address space and the hypervisor's blocker exactly as it found
        //      them, and using the real teardown is what stops that from being a second,
        //      less-tested code path.
        if let Err(e) = machine.realize_regions(&cfg) {
            machine.unrealize();
            return Err(e);
        }
        machine.plane.realized.store(true, Ordering::SeqCst);
        Ok(machine)
    }

    fn realize_regions(&self, cfg: &MachineConfig) -> Result<(), VmmError> {
        // The BARs first: a reservation or an overlay declared afterwards REPLACES the
        // device declaration over its own range, which is the layering a flat view has.
        {
            let (mut v, _h) = self.plane.view();
            for b in &cfg.bars {
                v.regions
                    .declare(
                        RamRegionId((1u64 << 63) | b.base),
                        RegionKind::Device,
                        b.base,
                        b.len,
                    )
                    .map_err(|_| VmmError::Unsupported("a BAR that leaves the 64-bit space"))?;
                v.ours.push((b.base, b.len));
            }
        }
        for w in &cfg.windows {
            self.install_ram_window(w.bar, w.gpa, w.len)?;
        }
        for o in &cfg.overlays {
            self.publish_overlay(o)?;
        }
        self.plane
            .host
            .register_listener()
            .map_err(|e| qemu_refused("registering the topology listener", &e))?;
        Ok(())
    }

    /// ★★ The coarse tier, and on this backend it is **realize-only**.
    ///
    /// Creates one reservation and hands it to the hypervisor as a subregion of `bar`.
    /// After realize has latched this refuses with [`TOPOLOGY_AFTER_REALIZE`] — see
    /// finding 1 in the crate docs for why that is the design and not a limitation.
    ///
    /// # Errors
    /// - [`VmmError::Unsupported`] — after realize, or a misaligned/empty request, or a
    ///   range outside the named BAR.
    /// - [`VmmError::HostRefused`] — the OS or the hypervisor refused. **On any of these
    ///   the reservation is dropped and nothing is recorded.**
    ///
    /// # Panics
    /// If called with any ranked lock or any leaf lock held (R1).
    pub fn install_ram_window(
        &self,
        bar: BarId,
        gpa: u64,
        len: u64,
    ) -> Result<RamRegionId, VmmError> {
        let p = &self.plane;
        p.about_to_syscall("installing a reservation");
        p.assert_realize_context()?;

        // ---- PLAN (installer held; pure bookkeeping, no syscall) --------------------
        let (region, placement, bar_offset) = {
            let (mut ins, _h) = p.installer();
            if !geometry::is_aligned(gpa, p.page) || !geometry::is_aligned(len, p.page) || len == 0
            {
                return Err(VmmError::Unsupported(
                    "a reservation whose base or length is not a whole number of host pages",
                ));
            }
            let Some(placement) = p.bars.iter().copied().find(|b| {
                b.bar == bar && gpa >= b.base && gpa.saturating_add(len) <= b.base + b.len
            }) else {
                return Err(VmmError::Unsupported(
                    "a reservation outside the BAR it names",
                ));
            };
            let region = RamRegionId(ins.next_region);
            ins.next_region += 1;
            (region, placement, gpa - placement.base)
        };

        // ---- EXECUTE (every lock dropped) ------------------------------------------
        assert_leaf_free("creating a reservation");
        let outcome = (|| -> Result<Installed, VmmError> {
            let window = Arc::new(GuestWindow::create(len, p.page).map_err(|e| {
                p.audit.host_refusals.fetch_add(1, Ordering::SeqCst);
                host_refused("a reservation mapping", &e)
            })?);
            let ram = if p.shareable_ram {
                let r = Arc::new(SharedRam::create(len).map_err(|e| {
                    p.audit.host_refusals.fetch_add(1, Ordering::SeqCst);
                    host_refused("a shareable guest-RAM backing", &e)
                })?);
                window
                    .place(
                        HostOffset::ZERO,
                        len,
                        RawBacking::SharedFile {
                            fd: r.as_backing_fd(),
                            offset: 0,
                        },
                    )
                    .map_err(|e| {
                        p.audit.host_refusals.fetch_add(1, Ordering::SeqCst);
                        host_refused("placing the guest-RAM backing", &e)
                    })?;
                Some(r)
            } else {
                None
            };
            let mr = p
                .host
                .publish_window("nvkvm-window", placement, bar_offset, len)
                .map_err(|e| {
                    p.audit.host_refusals.fetch_add(1, Ordering::SeqCst);
                    qemu_refused("publishing a reservation", &e)
                })?;
            p.audit.regions_published.fetch_add(1, Ordering::SeqCst);
            Ok(Installed { window, ram, mr })
        })();
        // The partial-failure path: `outcome` owns everything that was created, so
        // dropping it here unmaps the reservation before any of it was recorded.
        let Installed { window, ram, mr } = outcome?;

        // ---- COMMIT (installer, then the view) -------------------------------------
        {
            let (mut ins, _h) = p.installer();
            ins.published.push(mr);
            if let Some(r) = ram {
                ins.rams.insert(region, r);
            }
            ins.windows.insert(
                region,
                Window {
                    gpa,
                    len,
                    window: Arc::clone(&window),
                    placements: BTreeMap::new(),
                },
            );
            Audit::bump(&p.audit.live_windows, &p.audit.peak_windows, 1);
            p.audit.window_bytes.fetch_add(len, Ordering::SeqCst);
        }
        {
            let (mut v, _h) = p.view();
            v.regions
                .declare(region, RegionKind::Ram, gpa, len)
                .map_err(|_| VmmError::Unsupported("a reservation that leaves the 64-bit space"))?;
            v.backings.insert(region, Backing::Ours(window));
        }
        Ok(region)
    }

    /// Publish one realize-time read-native overlay (§5.4).
    fn publish_overlay(&self, spec: &OverlaySpec) -> Result<(), VmmError> {
        let p = &self.plane;
        p.about_to_syscall("publishing a read-native overlay");
        p.assert_realize_context()?;
        let Some(placement) = p.bars.iter().copied().find(|b| {
            b.bar == spec.bar
                && spec.gpa >= b.base
                && spec.gpa.saturating_add(spec.len) <= b.base + b.len
        }) else {
            return Err(VmmError::Unsupported(
                "a read-native overlay outside the BAR it names",
            ));
        };
        if let Some(t) = &spec.write_trap
            && (t.start < spec.gpa || t.end > spec.gpa + spec.len || t.start >= t.end)
        {
            return Err(VmmError::Unsupported(
                "a write-trap sub-range that is not inside the overlay it overlays",
            ));
        }
        assert_leaf_free("publishing a read-native overlay");
        let mr = p
            .host
            .publish_rom_overlay(
                "nvkvm-read-native",
                placement,
                spec.gpa - placement.base,
                spec.len,
            )
            .map_err(|e| {
                p.audit.host_refusals.fetch_add(1, Ordering::SeqCst);
                qemu_refused("publishing a read-native overlay", &e)
            })?;
        p.audit.regions_published.fetch_add(1, Ordering::SeqCst);
        let region = {
            let (mut ins, _h) = p.installer();
            let region = RamRegionId(ins.next_region);
            ins.next_region += 1;
            ins.published.push(mr);
            ins.overlays.push(Overlay {
                spec: spec.clone(),
                mr,
                region,
                claimed_by: None,
            });
            region
        };
        // ★ Unclaimed, it is declared a DEVICE: the hypervisor is serving reads out of it
        // already, but nothing has told us what should be in it, so a `gpa_write` must
        // refuse by name rather than scribble on a register page. Claiming it through
        // `map_read_native` is what re-declares it RAM — finding 3 in the crate docs.
        let (mut v, _h) = p.view();
        v.regions
            .declare(region, RegionKind::Device, spec.gpa, spec.len)
            .map_err(|_| VmmError::Unsupported("an overlay that leaves the 64-bit space"))
    }

    /// A `Vmm` handle. Cheap to clone; hand one to each thread.
    #[must_use]
    pub fn vmm(&self) -> QemuVmm {
        QemuVmm {
            plane: Arc::clone(&self.plane),
        }
    }

    /// The host page size this machine's geometry is expressed in.
    #[must_use]
    pub fn page_size(&self) -> HostPageSize {
        self.plane.page
    }

    /// The audit / conservation ledger.
    #[must_use]
    pub fn audit(&self) -> AuditReport {
        self.plane.audit.report()
    }

    /// Create a host backing an isolate would have handed us, and name it as the port
    /// does.
    ///
    /// # Errors
    /// [`VmmError::HostRefused`] if the host refuses the backing.
    ///
    /// # Panics
    /// If called with any ranked lock or any leaf lock held (R1).
    pub fn register_backing(&self, len: u64) -> Result<HostRegion, VmmError> {
        let p = &self.plane;
        p.about_to_syscall("creating a host backing");
        let ram = SharedRam::create(len).map_err(|e| {
            p.audit.host_refusals.fetch_add(1, Ordering::SeqCst);
            host_refused("a host backing", &e)
        })?;
        let dup = ram
            .dup_for_export()
            .map_err(|e| host_refused("duplicating a host backing", &e))?;
        let (mut ins, _h) = p.installer();
        ins.exports.push(dup);
        Ok(HostRegion {
            id: ins.exports.len() as u64 - 1,
            offset: 0,
        })
    }

    /// Query the guest-physical region map directly, without touching a backing.
    ///
    /// Exists for the reason the other backend's does: nothing else can tell *"the region
    /// was undeclared"* from *"the region is still declared RAM but its backing is gone"*,
    /// and both arrive at `gpa_read` as [`VmmError::BadGpa`].
    ///
    /// # Errors
    /// As [`GuestRamMap::resolve`].
    pub fn resolve_region(&self, gpa: u64, len: u64) -> Result<kayfabe_vmm::RamSpan, VmmError> {
        let (v, _h) = self.plane.view();
        v.regions.resolve(gpa, len)
    }

    /// Advance the virtual clock, returning every deferred event that became due.
    pub fn advance(&self, d: Duration) -> Vec<CoreEvent> {
        let mut c = self.plane.clock.lock().expect("clock");
        c.0 = c.0.advanced(d);
        let now = c.0;
        c.1.due(now)
    }

    // --- the topology listener (§5.2) ---------------------------------------------

    /// ★ The listener's add callback. Arrives holding the VMM's global lock, so the only
    /// thing it may do is a bounded update of our leaf map (§4.2's table).
    ///
    /// Lock order on this path is `global -> leaf(view)`, the safe direction; the reverse
    /// never occurs because no leaf critical section takes the global lock.
    ///
    /// # Errors
    /// [`VmmError::Unsupported`] naming [`FOREIGN_OVERLAPS_OURS`] if the reported section
    /// overlaps a region this device published — the two declaration sources would
    /// otherwise race to own the same guest-physical range, and the loser would be
    /// whichever ran second.
    /// [`VmmError::HostRefused`] if the reference could not be taken.
    pub fn region_add(&self, s: SectionDesc) -> Result<(), VmmError> {
        let p = &self.plane;
        let kind = classify::classify(&s.facts);
        // ★ The overlap check happens BEFORE any reference is taken, so a refusal leaves
        // the hypervisor's reference count exactly as it found it. It is checked against
        // the ranges WE published, not against the map — a previously reported foreign
        // section may legitimately be re-reported, and the map cannot tell the two
        // sources apart.
        {
            let (v, _h) = p.view();
            let end = u128::from(s.gpa) + u128::from(s.len.max(1));
            if v.ours.iter().any(|(b, l)| {
                u128::from(s.gpa) < u128::from(*b) + u128::from(*l) && u128::from(*b) < end
            }) {
                return Err(VmmError::Unsupported(FOREIGN_OVERLAPS_OURS));
            }
        }
        let mr = if kind == RegionKind::Ram {
            p.host
                .ref_region(s.mr)
                .map_err(|e| qemu_refused("taking a reference to a reported region", &e))?;
            Some(s.mr)
        } else {
            None
        };
        let (mut v, _h) = p.view();
        let region = RamRegionId(0x4000_0000_0000_0000 | s.gpa);
        v.regions.declare(region, kind, s.gpa, s.len).map_err(|_| {
            VmmError::Unsupported("a reported section that leaves the 64-bit space")
        })?;
        if let Some(mr) = mr {
            v.backings.insert(
                region,
                Backing::HostOwned {
                    mr,
                    region_off: s.offset_within_region,
                },
            );
        }
        v.foreign.insert(s.gpa, (region, s.len, mr));
        p.bump_topology(&mut v);
        p.audit.topology_adds.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// ★★ The listener's delete callback, in §5.2's order — **undeclare first**, drop the
    /// backing, and only then release the reference.
    ///
    /// The release happens **after** the leaf lock is dropped and **on this thread**,
    /// which is the whole of finding 2 in the crate docs: because a copy out of a foreign
    /// region runs *inside* the view lock, undeclaring under that lock means no accessor
    /// can be holding this region when the reference falls — so the release never has to
    /// leave the one context that is allowed to run a finalizer.
    pub fn region_del(&self, gpa: u64, len: u64) {
        let p = &self.plane;
        let mr = {
            let (mut v, _h) = p.view();
            let Some((region, declared_len, mr)) = v.foreign.remove(&gpa) else {
                return;
            };
            v.regions.undeclare(gpa, declared_len.max(len));
            v.backings.remove(&region);
            p.bump_topology(&mut v);
            mr
        };
        if let Some(mr) = mr {
            p.host.unref_region(mr);
        }
        p.audit.topology_dels.fetch_add(1, Ordering::SeqCst);
    }

    // --- teardown ------------------------------------------------------------------

    /// ★ §8.3's unrealize: clear the latch, release every reservation, unpublish every
    /// region we published, and withdraw the migration blocker.
    ///
    /// The hypervisor frees **nothing** of ours — it never took ownership of the
    /// reservation — so this step is not optional and there is no backstop for it.
    ///
    /// # Panics
    /// If called with any ranked lock or any leaf lock held (R1).
    pub fn unrealize(&self) {
        let p = &self.plane;
        p.about_to_syscall("unrealize");
        // Clearing the latch first is what makes the teardown's own topology calls legal
        // under §4.3 — unrealize is the other context the hypervisor enters holding its
        // global lock.
        p.realized.store(false, Ordering::SeqCst);

        let regions: Vec<RamRegionId> = {
            let (ins, _h) = p.installer();
            ins.windows.keys().copied().collect()
        };
        for r in regions {
            let _ = self.remove_window(r);
        }
        let (published, blocker) = {
            let (mut ins, _h) = p.installer();
            ins.overlays.clear();
            ins.overlay_slot.clear();
            (core::mem::take(&mut ins.published), ins.blocker.take())
        };
        {
            let (mut v, _h) = p.view();
            *v = View::default();
        }
        assert_leaf_free("unpublishing the device's regions");
        for mr in published {
            p.host.unpublish(mr);
        }
        if let Some(b) = blocker {
            p.host.migrate_del_blocker(b);
        }
        let _ = p.host.ram_block_discard_disable(false);
        p.collect_retired();
    }

    /// Remove one reservation: the guest-physical range stops resolving **first**, then
    /// the mapping is retired.
    ///
    /// # Errors
    /// [`VmmError::BadSlot`] if the region is unknown.
    fn remove_window(&self, region: RamRegionId) -> Result<(), VmmError> {
        let p = &self.plane;
        p.about_to_syscall("removing a reservation");
        // ★ Releasing a reservation is half of a topology transaction — the other half is
        // the `unpublish` that unrealize performs right after — so it is latched exactly
        // like its creation. This is also what makes `unrealize`'s FIRST line load-bearing
        // rather than decorative: clear the latch or nothing below it happens.
        p.assert_realize_context()?;
        let taken = {
            let (mut ins, _h) = p.installer();
            let Some(w) = ins.windows.remove(&region) else {
                return Err(VmmError::BadSlot(SlotId(region.0)));
            };
            ins.placement_owner.retain(|_, r| *r != region);
            ins.rams.remove(&region);
            Audit::bump(&p.audit.live_windows, &p.audit.peak_windows, -1);
            Audit::bump(
                &p.audit.live_placements,
                &p.audit.peak_placements,
                -(w.placements.len() as i64),
            );
            p.audit.window_bytes.fetch_sub(w.len, Ordering::SeqCst);
            w
        };
        {
            let (mut v, _h) = p.view();
            v.regions.undeclare(taken.gpa, taken.len);
            v.backings.remove(&region);
        }
        assert_leaf_free("retiring a reservation's mapping");
        p.retire(taken.window);
        Ok(())
    }
}

// =====================================================================================
// The Vmm impl
// =====================================================================================

/// A per-thread handle onto a [`QemuMachine`]'s memory plane.
#[derive(Debug, Clone)]
pub struct QemuVmm {
    plane: Arc<Plane>,
}

/// The copy a host-owned resolution performs **inside** the view lock: the host, the
/// region, and the byte offset within it. Named because the whole point of finding 2 is
/// that this call happens at exactly one place and under exactly one lock.
type CopyIntoHostOwned<'a> = dyn FnMut(&dyn QemuHost, MrHandle, u64) -> Result<(), VmmError> + 'a;

/// What a resolved guest-physical access copies against.
enum Resolved {
    /// Our reservation: the copy runs outside the lock, held alive by the `Arc`.
    Ours(Arc<GuestWindow>, u64),
    /// A host-owned region: the copy already ran, inside the lock.
    Done,
}

impl QemuVmm {
    /// The audit / conservation ledger of the machine behind this handle.
    #[must_use]
    pub fn audit(&self) -> AuditReport {
        self.plane.audit.report()
    }

    /// ★★ Resolve, and for a host-owned region **copy as well** — the two are indivisible
    /// on that path by construction (finding 2).
    ///
    /// `copy` is applied under the view lock for [`Backing::HostOwned`] and returns
    /// [`Resolved::Ours`] for [`Backing::Ours`], whose copy the caller then performs
    /// outside the lock.
    fn resolve_and_maybe_copy(
        &self,
        gpa: u64,
        len: u64,
        copy: &mut CopyIntoHostOwned<'_>,
    ) -> Result<Resolved, VmmError> {
        let p = &self.plane;
        Audit::note_ranked(
            &p.audit.accessor_depth_min,
            &p.audit.accessor_depth_max,
            kayfabe_util::lockwitness::held_depth(),
        );
        let (v, held) = p.view();
        let span = v.regions.resolve(gpa, len).inspect_err(|_| {
            p.audit.accesses_refused.fetch_add(1, Ordering::SeqCst);
        })?;
        let backing = v.backings.get(&span.region).cloned().ok_or_else(|| {
            p.audit.accesses_refused.fetch_add(1, Ordering::SeqCst);
            VmmError::BadGpa { gpa }
        })?;
        match backing {
            Backing::Ours(w) => {
                let offset = span.offset;
                // Released EXPLICITLY, before the caller copies. Writing it out is what
                // makes the property visible at the line that owns it, and what a
                // bite-check can neuter — `own_copy_leaf_depth_max` is the assertion.
                core::mem::drop(v);
                core::mem::drop(held);
                Ok(Resolved::Ours(w, offset))
            }
            Backing::HostOwned { mr, region_off } => {
                // ★ INSIDE the lock, on purpose. The leaf witness must read >= 1 here,
                // and `host_copy_leaf_depth_min` is the assertion that it does.
                p.audit
                    .host_copy_leaf_min
                    .fetch_min(leafwitness::depth(), Ordering::SeqCst);
                p.audit.accesses_served.fetch_add(1, Ordering::SeqCst);
                let r = copy(p.host.as_ref(), mr, region_off + span.offset);
                core::mem::drop(v);
                core::mem::drop(held);
                r.map(|()| Resolved::Done)
            }
        }
    }

    fn about_to_copy_ours(&self) {
        self.plane
            .audit
            .own_copy_leaf_max
            .fetch_max(leafwitness::depth(), Ordering::SeqCst);
        self.plane
            .audit
            .accesses_served
            .fetch_add(1, Ordering::SeqCst);
    }
}

impl Vmm for QemuVmm {
    fn gpa_read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), VmmError> {
        let len = buf.len() as u64;
        // The closure borrows `buf` mutably, so the reservation arm cannot also use it —
        // hence the two-arm shape rather than one copy callback.
        let mut sink: Option<&mut [u8]> = Some(buf);
        let resolved = self.resolve_and_maybe_copy(gpa, len, &mut |host, mr, off| {
            let dst = sink.take().expect("the copy callback runs at most once");
            if dst.is_empty() {
                return Ok(());
            }
            host.read_region(mr, off, dst)
                .map_err(|e| qemu_refused("a guest-physical read", &e))
        })?;
        match resolved {
            Resolved::Done => Ok(()),
            Resolved::Ours(window, offset) => {
                let dst = sink
                    .take()
                    .expect("the reservation arm still owns the buffer");
                self.about_to_copy_ours();
                if dst.is_empty() {
                    return Ok(());
                }
                window
                    .read_into(HostOffset::new(offset), dst)
                    .map_err(|e| host_refused("a guest-physical read", &e))
            }
        }
    }

    fn gpa_write(&mut self, gpa: u64, buf: &[u8]) -> Result<(), VmmError> {
        let len = buf.len() as u64;
        let resolved = self.resolve_and_maybe_copy(gpa, len, &mut |host, mr, off| {
            if buf.is_empty() {
                return Ok(());
            }
            host.write_region(mr, off, buf)
                .map_err(|e| qemu_refused("a guest-physical write", &e))
        })?;
        match resolved {
            Resolved::Done => Ok(()),
            Resolved::Ours(window, offset) => {
                self.about_to_copy_ours();
                if buf.is_empty() {
                    return Ok(());
                }
                window
                    .write_from(HostOffset::new(offset), buf)
                    .map_err(|e| host_refused("a guest-physical write", &e))
            }
        }
    }

    /// ★★ The fine tier — a `MAP_FIXED` placement inside an already-published
    /// reservation. **No hypervisor call at all** (§5.4), which is why it is the one
    /// memory-plane op that stays legal after realize has latched.
    fn map_guest(
        &mut self,
        gpa: u64,
        len: u64,
        backing: HostRegion,
        prot: Prot,
    ) -> Result<SlotId, VmmError> {
        let p = &self.plane;
        p.about_to_syscall("map_guest (a MAP_FIXED placement inside a reservation)");

        // ---- PLAN ------------------------------------------------------------------
        let (slot_id, region, window, offset) = {
            let (mut ins, _h) = p.installer();
            if prot == Prot::ReadOnly {
                return Err(VmmError::Unsupported(PER_OBJECT_PROTECTION));
            }
            let Some((region, off)) = ins
                .windows
                .iter()
                .find(|(_, w)| gpa >= w.gpa && gpa.saturating_add(len) <= w.gpa + w.len)
                .map(|(r, w)| (*r, gpa - w.gpa))
            else {
                return Err(VmmError::BadGpa { gpa });
            };
            let w = ins.windows.get(&region).expect("just found");
            if w.placements
                .values()
                .any(|(o, l)| off < o + l && *o < off + len)
            {
                return Err(VmmError::Unsupported(
                    "a placement overlapping one already live in the same reservation",
                ));
            }
            let window = Arc::clone(&w.window);
            let slot_id = SlotId(ins.next_slot_id);
            ins.next_slot_id += 1;
            (slot_id, region, window, off)
        };

        // ---- EXECUTE (every lock dropped) ------------------------------------------
        assert_leaf_free("map_guest's MAP_FIXED");
        {
            let (ins, _h) = p.installer();
            let fd = usize::try_from(backing.id)
                .ok()
                .and_then(|i| ins.exports.get(i))
                .map(std::os::fd::AsFd::as_fd);
            let Some(fd) = fd else {
                return Err(VmmError::Unsupported(
                    "a host backing id this backend never minted",
                ));
            };
            // The borrow must not outlive the lock and the placement is a syscall, so the
            // descriptor is duplicated into an owned handle and the lock is dropped first.
            let owned = std::fs::File::from(
                dup_owned(fd).map_err(|e| host_refused("duplicating a host backing", &e))?,
            );
            drop(ins);
            drop(_h);
            assert_leaf_free("map_guest's MAP_FIXED");
            window
                .place(
                    HostOffset::new(offset),
                    len,
                    RawBacking::SharedFile {
                        fd: std::os::fd::AsFd::as_fd(&owned),
                        offset: backing.offset,
                    },
                )
                .map_err(|e| {
                    p.audit.host_refusals.fetch_add(1, Ordering::SeqCst);
                    host_refused("a placement inside a reservation", &e)
                })?;
        }
        p.audit.placements_made.fetch_add(1, Ordering::SeqCst);

        // ---- COMMIT + R5 -----------------------------------------------------------
        //
        // ★★ R5's token here is the reservation's **presence**, not a version counter, and
        // that is a bite-check finding rather than a simplification. The ported shape
        // carried a per-window `generation` compared against the one read at plan time —
        // and it is **unfalsifiable**: a `Window`'s generation is written once at install
        // and never mutated, a removal takes the whole `Window` out of the map, and a
        // `RamRegionId` is never reused. So the compare can be `true` unconditionally and
        // nothing changes; a mutation to exactly that survived the entire suite. A safety
        // check nobody can make fail is worse than no check, because it reads as
        // protection — so the token is the lookup, which a concurrent teardown really can
        // and does change. (The same vacuity is present in `kayfabe-vmm-kvm`, which is
        // where this shape was ported from; it is recorded here rather than fixed there.)
        let (mut ins, _h) = p.installer();
        match ins.windows.get_mut(&region) {
            Some(w) => {
                w.placements.insert(slot_id, (offset, len));
                ins.placement_owner.insert(slot_id, region);
                Audit::bump(&p.audit.live_placements, &p.audit.peak_placements, 1);
                drop(_h);
                drop(ins);
                Ok(slot_id)
            }
            _ => {
                drop(_h);
                drop(ins);
                p.audit.r5_failures.fetch_add(1, Ordering::SeqCst);
                Err(VmmError::BadGpa { gpa })
            }
        }
    }

    /// Remove a mapping — a fine-tier placement is **restored** to anonymous backing,
    /// never unmapped, and a read-native slot **un-claims** its overlay without
    /// unpublishing it (§4.3: unpublishing is a topology transaction).
    fn unmap_guest(&mut self, slot: SlotId) -> Result<(), VmmError> {
        let p = &self.plane;
        p.about_to_syscall("unmap_guest");

        enum Plan {
            Placement(Arc<GuestWindow>, u64, u64),
            Overlay(RamRegionId, u64, u64),
        }
        let plan = {
            let (mut ins, _h) = p.installer();
            if let Some(i) = ins.overlay_slot.remove(&slot) {
                let o = &mut ins.overlays[i];
                o.claimed_by = None;
                Plan::Overlay(o.region, o.spec.gpa, o.spec.len)
            } else if let Some(region) = ins.placement_owner.remove(&slot) {
                let w = ins
                    .windows
                    .get_mut(&region)
                    .ok_or(VmmError::BadSlot(slot))?;
                let (off, len) = w.placements.remove(&slot).ok_or(VmmError::BadSlot(slot))?;
                Plan::Placement(Arc::clone(&w.window), off, len)
            } else {
                return Err(VmmError::BadSlot(slot));
            }
        };
        match plan {
            Plan::Overlay(region, gpa, len) => {
                // Back to a device declaration: the hypervisor still serves reads out of
                // the overlay, but nothing of ours is keeping it current any more, so a
                // `gpa_write` must refuse by name again.
                let (mut v, _h) = p.view();
                v.backings.remove(&region);
                v.regions
                    .declare(region, RegionKind::Device, gpa, len)
                    .map_err(|_| {
                        VmmError::Unsupported("an overlay that leaves the 64-bit space")
                    })?;
                drop(v);
                drop(_h);
                p.audit.overlays_claimed.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            }
            Plan::Placement(window, off, len) => {
                assert_leaf_free("unmap_guest's restore");
                window
                    .restore(HostOffset::new(off), len)
                    .map_err(|e| host_refused("restoring anonymous backing", &e))?;
                Audit::bump(&p.audit.live_placements, &p.audit.peak_placements, -1);
                Ok(())
            }
        }
    }

    /// ★ Register a trapped range — and **validate it against the realize-time table**.
    ///
    /// This is the Rust half of §3.3's clause (c). The C shim owns the literal region
    /// table and the realize-time self-check that every row of it is marked; what nothing
    /// else covers is the core registering a trap over a range the table never named,
    /// which would be a trap that exists only in our bookkeeping. It is refused by name.
    fn set_trap(&mut self, bar: BarId, range: Range<u64>, mode: TrapMode) -> Result<(), VmmError> {
        let p = &self.plane;
        p.about_to_syscall("set_trap");
        if range.start >= range.end {
            return Err(VmmError::Unsupported("an empty or inverted trap range"));
        }
        let Some(b) = p.bars.iter().find(|b| b.bar == bar) else {
            return Err(VmmError::Unsupported(
                "a BAR this machine was not realized with",
            ));
        };
        if range.end > b.len {
            return Err(VmmError::Unsupported("a trap range outside its BAR"));
        }
        let (mut ins, _h) = p.installer();
        if !ins.traps.iter().any(|t| {
            t.bar == bar
                && t.mode == mode
                && t.range.start <= range.start
                && range.end <= t.range.end
        }) {
            return Err(VmmError::Unsupported(TRAP_OUTSIDE_THE_REALIZED_TABLE));
        }
        ins.registered_traps.push((bar, range, mode));
        Ok(())
    }

    /// ★ The one in-lock-legal foreign call: one descriptor write (§7).
    ///
    /// It must never become a notify call that takes the VMM's global lock — that is a
    /// normative requirement on [`QemuHost::signal_msix`], not something this line can
    /// enforce, and it is stated on the trait method for that reason.
    fn raise_irq(&mut self, irq: IrqSpec) -> Result<(), VmmError> {
        match irq {
            IrqSpec::Msix(v) => {
                self.plane
                    .host
                    .signal_msix(v)
                    .map_err(|e| qemu_refused("raising a message-signalled vector", &e))?;
                self.plane.audit.irqs_raised.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
            IrqSpec::IntxLevel(_) => Err(VmmError::Unsupported(NO_LEGACY_INTX)),
        }
    }

    fn export_ram(&mut self, slice: Option<Range<u64>>) -> Result<RamHandle, VmmError> {
        let p = &self.plane;
        p.about_to_syscall("export_ram");
        if !p.shareable_ram {
            return Err(VmmError::Unsupported(NO_SHARED_BACKING));
        }
        let ram = {
            let (ins, _h) = p.installer();
            let want = slice.clone();
            ins.windows
                .iter()
                .find(|(_, w)| match &want {
                    None => true,
                    Some(s) => s.start >= w.gpa && s.end <= w.gpa + w.len,
                })
                .and_then(|(r, _)| ins.rams.get(r).cloned())
        };
        let Some(ram) = ram else {
            return Err(VmmError::Unsupported(
                "no shareable reservation covers the requested slice",
            ));
        };
        assert_leaf_free("export_ram's descriptor duplication");
        let dup = ram
            .dup_for_export()
            .map_err(|e| host_refused("duplicating guest RAM", &e))?;
        let (mut ins, _h) = p.installer();
        ins.exports.push(dup);
        Ok(RamHandle {
            token: ins.exports.len() as u64 - 1,
            covers: slice,
        })
    }

    fn defer(&mut self, after: Duration, event: CoreEvent) {
        let mut c = self.plane.clock.lock().expect("clock");
        let now = c.0;
        c.1.push(now, after, event);
    }

    fn now(&self) -> Instant {
        self.plane.clock.lock().expect("clock").0
    }

    /// ★★ Claim a realize-time read-native overlay — see findings 1, 3 and 4 in the crate
    /// docs. This creates nothing: on this backend an overlay is a topology transaction
    /// and therefore realize-time configuration.
    ///
    /// The four refusals are deliberately distinguishable, because they are near
    /// neighbours and an operator must be able to tell a configuration mistake
    /// ([`NO_SUCH_OVERLAY`], [`OVERLAY_SHAPE_MISMATCH`]) from a lifecycle bug
    /// ([`OVERLAY_ALREADY_CLAIMED`]) from a portability difference
    /// ([`OVERLAY_BACKING_UNPLACEABLE`]).
    fn map_read_native(
        &mut self,
        gpa: u64,
        len: u64,
        backing: HostRegion,
        write_trap: Option<Range<u64>>,
    ) -> Result<SlotId, VmmError> {
        let p = &self.plane;
        p.about_to_syscall("map_read_native");
        // Finding 4: the overlay's RAM belongs to the hypervisor and nothing may be
        // placed into it, so a caller-supplied backing cannot mean here what it means on
        // a backend that MAP_FIXEDs it. `u64::MAX` is the port's established "fill it
        // from nothing" sentinel.
        if backing.id != u64::MAX {
            return Err(VmmError::Unsupported(OVERLAY_BACKING_UNPLACEABLE));
        }

        let (mut ins, _h) = p.installer();
        let Some(i) = ins.overlays.iter().position(|o| o.spec.gpa == gpa) else {
            return Err(VmmError::Unsupported(NO_SUCH_OVERLAY));
        };
        if ins.overlays[i].spec.len != len || ins.overlays[i].spec.write_trap != write_trap {
            return Err(VmmError::Unsupported(OVERLAY_SHAPE_MISMATCH));
        }
        if ins.overlays[i].claimed_by.is_some() {
            return Err(VmmError::Unsupported(OVERLAY_ALREADY_CLAIMED));
        }
        let slot_id = SlotId(ins.next_slot_id);
        ins.next_slot_id += 1;
        ins.overlays[i].claimed_by = Some(slot_id);
        ins.overlay_slot.insert(slot_id, i);
        let (region, mr) = (ins.overlays[i].region, ins.overlays[i].mr);
        drop(ins);
        drop(_h);

        // Finding 3: RAM, backed by the hypervisor's own allocation, so the core can keep
        // it current through the same `gpa_write` it uses everywhere else.
        let (mut v, _h) = p.view();
        v.regions
            .declare(region, RegionKind::Ram, gpa, len)
            .map_err(|_| VmmError::Unsupported("an overlay that leaves the 64-bit space"))?;
        v.backings
            .insert(region, Backing::HostOwned { mr, region_off: 0 });
        drop(v);
        drop(_h);
        p.audit.overlays_claimed.fetch_add(1, Ordering::SeqCst);
        Ok(slot_id)
    }
}

/// Duplicate a borrowed descriptor into an owned one, so the borrow — and the lock it
/// came from — can be released **before** the syscall that uses it runs.
fn dup_owned(fd: std::os::fd::BorrowedFd<'_>) -> Result<std::os::fd::OwnedFd, RawError> {
    fd.try_clone_to_owned().map_err(|e| RawError::Syscall {
        call: "dup",
        errno: e.raw_os_error(),
    })
}

kayfabe_util::assert_send_sync!(QemuVmm, QemuMachine, AuditReport);
