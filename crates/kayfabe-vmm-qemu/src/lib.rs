//! # `kayfabe-vmm-qemu` — the QEMU adapter's **logic half**, and all of it
//!
//! `l2_qemu_adapter.md` stages **Q1–Q5**, under the memory-plane decision that supersedes
//! §5.4: **`host_execution_plane.md` §1**. No guest, no GPU. Every hypervisor effect
//! crosses [`host::QemuHost`]; every *kernel* effect crosses [`slots::SlotPlane`].
//! `forbid(unsafe_code)`, like [`kayfabe_vmm_kvm`], and for the same reason: the unsound
//! surface belongs in one auditable crate per host axis, never in the logic.
//!
//! [`kayfabe_vmm_kvm`]: https://docs.rs/kayfabe-vmm-kvm
//!
//! ## ★★★ The shape, in one paragraph
//!
//! The hypervisor **reserves** the guest-physical window as a pure-MMIO BAR and does not
//! back it. We install our own memslots over that range with the kernel's own ioctl, so an
//! access with a slot resolves from the slot and never exits, and an access without one
//! exits to us. Passthrough is an ordinary read-write slot; read-native is a slot with the
//! kernel's read-only flag (reads native, writes exit — *that is* the read-native
//! semantic); observe-everything is the **absence** of a slot. Nothing is ever added to the
//! hypervisor's region tree, in realize or out of it.
//!
//! ## ★★ The four things this build does differently from the C artifact
//!
//! Each is a finding with a mechanism here, not a note. They are collected at the top
//! because a reader who takes the C as precedent will not otherwise find them.
//!
//! ### 1. The BAR base is **latched and re-checked**, and a move is loud
//!
//! `C: nvkvm_mmap_host.c:189-193` caches the window base once, and there is **no
//! BAR-change hook anywhere in the C's device**. If the guest moves the BAR after the slot
//! is installed, the hypervisor's own region follows and our memslot does not: the *old*
//! guest-physical range keeps working — and becomes reassignable to another BAR — while the
//! *new* one reads zeros. Silently. It never fired only because the install is lazy, the
//! transient unmap during BAR sizing is separately guarded, and Linux honours firmware
//! assignment; none of those is a mechanism.
//!
//! Here the base is latched at the first install and **asserted unchanged on every
//! resolve** ([`QemuMachine::note_bar_mapping`] is the tripwire, [`QemuMachine::bar_move_requested`]
//! is the refusal, and [`BAR_MOVED_UNDER_US`] is what an access gets once either fires).
//! The poison is **sticky**: a BAR that moved does not start working again if it moves back,
//! because in between it may have been another device's.
//!
//! ### 2. Slot numbers descend from the kernel's ceiling
//!
//! The C allocates upward from a hardcoded base of 64, on a convention enforced by nothing.
//! See [`slots::SlotAllocator`] for why a number collision is not an error but a silent
//! **replace**, and why counting down inverts the argument.
//!
//! ### 3. ★★★ The window is guest-physical-address-space visible but hypervisor-**opaque**
//!
//! The shadowing is the kernel's, not the hypervisor's. Anything that reaches the window
//! through the hypervisor's flat view hits the reservation BAR's stub read/write ops and
//! gets **zeros**, silently. So [`Vmm::gpa_read`]/[`Vmm::gpa_write`] must **not** route a
//! window address through the hypervisor — and they do not: the window is served from our
//! own mapping, by our own offset arithmetic, and the lookup that does it runs **before**
//! the region map is consulted at all.
//!
//! The region map then declares the window's BAR [`RegionKind::Device`] — which is *correct*
//! rather than a compromise, because that is what the hypervisor's flat view says it is. The
//! two together are a layered failure: if the window lookup were ever bypassed, the access
//! does not fall through to a memcpy of zeros, it refuses with [`VmmError::NonRamGpa`].
//! There is a test that removes the window and leaves the declaration standing precisely to
//! watch that happen.
//!
//! ### 4. The tiering is new construction and is gated as such
//!
//! Mode 2 in the C has never had a memslot at all, and the Mode-1 window that does has one,
//! read-write, forever — `readonly` is a dead parameter there. So there is no precedent to
//! port for the read-only tier or for a mixed layout; [`slots`] carries the taxonomy and
//! the suite gates it against a real kernel as well as a double.
//!
//! ## ★★ What §1 DELETED from this crate, which is a finding in its own right
//!
//! `f0053ef` built this adapter with a coarse tier that was **realize-only**, because
//! publishing a region was a topology transaction and §4.3 confines those to
//! realize/unrealize. That made [`Vmm::map_read_native`] a method that could only *claim* an
//! overlay realize had already created, with four refusals attached to the impossibility.
//!
//! Under §1 there is no topology transaction left, so **all of that is gone**: installing a
//! window is a kernel call, legal at any time, and `map_read_native` creates one exactly as
//! the sibling backend does. The `TOPOLOGY_AFTER_REALIZE` refusal lost its subject; what
//! replaces it is [`MEMORY_PLANE_AFTER_UNREALIZE`], which is a real and reachable lifecycle
//! error rather than a design constraint dressed as one.
//!
//! ## The lock ladder
//!
//! ```text
//!   the VMM's global lock   (foreign, unrankable)  ── outermost, on the paths that have it
//!     │
//!   rank 0 / rank 1         the core's ranked locks
//!     │
//!   leaf(bars) · leaf(view) · leaf(installer)      ── ours, unranked, leafwitness
//! ```
//!
//! Never the reverse. No leaf critical section calls the hypervisor **except**
//! [`host::QemuHost::read_region`]/[`host::QemuHost::write_region`], which are normatively a
//! bounded memcpy and take nothing. [`host::QemuHost::bar_base`] is called on the hot path
//! but **outside** every leaf, before the view is taken.
//!
//! ★★ **This adapter contains ZERO calls to `bql_lock`** (§4.3), and under
//! `host_execution_plane.md` §1 that is no longer a discipline — it is arithmetic. The only
//! reason to take the hypervisor's global lock would be to mutate its region tree, and this
//! adapter never mutates it: the window is a reservation the hypervisor made once, at
//! realize, in its own C shim, and every memory-plane operation after that is a call to the
//! **kernel**. A grep for `bql_lock` over this crate finds this sentence and nothing else,
//! which is the intended result.

#![allow(clippy::module_name_repetitions)]

pub mod classify;
pub mod host;
pub mod mock_host;
pub mod slots;

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

use host::{BarPlacement, BlockerId, HostError, MrHandle, QemuHost, SectionDesc};
use slots::{LiveSlot, SlotAllocator, SlotPlane, Span, Tier};

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
///
/// ★★ Under `host_execution_plane.md` §1 this stopped being only a performance argument.
/// The memory plane **is** the accelerator's memslot table; without the accelerator there
/// is no table to install into and the device has no data path at all.
pub const NOT_ACCELERATED: &str = "this device requires the hardware accelerator; the memory plane IS its memslot table, \
     and without it every trapped access also runs under the VMM's global lock — a measured \
     5.3x amplification and not a slow mode";

/// ★ Lifecycle refusal: a memory-plane operation after the device was unrealized.
pub const MEMORY_PLANE_AFTER_UNREALIZE: &str = "a memory-plane operation on a device that has been unrealized; its reservations are \
     gone, its memslots are cleared, and installing another would put a live slot in a \
     machine nothing is going to tear it down from";

/// ★★★ The BAR moved after we latched it — [`crate::VERSION_FLOOR`]'s neighbour in
/// importance, and the C's latent bug made loud.
pub const BAR_MOVED_UNDER_US: &str = "the reservation BAR moved after a memslot was installed into it; the hypervisor's own \
     region followed the guest and our memslot did not, so the OLD guest-physical range is \
     still live and now reassignable, and the NEW one reads zeros. Every access into this \
     BAR is refused from here on";

/// Realize/install refusal: the BAR has no base yet.
pub const BAR_NOT_PROGRAMMED: &str = "a window in a BAR the guest has not programmed yet; installing at a fallback base now \
     would cache the wrong one, which is exactly what the C guards against at its own \
     install";

