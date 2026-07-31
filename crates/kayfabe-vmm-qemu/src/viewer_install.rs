//! # `viewer_install` — the **consumer** of the GPGA viewer index: coverage made real
//!
//! ## What this is
//!
//! `kayfabe_mmu::gpga::ViewerIndex` knows, at every instant, what each framebuffer window
//! should show. It is a pure logic structure and it installs nothing; its own module docs say
//! so (*"No consumer is wired"*). Its author's stated doubt was exact and worth repeating:
//!
//! > *"'passthrough by construction' is not what this delivers — the index makes correctness
//! > **expressible**; the passthrough property is a data-plane claim and no code here
//! > delivers it. Necessary, not sufficient."*
//!
//! **This module is the sufficient half.** It turns index state into memslots, so that a
//! guest load from a window address the index says is covered is served by the second-level
//! page tables with **no exit** — a native memory access — and a load from an address the
//! index does *not* cover exits to us. The index says what should be visible; this makes it
//! literally true in the page tables.
//!
//! ## ★★★ The three rulings this is built to, all binding
//!
//! ### 1. Granularity is per OBJECT, never per page
//!
//! A cold boot writes the instance window 177 856 times. If a change unit were a page, that
//! is 178 000 plan/apply cycles against a deliberately coarse `ViewFault::PlanStale`, and a
//! planner would retry-starve. So [`ViewLayout`] is built from **object runs**, coalesced
//! (the consolidation ratio is counted, not claimed), and ⊘ no code here ever
//! enumerates a page. The one place a page appears is the outward rounding a memslot's
//! alignment *requires*, and that is a property of the kernel's interface, not a change unit.
//!
//! ### 2. DRAIN is a pull, and this is a drainer
//!
//! The index's three passes are DESCRIBE (rank 1) → PLAN/APPLY (rank 0, contacts no viewer)
//! → DRAIN (rank 1, one viewer alone). ⊘ Nothing here is called *from* `apply`, and nothing
//! here calls *into* `apply`. [`ViewInstaller::drain_and_install`] pulls
//! ([`ViewerIndex::drain`]), which is exactly what makes a slow installer harmless to every
//! other viewer: the index has already committed, and a hanging drainer delays only itself.
//!
//! ### 3. ★★★ The CPU optimization bits are a requirement, not a tuning pass
//!
//! A mapping that is *correct* but lands uncached is correct and roughly two orders of
//! magnitude slower, with no error and no log. See [`WindowCacheability`] for the per-window
//! requirement and [`ViewInstaller::assert_effective_cacheability`] for the read-back that
//! refuses to assume the request took.
//!
//! ## ★★★ Consolidation: the fewest mappings that cover the coverage — and the MERGE KEY
//!
//! The owner's ruling: *"you should consolidate as much pages in the least mmaps"*. The unit
//! is the **mapping**, not the page, and this is structural rather than an optimisation:
//! memslots have a hard ceiling ([`crate::slots::OUR_SLOT_BUDGET`] carved out of the
//! kernel's own), so one memslot per page is not slow, it is **impossible**. Huge pages then
//! fall out of consolidation plus alignment rather than from a second mechanism — a merged
//! run that is at least 2 MiB long and congruently aligned can be backed by a 2 MiB entry,
//! and a fragmented one cannot however it is requested ([`huge_pages_for`]).
//!
//! ⚠ **But adjacency is not a licence to merge**, and getting this wrong is a security
//! defect rather than a performance one. Two objects can sit back to back in GPGA and differ
//! in ways a single mapping cannot express. [`MergeKey`] is the whole condition, and every
//! field of it is there because flattening that field is a specific, nameable harm:
//!
//! | field | what merging across it would do |
//! |---|---|
//! | `owner` | one mapping whose owning handle namespace is ambiguous — boundary 2, the thing `ViewFault::ForeignObject` exists to refuse |
//! | `cache` | silently give one of the two runs the wrong memory type; `#111` is the precedent for a memory type being wrong **silently** |
//! | `prot` | merge read-only with read-write and the read-only half becomes writable |
//! | ★★★ `viewers` | merge a run visible to A with one visible to A **and** B, and B sees something the index never said it could — a cross-viewer leak *created by the optimisation* |
//! | `witness` | make unvouchable bytes native, which is the one thing miss = fault forbids |
//!
//! ★ The tests assert on **the key**, not on the resulting count. A test that only checks
//! *"fewer mappings than before"* is green for a merge that is wrong, which is the whole
//! difficulty: a bad merge looks exactly like a good one from the outside.
//!
//! ## ★★ What "passthrough" costs in memslots — and why it is not per object
//!
//! A memslot is installed per **merged run**, and object *content* is placed with a
//! `MAP_FIXED` inside a reservation that already has one — `kayfabe_vmm::Vmm::map_guest`
//! *"performs no QEMU call at all"* and installs no slot. So N objects inside one covered
//! run cost **one** memslot, not N. [`InstallReport::mappings_before_merge`] and
//! [`InstallReport::memslots`] report both numbers, so the consolidation ratio is a
//! measurement rather than a claim — taken over a real cold boot in
//! `crates/kayfabe-crec/tests/window_consolidation_census.rs`.
//!
//! ## ⊘ What this module refuses rather than fakes
//!
//! - **A mapping needing host-GPU backing.** The isolate ⇄ VMM descriptor crossing landed
//!   (`SCM_RIGHTS`, both directions) but **no verb uses it**, deliberately: adding one
//!   changes `RmBackend`, a pure logic crate, and that is an owner decision.
//!   [`InstallRefusal::HostGpuBackingHasNoVerb`] is the named refusal, and it is a *finding*,
//!   not a bug. ★ The raw layer agrees independently: `GuestWindow::place` refuses
//!   `Backing::DeviceFile` with `RawError::DeviceBackingNotPlaceable`, so the path is shut
//!   at two levels rather than one.
//! - **Content this port cannot vouch for.** `Witness::Unwitnessed` content becomes
//!   [`crate::slots::Tier::Observe`] — *no memslot* — so the guest traps rather than reading
//!   bytes nobody witnessed. Under miss = fault, a native read of unvouchable memory is the
//!   worse outcome, not the better one.
//! - **A layout that would exceed the slot budget**, by name and with both numbers.