/// Install refusal: the BAR is not where the configuration says it is.
pub const BAR_NOT_AT_ITS_DECLARED_BASE: &str = "the BAR is programmed somewhere other than the base this device was realized with; \
     one of the two is stale and installing a memslot would commit to the wrong one";

/// ★★★ Install refusal: the BAR is not a pure-MMIO reservation.
pub const WINDOW_IN_A_BACKED_BAR: &str = "a window in a BAR the hypervisor BACKS; the whole safety argument for installing our \
     own memslots is that the reservation's region never sets the RAM flag, so the \
     accelerator's listener early-returns for it and creates no slot of its own. A backed \
     BAR gets a hypervisor-managed slot over the same range, and only one of the two wins";

/// `map_guest` refusal: per-object protection (§6.7 item 4).
pub const PER_OBJECT_PROTECTION: &str = "per-object read-only protection inside a read-write window; protection is a SLOT \
     property and therefore a WINDOW property, so place the object in a read-native window \
     instead";

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

/// ★★★ `install_window` refusal: a reservation over a guest-physical range one of ours
/// already covers.
///
/// # Why this is ours to refuse and not the kernel's
///
/// It looks like the kernel's job — `KVM_SET_USER_MEMORY_REGION` answers `EEXIST` for an
/// overlapping range, and that is what this path relied on. But the kernel only ever sees
/// the spans that get a **memslot**, and a [`Tier::Observe`] span deliberately gets none:
/// it is the tier whose whole definition is "no slot at all". So a reservation whose
/// overlapping part is observe-tiered — on either side — is installed with the kernel never
/// asked, and two of our windows then claim the same guest-physical range. `resolve` picks
/// whichever the `BTreeMap` iteration order reaches first, and the loser's memslots stay
/// live underneath.
///
/// The kernel is also the wrong place to ask on principle: a refusal that arrives from the
/// execute phase has already `mmap`ed a reservation and installed some of its slots, and
/// unwinding that is strictly more expensive than not starting.
pub const WINDOW_OVER_A_LIVE_RESERVATION: &str = "a reservation over a guest-physical range one of this device's reservations already \
     covers; the kernel refuses overlapping MEMSLOTS, but an observe-tiered span has no \
     memslot for it to refuse, so nothing outside this check can see the collision";

/// ★★ `set_trap` refusal: a read-write trap over a range a memslot already serves.
pub const TRAP_OVER_A_LIVE_SLOT: &str = "a read-write trap over a range a live memslot serves; the guest's access resolves from \
     the slot and never leaves the guest, so the registration reads as a trap and is none";

/// ★★ `set_trap` refusal: a write-only trap with no read-only slot beneath it.
pub const WRITE_TRAP_WITHOUT_A_READ_ONLY_SLOT: &str = "a write-only trap over a range no read-native tier covers — reads are served from RAM \
     only if a memslot exists, and writes exit only if that memslot is READ-ONLY";

/// Topology-callback refusal: a reported section overlaps a region we published.
pub const FOREIGN_OVERLAPS_OURS: &str = "a reported topology section overlaps a range this device owns; the two sources would \
     race to declare the same guest-physical range";

// =====================================================================================
// Configuration
// =====================================================================================

/// One reservation the device is realized with, and how it is tiered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowSpec {
    /// Guest-physical base. Must lie inside a reservation BAR and be page-aligned.
    pub gpa: u64,
    /// Length in bytes. Must be a whole number of host pages.
    pub len: u64,
    /// ★ Guest-physical sub-ranges that get **no memslot at all**, so every guest access
    /// to them exits. Everything else in the window is passthrough.
    pub observe: Vec<Range<u64>>,
}

impl WindowSpec {
    /// A wholly-passthrough window.
    #[must_use]
    pub fn passthrough(gpa: u64, len: u64) -> Self {
        WindowSpec {
            gpa,
            len,
            observe: Vec::new(),
        }
    }
}

/// One trapped region of the device, as the realize-time table names it.
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
    /// [`Vmm::export_ram`], which then refuses loudly.
    pub shareable_ram: bool,
    /// The BARs, whose ranges are declared [`RegionKind::Device`] at realize.
    pub bars: Vec<BarPlacement>,
    /// The reservations, installed in order at realize. Runtime installs are legal too —
    /// see the crate docs' "what §1 deleted".
    pub windows: Vec<WindowSpec>,
    /// The trapped regions [`Vmm::set_trap`] is validated against.
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
    /// Cumulative `MAP_FIXED` placements — §9.3's memslot-frequency denominator.
    pub placements_made: u64,
    /// ★★★ Cumulative regions handed to the **hypervisor** to back.
    ///
    /// **Must be zero, forever.** It is the whole of `host_execution_plane.md` §1 as one
    /// number: the hypervisor reserves the window and backs nothing, so there is no
    /// constructor call to make. It is kept as a field rather than deleted because
    /// "we stopped calling it" is a claim a test can hold, and "the method no longer
    /// exists" is one that stops holding the moment somebody adds it back.
    pub regions_published: u64,
    /// Memslots currently installed in the kernel.
    pub live_memslots: u64,
    /// High-water mark of live memslots.
    pub peak_memslots: u64,
    /// Cumulative memslot installs — §9.3's memslot-frequency numerator.
    pub memslot_installs: u64,
    /// Memslot numbers re-issued from the allocator's free list.
    pub slot_numbers_recycled: u64,
    /// ★★ Times the latched BAR base was re-read and compared. The non-vacuity half of
    /// [`BAR_MOVED_UNDER_US`]: a check that never runs cannot fire.
    pub bar_base_checks: u64,
    /// ★★ Times a BAR was found somewhere other than where it was latched.
    pub bar_moves_detected: u64,
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
    /// `>= 1` once one has happened, because the copy runs *inside* the view lock.
    /// `u32::MAX` means no such copy ever ran — the vacuous case, and the reason this is
    /// a minimum rather than a maximum.
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
    /// Ops that reached commit and were undone by the R5 re-validation.
    ///
    /// ★★ **NOT YET WITNESSED, and said so rather than left to be discovered.** The arm is
    /// genuinely reachable — a teardown on another thread between this op's execute and
    /// commit phases — but no test in this suite has been *seen* to reach it, because the
    /// gap is a few instructions wide and there is no place to rendezvous inside it.
    pub r5_revalidation_failures: u64,
    /// ★ Memory-plane operations refused because the device was already unrealized.
    /// Non-vacuity for [`MEMORY_PLANE_AFTER_UNREALIZE`].
    pub ops_refused_after_unrealize: u64,
    /// Topology sections added by the listener.
    pub topology_adds: u64,
    /// Topology sections removed by the listener.
    pub topology_dels: u64,
    /// ★ §5.5's generation — the instrument that says the listener really ran.
    pub topology_generation: u64,
    /// Interrupt vectors raised.
    pub irqs_raised: u64,
    /// ★★ Reservation releases that found an accessor still holding the mapping and so
    /// handed the release to the machine instead of performing it inline.
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
    live_memslots: AtomicU64,
    peak_memslots: AtomicU64,
    memslot_installs: AtomicU64,
    slot_numbers_recycled: AtomicU64,
    bar_base_checks: AtomicU64,
    bar_moves_detected: AtomicU64,
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
    ops_refused_after_unrealize: AtomicU64,
    topology_adds: AtomicU64,
    topology_dels: AtomicU64,
    topology_generation: AtomicU64,
    irqs_raised: AtomicU64,
    window_releases_deferred: AtomicU64,
    window_mappings_released: AtomicU64,
}

impl Audit {
    /// ★ Hand-written for the reason the other backend's is: a derived `Default` gives
    /// every minimum `0`, which makes the lower half of every span assertion vacuously
    /// true. "Never observed" must be distinguishable from "observed zero".
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
            live_memslots: g(&self.live_memslots),
            peak_memslots: g(&self.peak_memslots),
            memslot_installs: g(&self.memslot_installs),
            slot_numbers_recycled: g(&self.slot_numbers_recycled),
            bar_base_checks: g(&self.bar_base_checks),
            bar_moves_detected: g(&self.bar_moves_detected),
            accessor_ranked_depth: (h(&self.accessor_depth_min), h(&self.accessor_depth_max)),
            syscall_ranked_depth: (h(&self.syscall_depth_min), h(&self.syscall_depth_max)),
            own_copy_leaf_depth_max: h(&self.own_copy_leaf_max),
            host_copy_leaf_depth_min: h(&self.host_copy_leaf_min),
            view_leaf_depth_max: h(&self.view_leaf_max),
            accesses_served: g(&self.accesses_served),
            accesses_refused: g(&self.accesses_refused),
            host_refusals: g(&self.host_refusals),
            r5_revalidation_failures: g(&self.r5_failures),
            ops_refused_after_unrealize: g(&self.ops_refused_after_unrealize),
            topology_adds: g(&self.topology_adds),
            topology_dels: g(&self.topology_dels),
            topology_generation: g(&self.topology_generation),
            irqs_raised: g(&self.irqs_raised),
            window_releases_deferred: g(&self.window_releases_deferred),
            window_mappings_released: g(&self.window_mappings_released),
        }
    }
}

// =====================================================================================
// The BAR latch
// =====================================================================================

/// ★★★ Where each reservation BAR was when we first installed into it, and whether it has
/// since been caught somewhere else.
///
/// See crate doc finding 1. The poison is per-BAR and **sticky**.
#[derive(Debug, Default)]
struct BarLatch {
    /// `(bar, base at first install)`. An association list because the port's `BarId` is
    /// deliberately unordered vocabulary and there are three of them.
    latched: Vec<(BarId, u64)>,
    poisoned: Vec<BarId>,
}

impl BarLatch {
    fn base_of(&self, bar: BarId) -> Option<u64> {
        self.latched
            .iter()
            .find(|(b, _)| *b == bar)
            .map(|(_, v)| *v)
    }

    fn poison(&mut self, bar: BarId) {
        if !self.poisoned.contains(&bar) {
            self.poisoned.push(bar);
        }
    }
}

// =====================================================================================
// The plane
// =====================================================================================

/// A region the **hypervisor** owns: a section the topology listener reported.
///
/// The copy out of one runs *inside* the view lock, so the owner's release can never race
/// an accessor and never has to leave the context that is allowed to run a finalizer.
#[derive(Debug, Clone, Copy)]
struct HostOwned {
    mr: MrHandle,
    region_off: u64,
}

/// One of **our** reservations, as an accessor sees it.
#[derive(Debug, Clone)]
struct WindowView {
    gpa: u64,
    len: u64,
    window: Arc<GuestWindow>,
}

/// What `gpa_read`/`gpa_write` and the topology listener share, under the **view** leaf.
#[derive(Debug, Default)]
pub(crate) struct View {
    /// The hypervisor's flat view as reported, plus our BARs — which are `Device`, because
    /// that is what they are through the hypervisor. Crate doc finding 3.
    regions: GuestRamMap,
    backings: BTreeMap<RamRegionId, HostOwned>,
    /// ★★★ **Our** reservations, consulted BEFORE `regions` and served by our own offset
    /// arithmetic. Nothing here is ever reachable through the hypervisor.
    windows: BTreeMap<RamRegionId, WindowView>,
    /// The guest-physical tiering of every live window, as the kernel was told it,
    /// **keyed by the window that owns it**. Published under the same lock as the window,
    /// so no dispatcher can see one without the other.
    ///
    /// ★★★ Keyed, not flat, and that is the fix for a guard-defeating bookkeeping bug.
    /// This was a flat `Vec` that `remove_window` pruned by **containment** — every row
    /// inside the departing window's `[gpa, gpa+len)`. Windows nest (a `map_read_native`
    /// overlay inside a reservation is the ordinary case), so removing an OUTER window
    /// deleted an INNER window's rows while the inner window's memslots stayed live. What
    /// that costs is not a stale table: [`Vmm::set_trap`]'s [`TRAP_OVER_A_LIVE_SLOT`] check
    /// reads exactly these rows, so it would then find no slot and **pass vacuously** —
    /// registering a read-write trap over a range a live memslot serves, which is the
    /// precise condition it exists to refuse. A guard defeated by a bookkeeping bug is
    /// indistinguishable from a guard that works. Ownership is not a geometry question and
    /// is no longer answered with one.
    tiers: BTreeMap<RamRegionId, Vec<(u64, u64, Tier)>>,
    /// Foreign sections the listener declared, keyed by guest-physical base.
    foreign: BTreeMap<u64, (RamRegionId, u64, Option<MrHandle>)>,
    /// The guest-physical ranges **this device owns**, so a reported topology section that
    /// lands on one is refused rather than silently replacing a declaration we own.
    ///
    /// ★★ The third element is the owner: `Some(region)` for a reservation, `None` for a
    /// realize-time BAR. Same bug as `tiers`, one axis over — this was pruned by matching
    /// `(gpa, len)` exactly, so removing a reservation that happened to span its **whole
    /// BAR** (`WindowSpec::passthrough(bar.base, bar.len)`, the ordinary full-BAR shape)
    /// deleted the BAR's own row, after which a reported topology section could declare
    /// over a range this device owns and [`FOREIGN_OVERLAPS_OURS`] would not fire.
    ours: Vec<(u64, u64, Option<RamRegionId>)>,
    /// §5.5's counter. Bumped by every listener callback.
    topology_generation: u64,
}

impl View {
    /// Every live window's tier rows, flattened. The map is keyed by owner; a *rule* about
    /// the guest-physical plane does not care which window a row came from.
    fn tier_rows(&self) -> impl Iterator<Item = &(u64, u64, Tier)> {
        self.tiers.values().flatten()
    }
}

/// ★★★ **Is `[gpa, gpa+len)` physically what `mode` claims it is?**
///
/// The single evaluation of the rule behind [`TRAP_OVER_A_LIVE_SLOT`] and
/// [`WRITE_TRAP_WITHOUT_A_READ_ONLY_SLOT`]. [`Vmm::set_trap`] asks it at registration;
/// [`QemuMachine::assert_map_matches_the_kernel`] asks it again of the live plane, because a
/// window installed *after* a registration can falsify it and the registration would then
/// survive only in our bookkeeping.
///
/// # Errors
/// The refusal text naming which half failed.
fn trap_is_physical(v: &View, gpa: u64, len: u64, mode: TrapMode) -> Result<(), &'static str> {
    match mode {
        // A read-write trap IS the absence of a memslot. Any span with a slot under it —
        // passthrough or read-native — serves the access inside the guest.
        TrapMode::ReadWrite
            if v.tier_rows()
                .any(|(g, l, t)| *t != Tier::Observe && gpa < g + l && *g < gpa + len) =>
        {
            Err(TRAP_OVER_A_LIVE_SLOT)
        }
        // A write-only trap IS a read-only memslot. Without one the reads are served and so
        // are the writes, silently.
        TrapMode::WriteOnly
            if !v
                .tier_rows()
                .any(|(g, l, t)| *t == Tier::ReadNative && gpa >= *g && gpa + len <= g + l) =>
        {
            Err(WRITE_TRAP_WITHOUT_A_READ_ONLY_SLOT)
        }
        _ => Ok(()),
    }
}

/// One realize- or run-time reservation, as the **installer** knows it.
#[derive(Debug)]
struct Window {
    gpa: u64,
    len: u64,
    window: Arc<GuestWindow>,
    /// The live memslots and the numbers they hold. Dropping a slot clears it in the
    /// kernel; the number goes back to the allocator only afterwards.
    memslots: Vec<(u32, Box<dyn LiveSlot>)>,
    placements: BTreeMap<SlotId, (u64, u64)>,
}

/// Everything one reservation's execute phase created, so a failure anywhere inside it
/// drops the whole lot before a single field has been recorded anywhere.
#[derive(Debug)]
struct Installed {
    window: Arc<GuestWindow>,
    ram: Option<Arc<SharedRam>>,
    memslots: Vec<(u32, Box<dyn LiveSlot>)>,
}