use std::collections::BTreeMap;
use std::ops::Range;

use kayfabe_arch::{Aperture, FbWindow};
use kayfabe_isolate::IsolateId;
use kayfabe_linux_raw::memtype::{self, EffectiveMemtype, MemtypeError};
use kayfabe_linux_raw::{CachePolicy, HostPageSize};
use kayfabe_mmu::gpga::{
    ObjectId, UnwitnessedTransport, ViewFault, ViewState, ViewUpdate, ViewerId, ViewerIndex,
};
use kayfabe_vmm::{Prot, Vmm, VmmError};

use crate::slots::{OUR_SLOT_BUDGET, Tier};
use crate::{QemuMachine, RamRegionId, WindowSpec};

// ─────────────────────────────────────────────────────────────────────────────────────
// ★★★ The CPU optimization bits — memory type, per window, with its reason
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★★ **What memory type a framebuffer window must be, and who actually decides.**
///
/// Four things decide the memory type of one guest access, and only the first is ours:
///
/// | # | decider | held by |
/// |---|---|---|
/// | 1 | the host userspace page-table entry | **us** — the backing this module installs |
/// | 2 | the host's second-dimension tables (EPT/NPT) | the kernel, from the backing's class |
/// | 3 | the guest's own page-table entry | the guest driver |
/// | 4 | what the device needs | the hardware |
///
/// ⚠ **Deciders 2 and 3 combine differently on the two x86 vendors.** Intel EPT sets `IPAT`
/// for a normal-memory backing and the guest's entry is ignored; AMD NPT has no `IPAT` and
/// honours it. So the same host mapping yields a different guest-effective type on the two
/// vendors — which is why a request must never be read as an outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowCacheability {
    /// What the host-side backing must be for this window.
    pub host: CachePolicy,
    /// ★ Whether the guest-effective type is ours to determine at all.
    pub guest_effective: GuestEffective,
    /// One clause naming why, for a diagnostic. Never branched on.
    pub because: &'static str,
}

/// Whether this port controls what the *guest* sees, or merely what the host mapping is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GuestEffective {
    /// ★★ **Vendor-dependent, and not ours.** The guest chose a type when it mapped the
    /// containing base-address register; whether that choice survives depends on which
    /// x86 vendor's second-dimension paging is underneath. ⊘ No userspace instrument can
    /// read it — it must be measured *in the guest* or left unclaimed. Nothing in this port
    /// measures it today (2026-07-31), and no code here claims it.
    DecidedByTheGuestAndTheVendor,
}

/// ★★★ **The per-window requirement, decided here so it is one statement rather than three
/// scattered assumptions.**
///
/// All three windows resolve to the same **host-side** answer, and the reasons are not the
/// same, which is why this is a `match` with three arms and not a constant.
#[must_use]
pub const fn cacheability_of(window: FbWindow) -> WindowCacheability {
    match window {
        // PRAMIN is a 1 MiB aperture *inside the register base-address register*. The guest
        // driver maps that whole register aperture uncached, because for a register the
        // access IS the transaction and a cached poll never observes the bit it waits for.
        // PRAMIN's own bytes are device MEMORY rather than registers, but the guest does not
        // get to choose a second type for a sub-range of a mapping it made once.
        FbWindow::Pramin => WindowCacheability {
            host: CachePolicy::WriteBack,
            guest_effective: GuestEffective::DecidedByTheGuestAndTheVendor,
            because: "PRAMIN is a window inside the register aperture, which the guest maps \
                      uncached for the whole aperture; our backing is ordinary memory and \
                      must be write-back, and whether the guest's uncached choice survives \
                      is the vendor's decision, not ours",
        },
        // The framebuffer aperture is the streaming-store window. Write-combining is the
        // right attribute for the guest's mapping of it; it is NOT attainable for a
        // page-cache backing, and asking for it here would be refused by the raw layer.
        FbWindow::FbAperture => WindowCacheability {
            host: CachePolicy::WriteBack,
            guest_effective: GuestEffective::DecidedByTheGuestAndTheVendor,
            because: "the framebuffer aperture wants write-combining in the GUEST, which no \
                      page-cache backing can supply on the host side; the host mapping is \
                      write-back and the guest's own attribute is the one that governs its \
                      stores",
        },
        // The instance window carries page tables and instance blocks. It is read and
        // written by the CPU in small pieces, so cached is exactly right for the host side.
        FbWindow::InstanceWindow => WindowCacheability {
            host: CachePolicy::WriteBack,
            guest_effective: GuestEffective::DecidedByTheGuestAndTheVendor,
            because: "the instance window carries page tables and instance blocks, touched by \
                      the CPU in small pieces; a cached host backing is what that traffic \
                      wants and ordinary memory supplies it",
        },
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────
// Page size — stated, because a 12 GiB framebuffer at 4 KiB is three million entries
// ─────────────────────────────────────────────────────────────────────────────────────

/// The second-dimension page size a run can reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HugePages {
    /// A 2 MiB entry is reachable: the run is long enough and the guest-physical and host
    /// addresses are **congruent** modulo 2 MiB.
    Reachable,
    /// It is not, and this is why. ★ A named reason rather than a `false`, because the two
    /// causes have different fixes: a short run can never be helped, while an incongruent
    /// one is an allocator decision somebody could change.
    OutOfReach(&'static str),
}

/// ★★ **2 MiB.** The smallest second-dimension entry above the base page on x86-64.
pub const HUGE_PAGE_BYTES: u64 = 2 << 20;

/// ★★★ **Is a 2 MiB entry reachable for this run?**
///
/// Three conditions, all necessary, and the third is the one that is silently lost:
///
/// 1. the run is at least 2 MiB long;
/// 2. its guest-physical base is 2 MiB aligned;
/// 3. ★ its **host** address is congruent to its guest-physical address modulo 2 MiB.
///
/// Condition 3 has no error path anywhere. A memslot whose `userspace_addr` and
/// `guest_phys_addr` differ modulo 2 MiB installs perfectly and is simply never promoted:
/// the kernel cannot build one entry describing two differently-offset ranges. The cost is
/// paid in entries and in walks, and nothing reports it. That is why it is a function with
/// a name here rather than a comment somewhere.
#[must_use]
pub const fn huge_pages_for(gpa: u64, host_off: u64, len: u64) -> HugePages {
    if len < HUGE_PAGE_BYTES {
        return HugePages::OutOfReach(
            "the run is shorter than a 2 MiB entry, so no entry above the base page can \
             describe it — this is the whole of PRAMIN's case, whose window is 1 MiB",
        );
    }
    if !gpa.is_multiple_of(HUGE_PAGE_BYTES) {
        return HugePages::OutOfReach(
            "the guest-physical base is not 2 MiB aligned, so a large entry would have to \
             begin before the run does",
        );
    }
    if !(gpa ^ host_off).is_multiple_of(HUGE_PAGE_BYTES) {
        return HugePages::OutOfReach(
            "the host and guest-physical addresses are not congruent modulo 2 MiB; the \
             mapping installs and is silently never promoted, which costs entries and walks \
             and reports nothing",
        );
    }
    HugePages::Reachable
}

/// What one installed run's geometry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MappingGeometry {
    /// Where in guest-physical space.
    pub gpa: u64,
    /// How long.
    pub len: u64,
    /// The host base page — every mapping gets at least this.
    pub page: HostPageSize,
    /// Whether anything larger is reachable.
    pub huge: HugePages,
}

// ─────────────────────────────────────────────────────────────────────────────────────
// Backing — and the one thing this port cannot obtain
// ─────────────────────────────────────────────────────────────────────────────────────

/// Where an object's bytes physically are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostBacking {
    /// ★ Host memory this virtual machine monitor minted, named by its offset into the
    /// window's own reservation. This is the correct backing for emulated device memory:
    /// the page tables and instance blocks a guest writes through `PRAMIN` and the instance
    /// window live in *emulated* video memory and never need to be on a real device.
    VmmOwned {
        /// Byte offset into the window's reservation.
        offset: u64,
    },
    /// ⊘ **The real device's own framebuffer.** Obtaining this means an isolate opening
    /// `/dev/nvidia*`, mapping the aperture, and passing the descriptor up the crossing —
    /// and **no verb does that yet**, deliberately. Returned by a backing source that knows
    /// it needs one; refused by name at [`ViewInstaller::drain_and_install`].
    HostGpuFramebuffer,
}

/// The backing source does not know this object at all.
///
/// ★ A named type rather than `()`: *"I have never heard of this object"* and *"its bytes are
/// on the device"* are different answers with different consequences, and a unit error would
/// have collapsed them into one at the seam where they are told apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackingUnknown;

/// Where the installer asks what backs an object.
///
/// ★ A trait rather than a map, because the answer is the *emulated device model's* to give
/// and this crate must not contain a device model. It is the seam, and it is the only thing
/// the installer needs from outside the index.
pub trait ObjectBacking {
    /// What backs `object`, whose bytes begin at GPGA `gpga_base` in `aperture`.
    ///
    /// # Errors
    /// A backing source that does not know the object at all says so; the installer turns
    /// that into [`InstallRefusal::NoBackingForObject`] rather than guessing an address.
    fn backing_for(
        &self,
        object: ObjectId,
        aperture: Aperture,
        gpga_base: u64,
        len: u64,
    ) -> Result<HostBacking, BackingUnknown>;
}