/// The **installer**'s state: everything authoritative about the memory plane.
#[derive(Debug)]
struct Installer {
    windows: BTreeMap<RamRegionId, Window>,
    placement_owner: BTreeMap<SlotId, RamRegionId>,
    window_slot: BTreeMap<SlotId, RamRegionId>,
    traps: Vec<TrapSpec>,
    registered_traps: Vec<(BarId, Range<u64>, TrapMode)>,
    exports: Vec<std::os::fd::OwnedFd>,
    rams: BTreeMap<RamRegionId, Arc<SharedRam>>,
    blocker: Option<BlockerId>,
    next_region: u64,
    next_slot_id: u64,
    alloc: SlotAllocator,
    /// Removed reservations whose mapping has not been released yet — #57's mechanism.
    retired: Vec<Arc<GuestWindow>>,
}

#[derive(Debug)]
pub(crate) struct Plane {
    host: Arc<dyn QemuHost>,
    slots: Arc<dyn SlotPlane>,
    page: HostPageSize,
    shareable_ram: bool,
    bars: Vec<BarPlacement>,
    /// Cleared at unrealize; every memory-plane operation asserts it is set.
    live: AtomicBool,
    bar_latch: Mutex<BarLatch>,
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
        // Every door that reaches this line has just been proved lock-free on both halves,
        // which makes it the legal place to release a reservation an accessor was still
        // reading when it was removed.
        self.collect_retired();
    }

    /// The lifecycle gate.
    fn assert_live(&self) -> Result<(), VmmError> {
        if self.live.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.audit
            .ops_refused_after_unrealize
            .fetch_add(1, Ordering::SeqCst);
        Err(VmmError::Unsupported(MEMORY_PLANE_AFTER_UNREALIZE))
    }

    /// ★★★ **Latch the BAR's base**, or refuse. Called once per BAR, at its first install.
    ///
    /// Three refusals, deliberately distinct: the BAR has no base at all (the C's own
    /// guard, `C: nvkvm_mmap_host.c:194-207` — a base of zero means the guest has not
    /// programmed it and installing at a fallback would cache the wrong one); the BAR is
    /// somewhere other than where this device was realized; and the BAR has already been
    /// caught moving.
    fn latch_bar(&self, bar: BarId, declared_base: u64) -> Result<(), VmmError> {
        let mut l = self
            .bar_latch
            .lock()
            .expect("the BAR latch is never poisoned");
        if l.poisoned.contains(&bar) {
            return Err(VmmError::Unsupported(BAR_MOVED_UNDER_US));
        }
        self.audit.bar_base_checks.fetch_add(1, Ordering::SeqCst);
        let Some(live) = self.host.bar_base(bar) else {
            return Err(VmmError::Unsupported(BAR_NOT_PROGRAMMED));
        };
        if live != declared_base {
            return Err(VmmError::Unsupported(BAR_NOT_AT_ITS_DECLARED_BASE));
        }
        match l.base_of(bar) {
            Some(was) if was != live => {
                l.poison(bar);
                self.audit.bar_moves_detected.fetch_add(1, Ordering::SeqCst);
                Err(VmmError::Unsupported(BAR_MOVED_UNDER_US))
            }
            Some(_) => Ok(()),
            None => {
                l.latched.push((bar, live));
                Ok(())
            }
        }
    }

    /// ★★★ **The assertion on every resolve.** Re-reads each latched BAR's base and
    /// compares it with what it was when we installed into it.
    ///
    /// Runs **outside** every leaf lock, before the view is taken — see the crate docs'
    /// ladder. [`host::QemuHost::bar_base`] is normatively one field read for exactly this
    /// reason.
    fn check_bars(&self) -> Result<(), VmmError> {
        let mut l = self
            .bar_latch
            .lock()
            .expect("the BAR latch is never poisoned");
        if !l.poisoned.is_empty() {
            return Err(VmmError::Unsupported(BAR_MOVED_UNDER_US));
        }
        let latched = l.latched.clone();
        let mut moved = false;
        for (bar, was) in latched {
            self.audit.bar_base_checks.fetch_add(1, Ordering::SeqCst);
            if self.host.bar_base(bar) != Some(was) {
                l.poison(bar);
                self.audit.bar_moves_detected.fetch_add(1, Ordering::SeqCst);
                moved = true;
            }
        }
        if moved {
            return Err(VmmError::Unsupported(BAR_MOVED_UNDER_US));
        }
        Ok(())
    }

    /// Park a removed reservation's mapping where no accessor's clone can be the last one.
    fn retire(&self, window: Arc<GuestWindow>) {
        if Arc::strong_count(&window) > 1 {
            self.audit
                .window_releases_deferred
                .fetch_add(1, Ordering::SeqCst);
        }
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

    /// The BAR a guest-physical range lies wholly inside.
    fn bar_for(&self, gpa: u64, len: u64) -> Option<BarPlacement> {
        self.bars
            .iter()
            .copied()
            .find(|b| gpa >= b.base && gpa.checked_add(len).is_some_and(|e| e <= b.base + b.len))
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

/// ★★ Map a hypervisor refusal onto the port's vocabulary.
///
/// [`HostError::Busy`] is the interesting arm and it is task #97's whole point: it carries
/// **the name of the conflicting device**, and that name must reach the operator. Flattening
/// it to a class constant — which is what this function used to do — leaves an operator who
/// is told only *"a discard requirer is present"* to bisect their own command line. So the
/// name is what comes out, with the kernel's own `EBUSY` beside it.
fn qemu_refused(what: &'static str, e: &HostError) -> VmmError {
    match e {
        HostError::Busy { what } => VmmError::HostRefused {
            what,
            errno: Some(slots::KERNEL_EBUSY),
        },
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

/// A realized device: the owner of the memory plane and of every memslot we installed.
///
/// Hand out [`QemuMachine::vmm`] handles to threads; keep this to drive the lifecycle, to
/// feed the topology listener and the BAR tripwire, and to read the [`AuditReport`].
#[derive(Debug)]
pub struct QemuMachine {
    plane: Arc<Plane>,
}

impl QemuMachine {
    /// ★★ §8.1's realize, in its order, with every failure arm unwinding what it created
    /// **before** recording it anywhere.
    ///
    /// # Errors
    /// - [`VmmError::Unsupported`] naming [`BELOW_FLOOR`] or [`NOT_ACCELERATED`];
    /// - [`VmmError::HostRefused`] if the hypervisor or the OS refused something —
    ///   including task #97's `EBUSY` arm, which carries the **name of the conflicting
    ///   device**.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1).
    pub fn realize(
        cfg: MachineConfig,
        host: Arc<dyn QemuHost>,
        slots: Arc<dyn SlotPlane>,
    ) -> Result<Self, VmmError> {
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
        // 3. The kernel's memslot ceiling, and a descending allocator carved out of it.
        let ceiling = slots
            .ceiling()
            .map_err(|e| host_refused("querying the memslot ceiling", &e))?;
        let alloc = SlotAllocator::new(ceiling).map_err(VmmError::Unsupported)?;
        // 4. Block migration and checkpoint-restart, before anything is mapped (§8.4).
        let blocker = host
            .migrate_add_blocker("this device forwards to a host GPU through process-local state")
            .map_err(|e| qemu_refused("adding a migration blocker", &e))?;
        // 5. Refuse guest-driven discard, or unwind the blocker (task #97).
        if let Err(e) = host.ram_block_discard_disable(true) {
            host.migrate_del_blocker(blocker);
            return Err(qemu_refused("disabling guest-driven RAM discard", &e));
        }

        let page = HostPageSize::query();
        let plane = Arc::new(Plane {
            host: Arc::clone(&host),
            slots,
            page,
            shareable_ram: cfg.shareable_ram,
            bars: cfg.bars.clone(),
            live: AtomicBool::new(true),
            bar_latch: Mutex::new(BarLatch::default()),
            view: Mutex::new(View::default()),
            installer: Mutex::new(Installer {
                windows: BTreeMap::new(),
                placement_owner: BTreeMap::new(),
                window_slot: BTreeMap::new(),
                traps: cfg.traps.clone(),
                registered_traps: Vec::new(),
                exports: Vec::new(),
                rams: BTreeMap::new(),
                blocker: Some(blocker),
                next_region: 1,
                next_slot_id: 1,
                alloc,
                retired: Vec::new(),
            }),
            clock: Mutex::new((Instant::ZERO, DeferQueue::new())),
            audit: Audit::new(),
        });
        let machine = QemuMachine { plane };

        // 6-8. Everything that can fail from here unwinds through `unrealize`, which is the
        //      same teardown the ordinary path takes. A partial realize must leave the host
        //      address space, the kernel's slot table and the hypervisor's blocker exactly
        //      as it found them, and using the real teardown is what stops that from being
        //      a second, less-tested code path.
        if let Err(e) = machine.realize_regions(&cfg) {
            machine.unrealize();
            return Err(e);
        }
        Ok(machine)
    }

    fn realize_regions(&self, cfg: &MachineConfig) -> Result<(), VmmError> {
        {
            let (mut v, _h) = self.plane.view();
            for b in &cfg.bars {
                // ★★ Every BAR is DEVICE, and a reservation installed into one does NOT
                // change that (crate doc finding 3). It is what the hypervisor's flat view
                // says, and it is the backstop under our own window lookup.
                v.regions
                    .declare(
                        RamRegionId((1u64 << 63) | b.base),
                        RegionKind::Device,
                        b.base,
                        b.len,
                    )
                    .map_err(|_| VmmError::Unsupported("a BAR that leaves the 64-bit space"))?;
                v.ours.push((b.base, b.len, None));
            }
        }
        for w in &cfg.windows {
            self.install_window(w, "installing a realize-time reservation")?;
        }
        self.plane
            .host
            .register_listener()
            .map_err(|e| qemu_refused("registering the topology listener", &e))?;
        Ok(())
    }

    /// ★★ **The coarse tier** — one reservation and the memslots that shadow it.
    ///
    /// Legal at any time: under `host_execution_plane.md` §1 this touches the *kernel*, not
    /// the hypervisor's region tree, so it is not a topology transaction and §4.3 has
    /// nothing to say about it.
    ///
    /// # Errors
    /// - [`VmmError::Unsupported`] — a misaligned or empty request, a range outside every
    ///   BAR, a BAR that is backed / unprogrammed / moved, an unsatisfiable tier list, or
    ///   the slot budget.
    /// - [`VmmError::HostRefused`] — the OS or the kernel refused. **On any of these the
    ///   reservation is dropped, every slot that did install is cleared, and nothing is
    ///   recorded.**
    ///
    /// # Panics
    /// If called with any ranked lock or any leaf lock held (R1).
    pub fn install_ram_window(&self, gpa: u64, len: u64) -> Result<RamRegionId, VmmError> {
        self.install_window(
            &WindowSpec::passthrough(gpa, len),
            "installing a reservation",
        )
    }

    /// The same, with an explicit tier list — the mixed passthrough/observe layout.
    ///
    /// # Errors
    /// As [`QemuMachine::install_ram_window`].
    ///
    /// # Panics
    /// If called with any ranked lock or any leaf lock held (R1).
    pub fn install_tiered_window(&self, spec: &WindowSpec) -> Result<RamRegionId, VmmError> {
        self.install_window(spec, "installing a tiered reservation")
    }

    fn install_window(
        &self,
        spec: &WindowSpec,
        what: &'static str,
    ) -> Result<RamRegionId, VmmError> {
        self.install_window_inner(spec, None, what).map(|(r, _)| r)
    }

    /// The shared body. `read_native` carries the write-trap sub-range that needs a
    /// read-only slot of its own.
    fn install_window_inner(
        &self,
        spec: &WindowSpec,
        read_native: Option<&Range<u64>>,
        what: &'static str,
    ) -> Result<(RamRegionId, SlotId), VmmError> {
        let p = &self.plane;
        p.about_to_syscall(what);
        p.assert_live()?;
        let (gpa, len) = (spec.gpa, spec.len);

        // ---- PLAN (pure bookkeeping; the only syscall is the BAR field read) ----------
        if !geometry::is_aligned(gpa, p.page) || !geometry::is_aligned(len, p.page) || len == 0 {
            return Err(VmmError::Unsupported(
                "a reservation whose base or length is not a whole number of host pages",
            ));
        }
        let Some(placement) = p.bar_for(gpa, len) else {
            return Err(VmmError::Unsupported(
                "a reservation that is not inside any realized BAR",
            ));
        };
        // ★★★ The reservation BAR must be one the hypervisor does NOT back. This is the
        // whole §1.5 safety argument, asked rather than assumed.
        if !p.host.bar_is_unbacked_reservation(placement.bar) {
            return Err(VmmError::Unsupported(WINDOW_IN_A_BACKED_BAR));
        }
        p.latch_bar(placement.bar, placement.base)?;

        let cuts = self.tier_cuts(spec, read_native)?;
        let spans = slots::spans(len, &cuts).map_err(VmmError::Unsupported)?;
        // ★★ ONE place decides whether a tier installs a slot, and it is
        // `Tier::readonly_slot`. This used to read `s.tier != Tier::Observe` — a **second
        // evaluation site** for the same rule, and a bite-check proved it: flipping
        // `Tier::Observe`'s arm from "no slot" to "a read-write slot" changed nothing here,
        // because this line was not asking. That is the decay `classify` is a whole module
        // to prevent, reproduced one crate over.
        let want: Vec<&Span> = spans
            .iter()
            .filter(|s| s.tier.readonly_slot().is_some())
            .collect();

        let (region, slot_id, numbers) = {
            let (mut ins, _h) = p.installer();
            // ★★★ No two of OUR reservations may claim the same guest-physical range. See
            // [`WINDOW_OVER_A_LIVE_RESERVATION`] for why the kernel's `EEXIST` does not
            // cover this: an observe-tiered span installs no memslot, so the kernel is
            // never asked about it.
            if ins
                .windows
                .values()
                .any(|w| gpa < w.gpa + w.len && w.gpa < gpa + len)
            {
                return Err(VmmError::Unsupported(WINDOW_OVER_A_LIVE_RESERVATION));
            }
            let numbers = ins.alloc.alloc(want.len()).map_err(VmmError::Unsupported)?;
            p.audit
                .slot_numbers_recycled
                .store(ins.alloc.recycled(), Ordering::SeqCst);
            let region = RamRegionId(ins.next_region);
            ins.next_region += 1;
            let slot_id = SlotId(ins.next_slot_id);
            ins.next_slot_id += 1;
            (region, slot_id, numbers)
        };

        // ---- EXECUTE (every lock dropped) --------------------------------------------
        assert_leaf_free(what);
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
            let mut memslots = Vec::new();
            for (n, s) in numbers.iter().zip(want.iter()) {
                let readonly = s
                    .tier
                    .readonly_slot()
                    .expect("the filter above admitted exactly the tiers that install one");
                let live = p
                    .slots
                    .install(*n, gpa + s.offset, &window, s.offset, s.len, readonly)
                    .map_err(|e| {
                        p.audit.host_refusals.fetch_add(1, Ordering::SeqCst);
                        host_refused("a memslot install", &e)
                    })?;
                p.audit.memslot_installs.fetch_add(1, Ordering::SeqCst);
                memslots.push((*n, live));
            }
            Ok(Installed {
                window,
                ram,
                memslots,
            })
        })();

        // ★ The partial-failure path. `outcome` owns everything that was created, so
        // dropping it here unmaps the reservation and clears whatever slots did install —
        // before any of it was recorded anywhere. The numbers still have to go back, and
        // they go back AFTER that drop, which is the ordering the allocator's contract
        // demands.
        let installed = match outcome {
            Ok(i) => i,
            Err(e) => {
                let (mut ins, _h) = p.installer();
                for n in numbers {
                    ins.alloc.release(n);
                }
                return Err(e);
            }
        };
        let n_slots = installed.memslots.len() as u64;
        let win_handle = Arc::clone(&installed.window);
        let tiers: Vec<(u64, u64, Tier)> = spans
            .iter()
            .map(|s| (gpa + s.offset, s.len, s.tier))
            .collect();

        // ---- COMMIT (installer, then the view) ---------------------------------------
        {
            let (mut ins, _h) = p.installer();
            ins.window_slot.insert(slot_id, region);
            if let Some(r) = installed.ram {
                ins.rams.insert(region, r);
            }
            ins.windows.insert(
                region,
                Window {
                    gpa,
                    len,
                    window: Arc::clone(&installed.window),
                    memslots: installed.memslots,
                    placements: BTreeMap::new(),
                },
            );
            Audit::bump(&p.audit.live_windows, &p.audit.peak_windows, 1);
            Audit::bump(
                &p.audit.live_memslots,
                &p.audit.peak_memslots,
                n_slots as i64,
            );
            p.audit.window_bytes.fetch_add(len, Ordering::SeqCst);
        }
        {
            let (mut v, _h) = p.view();
            // ★★★ NOT a `regions.declare(.., Ram, ..)`. The window is served from OUR map,
            // and the region map keeps saying `Device` — crate doc finding 3.
            v.windows.insert(
                region,
                WindowView {
                    gpa,
                    len,
                    window: win_handle,
                },
            );
            v.tiers.insert(region, tiers);
            v.ours.push((gpa, len, Some(region)));
        }
        Ok((region, slot_id))
    }

    /// The tier cuts for one window: its configured observe ranges, plus the read-native
    /// span if this is a `map_read_native`. All rounded out to whole host pages, because a
    /// tier is a **page** attribute in the kernel's tables.
    fn tier_cuts(
        &self,
        spec: &WindowSpec,
        read_native: Option<&Range<u64>>,
    ) -> Result<Vec<(u64, u64, Tier)>, VmmError> {
        let p = &self.plane;
        let mut cuts = Vec::new();
        for r in spec.observe.iter().chain(read_native) {
            let tier = if read_native.is_some_and(|rn| rn == r) {
                Tier::ReadNative
            } else {
                Tier::Observe
            };
            if r.start < spec.gpa || r.end > spec.gpa + spec.len || r.start >= r.end {
                return Err(VmmError::Unsupported(slots::TIER_OUTSIDE_ITS_WINDOW));
            }
            let (off, l) = geometry::round_out(r.start - spec.gpa, r.end - r.start, p.page)
                .map_err(|_| {
                    VmmError::Unsupported("a tier sub-range that cannot be page-aligned")
                })?;
            cuts.push((off, l.min(spec.len - off), tier));
        }
        Ok(cuts)
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

    /// The kernel's memslot ceiling, and the floor this device's numbers descend to.
    #[must_use]
    pub fn slot_range(&self) -> (u32, u32) {
        let ins = self.plane.installer.lock().expect("installer");
        (ins.alloc.floor(), ins.alloc.ceiling())
    }

    /// The audit / conservation ledger.
    #[must_use]
    pub fn audit(&self) -> AuditReport {
        self.plane.audit.report()
    }

    /// Create a host backing an isolate would have handed us, and name it as the port does.
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
    /// ★ Under crate doc finding 3 this reports our window's BAR as a **device**, which is
    /// what the hypervisor's flat view says it is. A window is not in this map at all; it
    /// is in ours.
    ///
    /// # Errors
    /// As [`GuestRamMap::resolve`].
    pub fn resolve_region(&self, gpa: u64, len: u64) -> Result<kayfabe_vmm::RamSpan, VmmError> {
        let (v, _h) = self.plane.view();
        v.regions.resolve(gpa, len)
    }

    /// The guest-physical tiering, as the kernel was told it.
    #[must_use]
    pub fn tiers(&self) -> Vec<(u64, u64, Tier)> {
        let (v, _h) = self.plane.view();
        let mut t: Vec<(u64, u64, Tier)> = v.tiers.values().flatten().copied().collect();
        t.sort_unstable_by_key(|(g, _, _)| *g);
        t
    }

    /// ★★★ **The whole memory plane, re-checked against itself — and the only thing that
    /// reads [`Installer::registered_traps`].**
    ///
    /// Returns the number of live reservations checked, so a caller can assert non-vacuity;
    /// an empty plane passes this trivially and must never be mistaken for a verified one.
    ///
    /// # Why it exists
    ///
    /// `registered_traps` was **write-only**: [`Vmm::set_trap`] pushed to it and nothing —
    /// not even an accessor — ever read it. That is the same shape the sibling KVM adapter
    /// found and fixed in its own `Installer::traps`, and the same shape as #89: a
    /// registration that looks like it configures something and configures nothing.
    ///
    /// The defect is not merely tidiness. `set_trap` checks its precondition **once, at
    /// registration time**, against the tiering as it stands then. A reservation installed
    /// afterwards can falsify it — install a passthrough window over a range registered as
    /// a read-write trap and the guest's access is served silently from the memslot, with
    /// the trap surviving only in that vector. Re-asking the question of the live plane is
    /// the only thing that makes keeping the vector worth more than deleting it.
    ///
    /// It asks three further questions that only this side of the seam can answer, because
    /// the installer's bookkeeping and the view's are separate structures that a partial
    /// failure or a mis-scoped prune can silently pull apart.
    ///
    /// # Panics
    /// If the plane disagrees with itself, naming which of the four clauses failed.
    #[must_use]
    pub fn assert_map_matches_the_kernel(&self) -> usize {
        let p = &self.plane;
        let (v, _h) = p.view();
        let (ins, _h2) = p.installer();
        let mut checked = 0usize;
        for (region, w) in &ins.windows {
            let view = v.windows.get(region).unwrap_or_else(|| {
                panic!(
                    "{region:?} is installed — {} live memslot(s) over [{:#x}, {:#x}) — and \
                     the view cannot resolve it; the guest can reach a mapping our own \
                     lookup denies",
                    w.memslots.len(),
                    w.gpa,
                    w.gpa + w.len
                )
            });
            assert_eq!(
                (view.gpa, view.len),
                (w.gpa, w.len),
                "{region:?}'s view and installer disagree about where it is"
            );
            assert!(
                Arc::ptr_eq(&view.window, &w.window),
                "{region:?}'s view and installer disagree about which mapping backs it"
            );

            // ★★ The tier rows this window owns must still BE this window's tier rows: they
            // must tile `[gpa, gpa+len)` exactly, and the ones that install a slot must
            // number exactly its live memslots. This is what a prune-by-containment cannot
            // survive — the inner window it deleted has zero rows and one live memslot.
            let rows = v.tiers.get(region).unwrap_or_else(|| {
                panic!(
                    "{region:?} is a live reservation with NO tier rows — every rule about \
                     what the guest's access does reads those rows, so a trap registration \
                     over this range would now pass vacuously"
                )
            });
            let mut at = w.gpa;
            for (g, l, _) in rows {
                assert_eq!(
                    *g, at,
                    "{region:?}'s tier rows must tile its range with no gap and no \
                     overlap ({rows:?})"
                );
                at += l;
            }
            assert_eq!(
                at,
                w.gpa + w.len,
                "{region:?}'s tier rows must cover it all"
            );
            assert_eq!(
                rows.iter()
                    .filter(|(_, _, t)| t.readonly_slot().is_some())
                    .count(),
                w.memslots.len(),
                "{region:?} has {} live memslot(s) and {} slot-installing tier row(s) — the \
                 kernel and the table disagree about which spans exit",
                w.memslots.len(),
                rows.iter()
                    .filter(|(_, _, t)| t.readonly_slot().is_some())
                    .count()
            );

            // ★ And the range is still claimed as ours, or a reported topology section
            // could declare over it and `FOREIGN_OVERLAPS_OURS` would not fire.
            assert!(
                v.ours.contains(&(w.gpa, w.len, Some(*region))),
                "{region:?} covers [{:#x}, {:#x}) and no longer claims it — a foreign \
                 section over this range would be accepted",
                w.gpa,
                w.gpa + w.len
            );
            checked += 1;
        }
        assert_eq!(
            checked,
            v.windows.len(),
            "the view resolves reservations the installer does not hold — a range served \
             from a mapping nobody is going to unmap"
        );
        assert_eq!(
            checked,
            v.tiers.len(),
            "there are tier rows for reservations that no longer exist; a stale row is \
             worse than a missing one, because `set_trap` would refuse on the strength of a \
             memslot that is gone"
        );

        // ★★★ THE READ. Every registered trap, re-asked of the live plane through the same
        // one function `set_trap` used, so the two can never drift.
        for (bar, range, mode) in &ins.registered_traps {
            let Some(b) = p.bars.iter().find(|b| b.bar == *bar) else {
                continue;
            };
            if let Err(why) =
                trap_is_physical(&v, b.base + range.start, range.end - range.start, *mode)
            {
                panic!(
                    "{bar:?}+{range:?} is registered as a {mode:?} trap and the live plane \
                     no longer makes it one: {why}"
                );
            }
        }
        checked
    }

    /// Advance the virtual clock, returning every deferred event that became due.
    pub fn advance(&self, d: Duration) -> Vec<CoreEvent> {
        let mut c = self.plane.clock.lock().expect("clock");
        c.0 = c.0.advanced(d);
        let now = c.0;
        c.1.due(now)
    }

    // --- the BAR tripwire (crate doc finding 1) ------------------------------------

    /// ★★★ **The tripwire.** What a topology listener — or the device's own PCI
    /// mapping-update path — calls when a BAR is (re)mapped.
    ///
    /// `host_execution_plane.md` §1.5: the accelerator's own handler early-returns for our
    /// pure-MMIO reservation, so it creates no slot — but the hypervisor's *listener* still
    /// fires for it, which is what makes this callable at all. A base that differs from the
    /// latched one poisons the BAR, and every access into it refuses from then on.
    ///
    /// Sticky on purpose: a BAR that moved away and back may have been another device's in
    /// between, and our memslot was live over the old range the whole time.
    pub fn note_bar_mapping(&self, bar: BarId, base: Option<u64>) {
        let p = &self.plane;
        let mut l = p.bar_latch.lock().expect("the BAR latch is never poisoned");
        let Some(was) = l.base_of(bar) else {
            return;
        };
        if base != Some(was) {
            l.poison(bar);
            p.audit.bar_moves_detected.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// ★★ **The refusal.** What a configuration-space write override calls *before* letting
    /// a base-address-register write through.
    ///
    /// The tripwire above is a detector and this is a preventer; they are both here because
    /// they close different halves. A device that refuses the move never gets into the
    /// inconsistent state at all; a device that only detects it finds out afterwards, which
    /// is still infinitely better than the C's silence but is not the same thing.
    ///
    /// # Errors
    /// [`VmmError::Unsupported`] naming [`BAR_MOVED_UNDER_US`] once a memslot has been
    /// installed into that BAR.
    pub fn bar_move_requested(&self, bar: BarId) -> Result<(), VmmError> {
        let l = self
            .plane
            .bar_latch
            .lock()
            .expect("the BAR latch is never poisoned");
        if l.base_of(bar).is_some() {
            return Err(VmmError::Unsupported(BAR_MOVED_UNDER_US));
        }
        Ok(())
    }

    // --- the topology listener (§5.2) ---------------------------------------------

    /// ★ The listener's add callback. Arrives holding the VMM's global lock, so the only
    /// thing it may do is a bounded update of our leaf map (§4.2's table).
    ///
    /// # Errors
    /// [`VmmError::Unsupported`] naming [`FOREIGN_OVERLAPS_OURS`] if the reported section
    /// overlaps a range this device owns; [`VmmError::HostRefused`] if the reference could
    /// not be taken.
    pub fn region_add(&self, s: SectionDesc) -> Result<(), VmmError> {
        let p = &self.plane;
        let kind = classify::classify(&s.facts);
        // ★ The overlap check happens BEFORE any reference is taken, so a refusal leaves
        // the hypervisor's reference count exactly as it found it.
        {
            let (v, _h) = p.view();
            let end = u128::from(s.gpa) + u128::from(s.len.max(1));
            if v.ours.iter().any(|(b, l, _)| {
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
                HostOwned {
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

    /// ★ §8.3's unrealize: stop serving, release every reservation and its memslots, and
    /// withdraw the migration blocker.
    ///
    /// The hypervisor frees **nothing** of ours — it never took ownership of anything — so
    /// this step is not optional and there is no backstop for it.
    ///
    /// # Panics
    /// If called with any ranked lock or any leaf lock held (R1).
    pub fn unrealize(&self) {
        let p = &self.plane;
        p.about_to_syscall("unrealize");

        let regions: Vec<RamRegionId> = {
            let (ins, _h) = p.installer();
            ins.windows.keys().copied().collect()
        };
        for r in regions {
            let _ = self.remove_window(r);
        }
        p.live.store(false, Ordering::SeqCst);
        let blocker = {
            let (mut ins, _h) = p.installer();
            ins.blocker.take()
        };
        // ★★★ THE REFERENCES, BEFORE THE VIEW IS DROPPED — found by the reload test
        // (`kayfabe-qemu-raw/tests/device_recycle.rs`), 2026-07-31.
        //
        // Every reported RAM section carries a reference this device took in `region_add`,
        // and `*v = View::default()` used to drop the map holding them: a `MrHandle` is an
        // opaque name, not an owning value, so nothing was released. The hypervisor could
        // then never finalize a region this device had merely *seen*, and a device that is
        // unloaded and reloaded leaks one reference per reported section PER LOAD.
        //
        // ★ The shipping shell masked it: `nvkvm_exit` unregisters the topology listener
        // first, and QEMU's `memory_listener_unregister` replays `region_del` over every
        // flat range (`system/memory.c:3112-3137`), which releases them one at a time. So
        // the leak was reachable only through the seam, which is precisely where this
        // archive's contract lives — "unrealize leaves the machine as it found it" was
        // false for the archive and true only for one caller's ordering.
        //
        // ★ Draining rather than iterating is what makes it safe against BOTH orderings:
        // a section the listener already deleted is no longer in the map, so the shell's
        // order releases each reference exactly once and this loop finds nothing left.
        let orphaned: Vec<MrHandle> = {
            let (mut v, _h) = p.view();
            let mrs = core::mem::take(&mut v.foreign)
                .into_values()
                .filter_map(|(_, _, mr)| mr)
                .collect();
            *v = View::default();
            mrs
        };
        {
            let mut l = p.bar_latch.lock().expect("the BAR latch is never poisoned");
            *l = BarLatch::default();
        }
        assert_leaf_free("withdrawing the device's lifecycle claims");
        for mr in orphaned {
            p.host.unref_region(mr);
        }
        if let Some(b) = blocker {
            p.host.migrate_del_blocker(b);
        }
        let _ = p.host.ram_block_discard_disable(false);
        p.collect_retired();
    }

    /// Remove one reservation: the guest-physical range stops resolving **first**, then the
    /// memslots are cleared, then the mapping is retired and the numbers go back.
    ///
    /// Public because a reservation is now a runtime object — see the crate docs' "what §1
    /// deleted". [`Vmm::unmap_guest`] reaches the same code with the coarse slot id.
    ///
    /// # Errors
    /// [`VmmError::BadSlot`] if the region is unknown.
    ///
    /// # Panics
    /// If called with any ranked lock or any leaf lock held (R1).
    pub fn remove_window(&self, region: RamRegionId) -> Result<(), VmmError> {
        let p = &self.plane;
        p.about_to_syscall("removing a reservation");
        let taken = {
            let (mut ins, _h) = p.installer();
            let Some(w) = ins.windows.remove(&region) else {
                return Err(VmmError::BadSlot(SlotId(region.0)));
            };
            ins.placement_owner.retain(|_, r| *r != region);
            ins.window_slot.retain(|_, r| *r != region);
            ins.rams.remove(&region);
            Audit::bump(&p.audit.live_windows, &p.audit.peak_windows, -1);
            Audit::bump(
                &p.audit.live_placements,
                &p.audit.peak_placements,
                -(w.placements.len() as i64),
            );
            Audit::bump(
                &p.audit.live_memslots,
                &p.audit.peak_memslots,
                -(w.memslots.len() as i64),
            );
            p.audit.window_bytes.fetch_sub(w.len, Ordering::SeqCst);
            w
        };
        {
            let (mut v, _h) = p.view();
            v.windows.remove(&region);
            // ★★★ By OWNERSHIP, not by containment or by equality — see `View::tiers`
            // and `View::ours`. The two lines this replaced deleted rows belonging to
            // whatever else happened to sit inside, or to exactly match, the departing
            // window's range.
            v.tiers.remove(&region);
            v.ours.retain(|(_, _, owner)| *owner != Some(region));
        }
        assert_leaf_free("clearing a reservation's memslots");
        // ★★★ The ordering the allocator's contract demands: drop the live slots FIRST —
        // each `Drop` is the clearing ioctl and asserts it succeeded — and only then hand
        // the numbers back. A number returned before its slot was cleared turns the next
        // install from an ADD into a silent REPLACE.
        let mut numbers = Vec::with_capacity(taken.memslots.len());
        for (n, live) in taken.memslots {
            drop(live);
            numbers.push(n);
        }
        {
            let (mut ins, _h) = p.installer();
            for n in numbers {
                ins.alloc.release(n);
            }
        }
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

/// The copy a host-owned resolution performs **inside** the view lock.
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

    /// ★★★ Resolve, in the one order that matters.
    ///
    /// 1. **The BAR latch**, outside every leaf lock (crate doc finding 1).
    /// 2. **Our own windows**, by our own offset arithmetic (crate doc finding 3). The
    ///    hypervisor is not consulted, because through the hypervisor this range is a stub
    ///    that returns zeros.
    /// 3. Only then the region map, for foreign RAM — and for the refusal a bypassed step 2
    ///    lands on.
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
        p.check_bars().inspect_err(|_| {
            p.audit.accesses_refused.fetch_add(1, Ordering::SeqCst);
        })?;
        let (v, held) = p.view();
        if let Some(w) = v
            .windows
            .values()
            .find(|w| gpa >= w.gpa && gpa.checked_add(len).is_some_and(|e| e <= w.gpa + w.len))
        {
            let (window, offset) = (Arc::clone(&w.window), gpa - w.gpa);
            // Released EXPLICITLY, before the caller copies — `own_copy_leaf_depth_max` is
            // the assertion that this really happened.
            core::mem::drop(v);
            core::mem::drop(held);
            return Ok(Resolved::Ours(window, offset));
        }
        let span = v.regions.resolve(gpa, len).inspect_err(|_| {
            p.audit.accesses_refused.fetch_add(1, Ordering::SeqCst);
        })?;
        let backing = v.backings.get(&span.region).copied().ok_or_else(|| {
            p.audit.accesses_refused.fetch_add(1, Ordering::SeqCst);
            VmmError::BadGpa { gpa }
        })?;
        // ★ INSIDE the lock, on purpose. `host_copy_leaf_depth_min` is the assertion.
        p.audit
            .host_copy_leaf_min
            .fetch_min(leafwitness::depth(), Ordering::SeqCst);
        p.audit.accesses_served.fetch_add(1, Ordering::SeqCst);
        let r = copy(
            p.host.as_ref(),
            backing.mr,
            backing.region_off + span.offset,
        );
        core::mem::drop(v);
        core::mem::drop(held);
        r.map(|()| Resolved::Done)
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

    /// ★★ The fine tier — a `MAP_FIXED` placement inside an already-installed reservation.
    /// **No kernel call and no hypervisor call at all**: the window's memslot already names
    /// the whole range, which is what keeps §6.7's frequency rule satisfiable.
    fn map_guest(
        &mut self,
        gpa: u64,
        len: u64,
        backing: HostRegion,
        prot: Prot,
    ) -> Result<SlotId, VmmError> {
        let p = &self.plane;
        p.about_to_syscall("map_guest (a MAP_FIXED placement inside a reservation)");
        p.assert_live()?;

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
        // ★★ R5's token is the reservation's **presence**, not a version counter: a
        // per-window generation written once at install and never mutated is unfalsifiable
        // (a bite-check found exactly that survivor), whereas a concurrent teardown really
        // does remove the whole entry.
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

    /// Remove a mapping — a fine-tier placement is **restored** to anonymous backing, never
    /// unmapped; a coarse-tier slot removes the whole reservation and clears its memslots.
    fn unmap_guest(&mut self, slot: SlotId) -> Result<(), VmmError> {
        let p = &self.plane;
        p.about_to_syscall("unmap_guest");

        enum Plan {
            Placement(Arc<GuestWindow>, u64, u64),
            Window(RamRegionId),
        }
        let plan = {
            let (mut ins, _h) = p.installer();
            if let Some(region) = ins.window_slot.get(&slot).copied() {
                Plan::Window(region)
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
            Plan::Window(region) => {
                let machine = QemuMachine {
                    plane: Arc::clone(p),
                };
                machine.remove_window(region)
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

    /// ★★ Register a trapped range — validated against the realize-time table **and against
    /// the physical tiering**.
    ///
    /// The table check is the Rust half of §3.3's clause (c). The tier check is the half
    /// that is new here and is the whole point of `host_execution_plane.md` §1's taxonomy: a
    /// trap registration is a claim about what the guest's access *does*, and what it does
    /// is decided by whether a memslot covers the range and with which polarity. A
    /// read-write trap over a passthrough span never fires; a write-only trap over one never
    /// fires either. Both read as protection and are none.
    fn set_trap(&mut self, bar: BarId, range: Range<u64>, mode: TrapMode) -> Result<(), VmmError> {
        let p = &self.plane;
        p.about_to_syscall("set_trap");
        p.assert_live()?;
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
        let (gpa, len) = (b.base + range.start, range.end - range.start);
        {
            let (ins, _h) = p.installer();
            if !ins.traps.iter().any(|t| {
                t.bar == bar
                    && t.mode == mode
                    && t.range.start <= range.start
                    && range.end <= t.range.end
            }) {
                return Err(VmmError::Unsupported(TRAP_OUTSIDE_THE_REALIZED_TABLE));
            }
        }
        {
            let (v, _h) = p.view();
            // ★ ONE evaluation site for "is this registration physically a trap", shared
            // with [`QemuMachine::assert_map_matches_the_kernel`], which re-asks it of the
            // live plane. Two copies of this rule would drift, and the drifted copy that
            // still passes is the failure this crate has already caught once (see the
            // `Tier::readonly_slot` note in `install_window_inner`).
            if let Err(why) = trap_is_physical(&v, gpa, len, mode) {
                return Err(VmmError::Unsupported(why));
            }
        }
        let (mut ins, _h) = p.installer();
        ins.registered_traps.push((bar, range, mode));
        Ok(())
    }

    /// ★ The one in-lock-legal foreign call: one descriptor write (§7).
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

    /// ★★★ Back `gpa..gpa+len` with a reservation whose write-trap sub-range is a
    /// **read-only memslot**: the guest's reads are served from our RAM with no exit at
    /// all, and its writes leave the guest. That is the read-native semantic, and under
    /// `host_execution_plane.md` §1 it is one slot flag rather than a region construction.
    ///
    /// The window is created here, at runtime — see the crate docs' "what §1 deleted".
    fn map_read_native(
        &mut self,
        gpa: u64,
        len: u64,
        backing: HostRegion,
        write_trap: Option<Range<u64>>,
    ) -> Result<SlotId, VmmError> {
        let machine = QemuMachine {
            plane: Arc::clone(&self.plane),
        };
        let spec = WindowSpec::passthrough(gpa, len);
        let (region, slot) = machine.install_window_inner(
            &spec,
            write_trap.as_ref(),
            "map_read_native (a read-native reservation)",
        )?;
        // The overlay's contents come from the named backing; a read-native window whose
        // pages were anonymous zeroes would serve reads natively and serve the wrong value,
        // which is worse than trapping.
        if backing.id != u64::MAX {
            let mut this = self.clone();
            if let Err(e) = this.map_guest(gpa, len, backing, Prot::ReadWrite) {
                machine.remove_window(region)?;
                return Err(e);
            }
        }
        Ok(slot)
    }
}

/// Duplicate a borrowed descriptor into an owned one, so the borrow — and the lock it came
/// from — can be released **before** the syscall that uses it runs.
fn dup_owned(fd: std::os::fd::BorrowedFd<'_>) -> Result<std::os::fd::OwnedFd, RawError> {
    fd.try_clone_to_owned().map_err(|e| RawError::Syscall {
        call: "dup",
        errno: e.raw_os_error(),
    })
}

kayfabe_util::assert_send_sync!(QemuVmm, QemuMachine, AuditReport);