// ─────────────────────────────────────────────────────────────────────────────────────
// Refusals
// ─────────────────────────────────────────────────────────────────────────────────────

/// Every way this installer refuses. All loud; none is a fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallRefusal {
    /// ⊘ **The finding.** An object's bytes live in the real device's framebuffer, which
    /// needs a descriptor only the isolate can mint and a verb that does not exist. Named
    /// rather than approximated: a mapping of the wrong bytes is worse than no mapping.
    HostGpuBackingHasNoVerb {
        /// Which object.
        object: ObjectId,
        /// Which aperture its bytes are in.
        aperture: Aperture,
    },
    /// The backing source does not know this object.
    NoBackingForObject {
        /// Which object.
        object: ObjectId,
    },
    /// ★★ Content in this run arrived by a transport this port cannot observe, so no view of
    /// it can be vouched for. The run is demoted to [`Tier::Observe`] and the caller is told;
    /// it is a refusal to make it native, not a refusal to proceed.
    UnwitnessedRunNotMadeNative {
        /// Where in the view.
        view_off: u64,
        /// How long.
        len: u64,
        /// Which transport.
        transport: UnwitnessedTransport,
    },
    /// ★★ The layout needs more memslots than this device may hold. Both numbers, because
    /// "too many" without the ceiling is not actionable.
    SlotBudgetWouldBeExceeded {
        /// How many runs the layout has.
        needed: usize,
        /// How many this device may hold.
        budget: u32,
    },
    /// ★★★ The mapping installed, and the memory type the kernel gave it is not the one the
    /// window requires. **Read back, not assumed.**
    EffectiveCacheabilityDiffers {
        /// Which window.
        window: FbWindow,
        /// What it must be.
        required: CachePolicy,
        /// What the kernel actually installed.
        effective: CachePolicy,
    },
    /// The index refused.
    Index(ViewFault),
    /// The hypervisor or the kernel refused.
    Vmm(VmmError),
}

impl From<ViewFault> for InstallRefusal {
    fn from(f: ViewFault) -> Self {
        Self::Index(f)
    }
}

impl From<VmmError> for InstallRefusal {
    fn from(e: VmmError) -> Self {
        Self::Vmm(e)
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────
// The layout — the pure translation, per object and never per page
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★★ **THE MERGE KEY — every property one mapping encodes.**
///
/// Two runs may be consolidated into one mapping **only** when they are contiguous in both
/// the view and the host backing *and* their keys are equal. Contiguity alone is not
/// sufficient and never was; see this module's docs for what each field's flattening would
/// cost.
///
/// ★ It derives `PartialEq` and nothing branches on individual fields, so adding a property
/// to what a mapping encodes means adding a field here and every merge site tightens at
/// once. That is the property worth having: the key is one place, not a conjunction spread
/// across a comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeKey {
    /// Whose handle namespace the objects live in. Merging across it makes one mapping with
    /// two owners — boundary 2.
    pub owner: IsolateId,
    /// The memory type this run must have. Merging across it silently mistypes one half.
    pub cache: CachePolicy,
    /// The protection. Merging across it hands out the stronger one.
    pub prot: Prot,
    /// Whether every byte arrived by a transport this port observes. Merging a vouchable
    /// run with an unvouchable one would make the unvouchable half native.
    pub witnessed: bool,
    /// ★★★ **Who else can see these bytes**, ascending and deduplicated — read from
    /// `ViewerIndex::viewers_of`, which is the index's own answer to exactly this question.
    /// Merging across it is a cross-viewer leak manufactured by an optimisation.
    pub viewers: Vec<ViewerId>,
}

/// One maximal run of the window that a memslot will serve natively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveredRun {
    /// Offset into the window.
    pub view_off: u64,
    /// Length.
    pub len: u64,
    /// Offset into the window's host reservation. ★ Congruence between this and the
    /// guest-physical address is what [`huge_pages_for`] checks.
    pub host_off: u64,
    /// Which objects this run is made of, ascending. ★ Plural: consolidation is the point,
    /// and keeping the list is what lets a revocation find the run it belongs to without a
    /// reverse address lookup.
    pub objects: Vec<ObjectId>,
    /// ★★★ The key every object in this run shares. Carried on the run, not recomputed, so
    /// a test can assert the *reason* two runs merged rather than that they did.
    pub key: MergeKey,
}

/// What one window should look like, computed from drained updates.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewLayout {
    /// The runs a memslot will serve.
    pub covered: Vec<CoveredRun>,
    /// The runs that get **no memslot** — uncovered, or covered by unvouchable content.
    pub observe: Vec<Range<u64>>,
    /// ★ How many object runs went into [`Self::covered`] before coalescing. The ratio is
    /// the whole argument for per-object granularity, and it is a number rather than a
    /// claim: a layout that coalesced 400 objects into 3 runs costs 3 memslots.
    pub coalesced_from: usize,
}

impl ViewLayout {
    /// How many mappings this layout needs — one per merged run.
    ///
    /// ⚠ Not the memslot count; see [`InstallReport::memslots`] for why the two differ and
    /// why the difference is safe.
    #[must_use]
    pub fn mappings(&self) -> usize {
        self.covered.len()
    }

    /// The tier list, in the shape [`WindowSpec`] takes.
    ///
    /// ⚠ **A coordinate change, and it is the kind that is silently wrong.** [`Self::observe`]
    /// is in *window* offsets, because that is the space the index speaks and the space a
    /// test can read. `WindowSpec::observe` is in **guest-physical** addresses. Converting
    /// here, in one named place, is the alternative to converting at each call site and
    /// getting one of them wrong.
    #[must_use]
    pub fn observe_ranges(&self, window_gpa: u64) -> Vec<Range<u64>> {
        self.observe
            .iter()
            .map(|r| window_gpa + r.start..window_gpa + r.end)
            .collect()
    }
}

/// One object's placement in the window, as the installer tracks it between drains.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Placed {
    len: u64,
    host_off: u64,
    object: ObjectId,
    /// ★★★ Everything a mapping of this object would encode. Captured at fold time, when
    /// the region that produced it is still in hand — recomputing it later would need a
    /// reverse address lookup, which is the thing the governing rule forbids.
    key: MergeKey,
    /// ★★ Set when a [`ViewUpdate::Unwitnessed`] covered this run. A placed-but-unvouchable
    /// run is **not** removed — the object is still there and still owns the address — it is
    /// simply not made native.
    unwitnessed: Option<UnwitnessedTransport>,
}

/// ★★ **The alignment the index actually hands us**, counted rather than hoped for.
///
/// Huge pages are not a thing this installer can request; they are a thing the geometry
/// either permits or does not. So the honest report is a census of what arrived, and if it
/// says every run is 4 KiB-aligned and short then huge pages are unreachable **today** and
/// that is a finding about the allocator upstream, not about this code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AlignmentCensus {
    /// Runs whose guest-physical base is 2 MiB aligned.
    pub huge_aligned: usize,
    /// Runs whose host and guest-physical addresses are congruent modulo 2 MiB — the
    /// condition with no error path anywhere.
    pub congruent: usize,
    /// Runs at least 2 MiB long.
    pub long_enough: usize,
    /// Runs where all three hold, so a 2 MiB entry is actually reachable.
    pub huge_reachable: usize,
    /// How many runs were examined. ★ The denominator, without which the others are not
    /// numbers.
    pub runs: usize,
}

// ─────────────────────────────────────────────────────────────────────────────────────
// The installer
// ─────────────────────────────────────────────────────────────────────────────────────

/// What one drain-and-install did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InstallReport {
    /// How many updates were pulled out of the index.
    pub updates_drained: usize,
    /// How many objects the window now shows.
    pub objects: usize,
    /// ★★★ How many **mappings** the coverage needs after consolidation — one per merged
    /// [`CoveredRun`], which is one `MAP_FIXED` placement. This is the number the
    /// [`MergeKey`] governs.
    pub mappings: usize,
    /// ★★ How many mappings the same coverage would have needed **before** consolidation —
    /// one per vouchable object. The pair is the consolidation ratio, measured end to end in
    /// `crates/kayfabe-vmm-qemu/tests/viewer_install.rs`.
    pub mappings_before_merge: usize,
    /// ★★★ How many **memslots** the kernel actually holds for this window afterwards.
    ///
    /// ⚠ **This is NOT the mapping count, and conflating them was a real mistake in this
    /// module's first draft.** A memslot answers *"is this guest-physical range backed by
    /// host memory, or does it trap?"* — so two adjacent mappings with **no observe hole
    /// between them** are one contiguous backed range and the plane installs **one** slot
    /// for both. What is inside that range, and who owns it, is decided by the `MAP_FIXED`
    /// placements the mappings become, and *those* are what the merge key keeps apart.
    ///
    /// ★ So the key is not weakened by the collapse: merging two memslots loses only the
    /// boundary between two ranges that are both native anyway. Merging two *placements*
    /// across a key difference is the leak, and that never happens.
    pub memslots: usize,
    /// The geometry of each covered run, in order.
    pub geometry: Vec<MappingGeometry>,
    /// ★★ What alignment the index actually handed us, and therefore whether huge pages are
    /// reachable at all.
    pub alignment: AlignmentCensus,
    /// Runs demoted to [`Tier::Observe`] because nothing here could vouch for their bytes.
    pub unwitnessed: Vec<InstallRefusal>,
    /// ★ Whether the window's coverage actually changed. A drain that changed nothing must
    /// not reinstall anything: a reinstall is a `KVM_SET_USER_MEMORY_REGION` storm, and the
    /// index's own generation counter moves for reasons a window does not care about.
    pub reinstalled: bool,
}

/// ★★★ **The drainer.** One per window; owns that window's reservation and keeps its
/// memslots equal to what the index says the window shows.
#[derive(Debug)]
pub struct ViewInstaller {
    window: FbWindow,
    viewer: ViewerId,
    gpa: u64,
    len: u64,
    page: HostPageSize,
    /// view offset -> what is placed there. `BTreeMap`, so the layout comes out ascending
    /// without a sort and adjacency is a `windows(2)` over the iteration.
    placed: BTreeMap<u64, Placed>,
    region: Option<RamRegionId>,
    last: ViewLayout,
}

impl ViewInstaller {
    /// A drainer for `viewer`, whose window occupies `[gpa, gpa + len)`.
    ///
    /// ★ It installs nothing yet. A window with no coverage shows nothing, and the honest
    /// initial state is *no memslot at all* — every address in it traps until the index says
    /// otherwise. That is miss = fault applied to the data plane.
    #[must_use]
    pub fn new(window: FbWindow, viewer: ViewerId, gpa: u64, len: u64, page: HostPageSize) -> Self {
        Self {
            window,
            viewer,
            gpa,
            len,
            page,
            placed: BTreeMap::new(),
            region: None,
            last: ViewLayout::default(),
        }
    }

    /// Which window this drains.
    #[must_use]
    pub const fn window(&self) -> FbWindow {
        self.window
    }

    /// What this window's memory type must be, and why.
    #[must_use]
    pub const fn cacheability(&self) -> WindowCacheability {
        cacheability_of(self.window)
    }

    /// The layout as of the last drain.
    #[must_use]
    pub const fn layout(&self) -> &ViewLayout {
        &self.last
    }

    /// ★★★ **DRAIN, then make it true.**
    ///
    /// 1. **Pull** from the index — [`ViewerIndex::drain`], or, if the viewer went
    ///    [`ViewState::Desynced`], rebuild from [`ViewerIndex::view_contents`], which is the
    ///    index's own stated contract for that state.
    /// 2. Fold the updates into this window's placement map. Per **object**.
    /// 3. Compute the tier layout, coalescing adjacent runs whose host backing is also
    ///    adjacent.
    /// 4. If it changed, reinstall the window's reservation with that tier list.
    ///
    /// ⊘ Step 4 does **not** run when the layout is unchanged. A drain is cheap; a memslot
    /// reinstall is a syscall per run and a teardown of everything the guest was using.
    ///
    /// # Errors
    /// [`InstallRefusal::HostGpuBackingHasNoVerb`] — the finding; the object's bytes are on
    /// the real device and no verb fetches them. [`InstallRefusal::SlotBudgetWouldBeExceeded`],
    /// [`InstallRefusal::NoBackingForObject`], and whatever the index or the kernel refused.
    ///
    /// ★ [`InstallRefusal::UnwitnessedRunNotMadeNative`] is **not** returned — it is
    /// collected into [`InstallReport::unwitnessed`], because it is a per-run demotion and
    /// not a failure of the whole drain.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1), by way of the machine's own assertions.
    pub fn drain_and_install(
        &mut self,
        index: &mut ViewerIndex,
        backing: &dyn ObjectBacking,
        machine: &QemuMachine,
    ) -> Result<InstallReport, InstallRefusal> {
        let mut report = InstallReport::default();

        // ── 1. PULL. ────────────────────────────────────────────────────────────────
        let updates = if index.viewer_state(self.viewer)? == ViewState::Desynced {
            // ★ The queue overflowed, so it is no longer the authority. The index still is,
            // and it says so by name; rebuilding from it is the contract, and the
            // placements built so far must go first or a stale one survives the rebuild.
            self.placed.clear();
            let full = index.view_contents(self.viewer)?;
            index.resynced(self.viewer)?;
            let _ = index.drain(self.viewer)?;
            full
        } else {
            index.drain(self.viewer)?
        };
        report.updates_drained = updates.len();

        // ── 2. FOLD, per object. ────────────────────────────────────────────────────
        for u in &updates {
            match *u {
                ViewUpdate::Shows {
                    view_off,
                    region,
                    occupant,
                } => {
                    let host = backing
                        .backing_for(occupant.object, region.aperture, region.base, region.len)
                        .map_err(|BackingUnknown| InstallRefusal::NoBackingForObject {
                            object: occupant.object,
                        })?;
                    let offset = match host {
                        HostBacking::VmmOwned { offset } => offset,
                        // ⊘ THE REFUSAL. Named, and the whole drain stops: a window that is
                        // half real and half absent is a state neither side can reason about.
                        HostBacking::HostGpuFramebuffer => {
                            return Err(InstallRefusal::HostGpuBackingHasNoVerb {
                                object: occupant.object,
                                aperture: region.aperture,
                            });
                        }
                    };
                    // ★★★ The key, captured HERE, while the GPGA region that produced this
                    // update is still in hand. `viewers_of` is the index's own answer to
                    // "who else can see this" — the same question the merge must not
                    // flatten, asked of the structure that exists to answer it.
                    let mut viewers: Vec<ViewerId> = index
                        .viewers_of(region)
                        .into_iter()
                        .map(|s| s.viewer)
                        .collect();
                    viewers.sort_unstable();
                    viewers.dedup();
                    let key = MergeKey {
                        owner: occupant.owner,
                        cache: cacheability_of(self.window).host,
                        prot: Prot::ReadWrite,
                        witnessed: true,
                        viewers,
                    };
                    self.placed.insert(
                        view_off,
                        Placed {
                            len: region.len,
                            host_off: offset,
                            object: occupant.object,
                            key,
                            unwitnessed: None,
                        },
                    );
                }
                ViewUpdate::Revoked { view_off, .. } => {
                    self.placed.remove(&view_off);
                }
                ViewUpdate::Unwitnessed {
                    view_off,
                    region,
                    transport,
                } => {
                    // ★★ Mark, never drop. The object is still mapped there; what is gone is
                    // our ability to vouch for its bytes, and that costs it its memslot.
                    if let Some(p) = self.placed.get_mut(&view_off) {
                        p.unwitnessed = Some(transport);
                    }
                    report
                        .unwitnessed
                        .push(InstallRefusal::UnwitnessedRunNotMadeNative {
                            view_off,
                            len: region.len,
                            transport,
                        });
                }
            }
        }
        report.objects = self.placed.len();

        // ── 3. LAYOUT — coalesce per object, never per page. ────────────────────────
        let layout = self.compute_layout();
        report.mappings = layout.mappings();
        report.mappings_before_merge = layout.coalesced_from;
        let budget = OUR_SLOT_BUDGET;
        // ★ Checked against the MAPPING count, which is an upper bound on the memslot
        // count (adjacent mappings collapse, never split). Refusing on the bound is the
        // conservative direction: it can refuse a layout that would have fitted, and it can
        // never install one that does not.
        if layout.mappings() > budget as usize {
            return Err(InstallRefusal::SlotBudgetWouldBeExceeded {
                needed: layout.mappings(),
                budget,
            });
        }
        report.geometry = layout
            .covered
            .iter()
            .map(|r| MappingGeometry {
                gpa: self.gpa + r.view_off,
                len: r.len,
                page: self.page,
                huge: huge_pages_for(self.gpa + r.view_off, r.host_off, r.len),
            })
            .collect();
        report.alignment = census_of(&layout, self.gpa);

        // ── 4. INSTALL, only if it changed. ─────────────────────────────────────────
        if layout == self.last {
            report.memslots = usize::try_from(machine.audit().live_memslots).unwrap_or(usize::MAX);
            self.last = layout;
            return Ok(report);
        }
        if let Some(old) = self.region.take() {
            machine.remove_window(old)?;
        }
        if !layout.covered.is_empty() {
            let spec = WindowSpec {
                gpa: self.gpa,
                len: self.len,
                observe: layout.observe_ranges(self.gpa),
            };
            self.region = Some(machine.install_tiered_window(&spec)?);
        }
        report.reinstalled = true;
        report.memslots = usize::try_from(machine.audit().live_memslots).unwrap_or(usize::MAX);
        self.last = layout;
        Ok(report)
    }

    /// ★★★ **Assert the effective memory type of what was installed — do not assume the
    /// request took.**
    ///
    /// [`kayfabe_linux_raw::cache`] refuses a request the backing *cannot* satisfy, before
    /// the syscall. That is a check on our own intent. This is the other half: it reads the
    /// kernel's own record of what it *did*, for the host-physical range the backing
    /// occupies, and refuses a disagreement.
    ///
    /// ⚠ **What it cannot see, said plainly.** It observes decider 1, the host mapping. The
    /// guest-effective type is deciders 1, 2 and 3 combined and it differs by CPU vendor
    /// ([`GuestEffective`]). An `Ok` here means *the host half held*, and nothing more.
    ///
    /// # Errors
    /// [`InstallRefusal::EffectiveCacheabilityDiffers`] when the kernel installed a
    /// different type. ★ An **unreadable** instrument is `Ok(None)` — "I could not tell" is
    /// not "it is wrong", and a host without `debugfs` must not read as a broken one.
    pub fn assert_effective_cacheability(
        &self,
        host_phys: u64,
        len: u64,
    ) -> Result<Option<EffectiveMemtype>, InstallRefusal> {
        let want = self.cacheability().host;
        match memtype::require_effective(want, host_phys, len) {
            Ok(m) => Ok(Some(m)),
            Err(MemtypeError::Downgraded { effective, .. }) => {
                Err(InstallRefusal::EffectiveCacheabilityDiffers {
                    window: self.window,
                    required: want,
                    effective,
                })
            }
            // The instrument is absent. Honest gap, not a pass and not a failure.
            Err(_) => Ok(None),
        }
    }

    /// Place one object's **content** inside the reservation.
    ///
    /// ★★ This costs **no memslot** — it is a `MAP_FIXED` inside a range one already covers
    /// (`kayfabe_vmm::Vmm::map_guest`). That is the whole reason the slot count is per
    /// covered run and not per object.
    ///
    /// # Errors
    /// Whatever the placement refused.
    pub fn place_content(
        &self,
        machine: &QemuMachine,
        view_off: u64,
        len: u64,
        backing: kayfabe_vmm::HostRegion,
    ) -> Result<(), InstallRefusal> {
        let mut vmm = machine.vmm();
        vmm.map_guest(self.gpa + view_off, len, backing, Prot::ReadWrite)?;
        Ok(())
    }

    /// ★★★ **Consolidate placements into the fewest mappings that cover the coverage.**
    ///
    /// Per **object**, never per page. Two placements merge when **all three** hold:
    ///
    /// 1. they are contiguous in the window;
    /// 2. they are contiguous in the host reservation — a memslot is one contiguous host
    ///    range, so two objects that abut in the view but not in host memory cannot share
    ///    one however adjacent they look;
    /// 3. ★★★ their [`MergeKey`]s are **equal**. Contiguity is not a licence.
    ///
    /// ⊘ Nothing here enumerates a page. The loop is over placements, so its cost is
    /// proportional to objects, which is the ruling.
    fn compute_layout(&self) -> ViewLayout {
        let mut covered: Vec<CoveredRun> = Vec::new();
        let mut n = 0usize;
        for (&view_off, p) in &self.placed {
            // ⊘ An unvouchable run never becomes a memslot. It falls through to `observe`
            // below by simply not being added here.
            if p.unwitnessed.is_some() {
                continue;
            }
            n += 1;
            match covered.last_mut() {
                Some(last) if mergeable(last, view_off, p.host_off, &p.key) => {
                    last.len += p.len;
                    last.objects.push(p.object);
                }
                _ => covered.push(CoveredRun {
                    view_off,
                    len: p.len,
                    host_off: p.host_off,
                    objects: vec![p.object],
                    key: p.key.clone(),
                }),
            }
        }
        // Everything the covered runs do not reach gets NO memslot.
        let mut observe: Vec<Range<u64>> = Vec::new();
        let mut at = 0u64;
        for r in &covered {
            if r.view_off > at {
                observe.push(at..r.view_off);
            }
            at = r.view_off + r.len;
        }
        if at < self.len {
            observe.push(at..self.len);
        }
        ViewLayout {
            covered,
            observe,
            coalesced_from: n,
        }
    }
}

/// ★★★ **May the run `prev` absorb a placement at `view_off` / `host_off` with key `key`?**
///
/// The **entire** merge condition, in one place with a name, so that every axis of it is
/// drivable by a test rather than buried inside a `match` guard. That matters more than it
/// looks: a merge that is wrong produces a *smaller* mapping count, which is what a naive
/// test is checking for, so the condition itself has to be the thing under test.
///
/// Three clauses, all necessary:
///
/// 1. contiguous in the **view** — a gap is a gap;
/// 2. contiguous in the **host backing** — a memslot names one contiguous host range, so two
///    objects that abut in the view but not in host memory can never share one;
/// 3. ★★★ **equal [`MergeKey`]s** — see the module docs for what each field's flattening
///    would cost. This is the clause that is a security property rather than a correctness
///    one, and it is the clause an optimisation is tempted to drop.
#[must_use]
pub fn mergeable(prev: &CoveredRun, view_off: u64, host_off: u64, key: &MergeKey) -> bool {
    prev.view_off + prev.len == view_off && prev.host_off + prev.len == host_off && prev.key == *key
}

/// ★★ **Count the alignment the index actually handed us**, per merged run.
///
/// ⊘ Not a check and not a refusal — a census. Huge pages are not requestable; they are
/// permitted or not by geometry this installer does not choose, so the honest output is the
/// distribution and the conclusion a reader can draw from it.
#[must_use]
pub fn census_of(layout: &ViewLayout, gpa_base: u64) -> AlignmentCensus {
    let mut c = AlignmentCensus {
        runs: layout.covered.len(),
        ..AlignmentCensus::default()
    };
    for r in &layout.covered {
        let gpa = gpa_base + r.view_off;
        if gpa.is_multiple_of(HUGE_PAGE_BYTES) {
            c.huge_aligned += 1;
        }
        if (gpa ^ r.host_off).is_multiple_of(HUGE_PAGE_BYTES) {
            c.congruent += 1;
        }
        if r.len >= HUGE_PAGE_BYTES {
            c.long_enough += 1;
        }
        if huge_pages_for(gpa, r.host_off, r.len) == HugePages::Reachable {
            c.huge_reachable += 1;
        }
    }
    c
}

/// The tier a run of this window is in — the answer to *"would a guest access here exit?"*,
/// derivable without a guest.
///
/// ★ Exposed so a test can ask the question positively rather than inferring it from a
/// memslot record. All three of [`Tier`]'s answers are not reachable here: this installer
/// never emits [`Tier::ReadNative`], because per-object read-only protection inside a
/// read-write window is refused by the plane itself, and saying so is better than leaving a
/// reader to wonder which arm is dead.
#[must_use]
pub fn tier_at(layout: &ViewLayout, view_off: u64) -> Tier {
    for r in &layout.covered {
        if view_off >= r.view_off && view_off < r.view_off + r.len {
            return Tier::Passthrough;
        }
    }
    Tier::Observe
}

kayfabe_util::assert_send_sync!(
    ViewInstaller,
    ViewLayout,
    CoveredRun,
    InstallReport,
    InstallRefusal,
    MappingGeometry,
    AlignmentCensus,
    MergeKey,
    HugePages,
    HostBacking,
    WindowCacheability,
    GuestEffective,
);
