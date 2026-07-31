//! # `gpga` — the GPGA **viewer index**: who can see this region, and what does it contain
//!
//! The owner's answer to `Q13` (`open_questions_for_the_owner.md:515`), decided
//! 2026-07-31, in his words:
//!
//! > Each framebuffer window (`PRAMIN`, `BAR1`, `BAR2`) has its own mappings in GPGA, and
//! > the same GPGA can be mapped in multiple windows. Whenever a GPGA object or page is
//! > allocated, deallocated or remapped, the system asks **who can see this page** — which
//! > isolates, which windows, including **partial** maps — and updates them all, so every
//! > view is correct and **passthrough by construction**. Objects are the authority;
//! > **GPGA is the key**. The same object may be mapped many times, and **slices** of it,
//! > when a window covers only part of it.
//!
//! ## ★★★ The governing rule
//!
//! > **Never ask a single address where it belongs — always ask, per region, what it
//! > contains.**
//!
//! It is the rule the address plane already runs on ([`crate::AddressTable`]:
//! forward-populated, never reverse-resolved, miss = fault) and it is enforced here by
//! *API shape*, not by care: **there is no `fn owner_of(addr)` and there never will be.**
//! Every question this module answers is spelled `region -> contents`
//! ([`ViewerIndex::contents`]) or `region -> viewers` ([`ViewerIndex::viewers_of`]), both
//! **total** partitions in the sense of [`kayfabe_util::IntervalMap::spans`]. A caller
//! that wants one address asks about a one-byte region and gets a partition back.
//!
//! ## The three corrections that shaped this, all binding
//!
//! 1. ★★★ **The key is `(`[`Aperture`]`, address)`, never a bare address**
//!    ([`GpgaAddr`]). A bare-address key aliases vidmem offset `X` with sysmem offset `X`
//!    — the identity/uniqueness defect family that produced four bugs in one day in this
//!    repository, and the same shape as the measured client-aliasing bug where an
//!    `hClient` was taken to name a process and two CUDA processes turned out to share
//!    one. [`Aperture::Peer`] additionally carries the multi-GPU requirement.
//!    ⊘ **Known limit, not built:** `Peer` does not say *which* peer. A second GPU axis
//!    belongs in the key the day a second peer exists; it is named here rather than
//!    silently assumed away.
//! 2. ★★ **Range-keyed, never page-keyed.** A 12 GiB framebuffer is ~3 M pages; a
//!    page-keyed index makes every allocation `O(pages × viewers)`. Everything here is an
//!    [`kayfabe_util::IntervalMap`], so fan-out is proportional to **regions**. See
//!    [`ViewerIndex::viewers_of`] for the exact complexity, stated rather than implied.
//! 3. ★★ **A view that could be stale says so BY NAME** ([`Witness::Unwitnessed`]). The
//!    witness closes on every CPU write path (`211 836/211 836` cold boot,
//!    `250 041/250 041` matmul, **zero** unwitnessed bytes — measured at `ee3c808`) but
//!    **not** for a framebuffer-to-framebuffer copy-engine write, the CeUtils 512 MiB
//!    alias path `#13` was about. And the recorder cannot observe framebuffer accesses at
//!    all, so **no capture can say how often it happens**. Under miss = fault a view whose
//!    content came through that transport is a named refusal
//!    ([`ViewFault::UnwitnessedContent`]), never a silently stale view.
//!
//! ## ★★★ The lock constraint, resolved DELIBERATELY — three passes, not one
//!
//! Stage B (`379f712`) hit this wall and so does this: **the act phase holds the issuing
//! proc's rank-1 lock, and R3 forbids taking a second rank-1 lock** (`l1_concurrency.md`
//! §3.3: *"a thread may acquire only in strictly increasing rank, at most one lock per
//! rank"*). A GPGA object is visible to viewers that belong to **other** procs and other
//! isolates — precisely the asymmetry that made `kayfabe_fwd::latch_pt_writes` a separate
//! pass from `apply_pushbuffer`, and `apply_promote_ctx` a separate pass from
//! `route_promote_ctx`. So an inline update is not available, and the answer is the same
//! shape those two already have:
//!
//! | pass | rank | what it may touch |
//! |---|---|---|
//! | **DESCRIBE** — build an [`ObjectChange`] | 1 (the issuer's, already held) | plain owned data; the index is not touched at all |
//! | **PLAN** ([`ViewerIndex::plan`]) → **APPLY** ([`ViewerIndex::apply`]) | 0, the index is device-global | the index; **no viewer is contacted** |
//! | **DRAIN** ([`ViewerIndex::drain`]) | 1, **one viewer at a time** | that viewer alone |
//!
//! [`ObjectChange`] is deliberately a plain owned value carrying **IDs only, never held
//! references** — R5's rule that a route-phase product is a pre-validation *hint*. That is
//! what lets it cross a lock boundary at all.
//!
//! ★ **Why DRAIN is a pull and not a push, and what it buys.** `apply` never calls into a
//! viewer; it enqueues. A viewer that never drains therefore **cannot wedge the update**
//! for anybody else — the hanging-isolate property, tested rather than asserted. What it
//! *can* do is grow its own queue without bound, so the queue is bounded
//! ([`MAX_PENDING_UPDATES`]) and overflow marks the viewer [`ViewState::Desynced`] — a
//! named state meaning *"this view is no longer known-correct and must be rebuilt"* —
//! rather than dropping an update silently. A dropped update is exactly the stale view
//! that becomes a use-after-free.
//!
//! ★★ **The index is the authority, the queue is only a notification.** A revocation
//! removes the coverage in `apply`, so [`ViewerIndex::view_contents`] stops showing a
//! freed object **immediately**, whether or not the viewer ever drains. [`ObjectId`]s are
//! monotonic and never reused, so a stale id in an undrained queue cannot be confused with
//! a later object — the same defence `kayfabe_core::gpa::ArenaId`'s generation counter
//! gives against the recycled-range ABA.
//!
//! ## ⊘ What is NOT built in this pass, and why
//!
//! - **No consumer is wired.** By instruction: the index and its tests come first, which
//!   is when the invariant is cheap to get right.
//! - **`PRAMIN` is the window this is shaped for** — it is untranslated, so a mapping into
//!   it is arithmetic on a register the guest itself sets, needing no GMMU walk and no
//!   `FbRead`. `BAR1`/`BAR2` are GMMU-translated; the index holds their coverage
//!   perfectly well (a translated window is just a set of regions), but **nothing here
//!   derives that coverage from page tables** — that is a walker job and a second pass.
//! - **No byte is stored.** This index says *who sees what*, never *what it says*, exactly
//!   as [`crate::walker::FbRead`] has no method that hands content in.

use std::collections::{BTreeMap, VecDeque};

use kayfabe_arch::{Aperture, FbWindow};
use kayfabe_isolate::IsolateId;
use kayfabe_util::IntervalMap;

use crate::{HostExtent, HostSlice, HostSliceError};

/// ★ **The bound on one viewer's undelivered updates.**
///
/// A viewer that stops draining must not be able to grow the index without limit — that
/// is a memory-exhaustion path with a guest-reachable trigger. On overflow the viewer
/// becomes [`ViewState::Desynced`], which is louder than dropping an update and cheaper
/// than blocking the applier. The number is a policy dial, not a fact about hardware.
pub const MAX_PENDING_UPDATES: usize = 4096;

// ─────────────────────────────────────────────────────────────────────────────────────
// Addresses and regions — the key, and the only thing you may ask about
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★★ **A GPGA address: `(aperture, address)`, and NEVER a bare `u64`.**
///
/// See the module docs, correction 1. This type exists so that "I forgot the aperture" is
/// a compile error rather than an aliasing bug found months later in a fault handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GpgaAddr {
    /// Which physical aperture `addr` is an offset in.
    pub aperture: Aperture,
    /// The guest-physical GPU address inside that aperture.
    pub addr: u64,
}

impl GpgaAddr {
    /// An address in an aperture.
    #[must_use]
    pub const fn new(aperture: Aperture, addr: u64) -> Self {
        Self { aperture, addr }
    }
}

/// Why a [`GpgaRegion`] was refused at construction. Two variants because they are two
/// different mistakes (the same split [`HostSliceError`] makes, for the same reason).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionError {
    /// `len == 0`. A region of no bytes is not a region; nothing can be inside it, and
    /// "who can see nothing" is not a question with an answer.
    Empty,
    /// `base + len` wraps `u64`. Guest-influenced arithmetic refuses rather than panicking
    /// (boundary-1 posture).
    Wraps,
}

/// ★★★ **A region of GPGA — the unit every question is asked in.**
///
/// The governing rule (*never ask a single address where it belongs*) is enforced by
/// making this, not [`GpgaAddr`], the argument of every query. Fields are public because
/// the type is a coordinate triple with no ownership meaning, but the constructor is the
/// only way to get an *unchecked-free* one: [`GpgaRegion::new`] refuses the two ranges
/// that are not ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GpgaRegion {
    /// The aperture the region lives in. Half the key.
    pub aperture: Aperture,
    /// First byte.
    pub base: u64,
    /// Length in bytes. Never zero — [`GpgaRegion::new`] refuses that.
    pub len: u64,
}

impl GpgaRegion {
    /// `[base, base+len)` in `aperture`.
    ///
    /// # Errors
    /// - [`RegionError::Empty`] — `len == 0`.
    /// - [`RegionError::Wraps`] — `base + len` does not fit in `u64`.
    pub const fn new(aperture: Aperture, base: u64, len: u64) -> Result<Self, RegionError> {
        if len == 0 {
            return Err(RegionError::Empty);
        }
        if base.checked_add(len).is_none() {
            return Err(RegionError::Wraps);
        }
        Ok(Self {
            aperture,
            base,
            len,
        })
    }

    /// One past the last byte. Cannot overflow — the constructor refused that.
    #[must_use]
    pub const fn end(self) -> u64 {
        self.base + self.len
    }

    /// The region's first address, as a key.
    #[must_use]
    pub const fn start(self) -> GpgaAddr {
        GpgaAddr::new(self.aperture, self.base)
    }

    /// The overlap of two regions, or `None` if they are in different apertures or do not
    /// meet. ★ The aperture check is the whole point: two regions with identical
    /// coordinates in different apertures **do not intersect**, because they are not the
    /// same memory.
    #[must_use]
    pub fn intersect(self, other: Self) -> Option<Self> {
        if self.aperture != other.aperture {
            return None;
        }
        let base = self.base.max(other.base);
        let end = self.end().min(other.end());
        if base >= end {
            return None;
        }
        Some(Self {
            aperture: self.aperture,
            base,
            len: end - base,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────
// Objects — the authority
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★ **A GPGA object's identity — monotonic and NEVER reused.**
///
/// Reuse is what turns an undrained revocation into a use-after-free: a viewer holding a
/// stale id would silently start naming a *different* object. Same defence, same reason,
/// as the generation counter in `kayfabe_core::gpa::ArenaId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(pub u64);

/// What the index stores about one object. Private — objects are reached through region
/// queries, never by address lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ObjectEntry {
    id: ObjectId,
    owner: IsolateId,
    /// The object's OWN base and length, kept so that a partial view can name which part
    /// of the object it sees. The covering interval may be sliced by a query; the object
    /// is not.
    region: GpgaRegion,
}

/// The object occupying one run of a region, **and which part of it that run is**.
///
/// ★★ [`Occupant::extent`] is a [`HostExtent`], reused deliberately rather than
/// re-invented as a bare offset. `HostExtent` exists because *"which part"* and *"who
/// owns it"* are the same question (`gpga_address_space.md` §8.2 — and the commit that
/// introduced it, `b9500bf`, found a live double free by asking it). Here it answers:
/// [`HostExtent::Whole`] means this viewer sees the **entire** object, so an invalidation
/// retires the object as far as this viewer is concerned; [`HostExtent::Slice`] means the
/// object **outlives** this view of it and other viewers still hold the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Occupant {
    /// Which object.
    pub object: ObjectId,
    /// The isolate whose handle namespace the object lives in (§9.3's branding,
    /// inherited rather than re-invented).
    pub owner: IsolateId,
    /// ★ Which part of it — an ownership regime, not an offset.
    pub extent: HostExtent,
}

/// A transport that writes GPGA content **this port cannot observe**.
///
/// One variant today because exactly one such transport is known to exist. It is an enum
/// rather than a `bool` so the next one has somewhere to go with its own name — a
/// boolean "stale" flag would merge two different unknowns into one word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnwitnessedTransport {
    /// ★★ **A framebuffer-to-framebuffer copy-engine write** — the CeUtils 512 MiB alias
    /// path `#13` was about (`C: nvkvm_gpu_emul.c:6414-6417`, whose own comment says
    /// *"BYPASS `nvkvm_fb_write`"*). Its source is framebuffer memory, so the witness
    /// induction that closes on every CPU write path does **not** close here, and the C
    /// recorder observes no framebuffer access at all — ⊘ **no capture can say how often
    /// this happens, in either direction.**
    FbToFbCopyEngine,
}

/// Whether the content of a run is content we have seen written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Witness {
    /// Every byte of this run arrived through a transport the port observes.
    Witnessed,
    /// ★★ It did not. The bytes exist and we do not know them.
    Unwitnessed(UnwitnessedTransport),
}

/// One maximal run of a region over which the index's answer is constant — the element of
/// the `region -> contents` partition ([`ViewerIndex::contents`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpgaSpan {
    /// The run.
    pub region: GpgaRegion,
    /// What occupies it. ★ `None` is **not a miss** — it is a null page, a known state
    /// with known contents (`gpga_address_space.md` §3/§6). The miss-is-a-fault rule
    /// governs *resolution*, and this is a content question.
    pub occupant: Option<Occupant>,
    /// Whether this run's content was witnessed.
    pub witness: Witness,
}

// ─────────────────────────────────────────────────────────────────────────────────────
// Viewers
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★ **A viewer's identity.** Opaque and minted by [`ViewerIndex::add_view`], so the index
/// never has to answer "how many `PRAMIN` windows are there" — a question whose answer
/// changes the day a second GPU exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewerId(pub u32);

/// **What kind of thing can see GPGA** — the owner's *"which isolates, which windows"*, as
/// two cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ViewerKind {
    /// A CPU-visible framebuffer window: the guest's own processor reaching device memory
    /// through `PRAMIN`, `BAR1` or `BAR2`.
    Window(FbWindow),
    /// A host isolate's mapping of GPGA — a *host* process, on the far side of the
    /// cross-tenant boundary.
    Isolate(IsolateId),
}

impl ViewerKind {
    /// The isolate this view belongs to, if any.
    ///
    /// ★★ **The asymmetry is deliberate and it is the tenant axis.** A [`Self::Window`]
    /// has no isolate owner because it is the *guest's* window: under the declared access
    /// model the guest kernel is the authority for intra-VM rights, so a window showing
    /// any of its own VM's GPGA is not a boundary crossing. A [`Self::Isolate`] view is a
    /// **host** process's mapping, and one isolate reaching another's object is boundary 2
    /// — refused by name ([`ViewFault::ForeignObject`]).
    #[must_use]
    pub const fn isolate(self) -> Option<IsolateId> {
        match self {
            Self::Window(_) => None,
            Self::Isolate(id) => Some(id),
        }
    }
}

/// Whether a view is still known-correct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ViewState {
    /// Every change since registration is either delivered or queued for delivery.
    Live,
    /// ★★ The viewer stopped draining and its queue hit [`MAX_PENDING_UPDATES`]. Updates
    /// past that point were **not** applied to a queue nobody reads; the view must be
    /// rebuilt from [`ViewerIndex::view_contents`]. Naming it is the point: the
    /// alternative is dropping an update, and a dropped revocation is a use-after-free.
    Desynced,
}

/// One thing that happened to a view. Carries coordinates and ids only — never a
/// reference, never a byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewUpdate {
    /// This view now shows `occupant` over `region`, at view offset `view_off`.
    Shows {
        /// Where in the view it appears.
        view_off: u64,
        /// The GPGA run it covers.
        region: GpgaRegion,
        /// What is there, and which part of it.
        occupant: Occupant,
    },
    /// ★★ This view no longer shows the object. Emitted on free **and** on the
    /// vacated part of a remap.
    Revoked {
        /// Where in the view it used to appear.
        view_off: u64,
        /// The GPGA run it used to cover.
        region: GpgaRegion,
        /// Which object went away.
        object: ObjectId,
    },
    /// ★★ Content in this run was written by a transport the port cannot observe. The
    /// view is not wrong about *what is mapped*; it is unable to vouch for the bytes.
    Unwitnessed {
        /// Where in the view it appears.
        view_off: u64,
        /// The GPGA run affected.
        region: GpgaRegion,
        /// Which transport.
        transport: UnwitnessedTransport,
    },
}

/// A registered view: its kind, its coverage and its undelivered updates.
#[derive(Debug)]
struct Viewer {
    kind: ViewerKind,
    /// aperture -> (GPGA range -> the view offset its first byte appears at).
    ///
    /// ★ Keyed on **GPGA**, which is what makes `region -> viewers` a forward lookup
    /// rather than a scan. The consequence is that one viewer may not map the same GPGA
    /// twice ([`ViewFault::SelfAlias`]) — see [`ViewerIndex::map_into_view`].
    cover: BTreeMap<Aperture, IntervalMap<u64>>,
    pending: VecDeque<ViewUpdate>,
    state: ViewState,
}

/// Which viewer sees a run of a region, and where in that viewer it appears — the element
/// of the `region -> viewers` partition ([`ViewerIndex::viewers_of`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewerSlice {
    /// Which view.
    pub viewer: ViewerId,
    /// What kind of view it is.
    pub kind: ViewerKind,
    /// The GPGA run this viewer covers — a **sub**-range of the queried region whenever
    /// the coverage is partial. ★ This is the *"including partial maps"* clause of the
    /// design, and it is why the answer is a list of slices and not a list of ids.
    pub region: GpgaRegion,
    /// The offset within the view at which `region`'s first byte appears.
    pub view_off: u64,
}

// ─────────────────────────────────────────────────────────────────────────────────────
// Faults
// ─────────────────────────────────────────────────────────────────────────────────────

/// Every way this index refuses. All LOUD: callers propagate, they never guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewFault {
    /// The named viewer is not registered (or was removed).
    UnknownViewer(ViewerId),
    /// The named object is not in the index.
    UnknownObject(ObjectId),
    /// A region argument was malformed. Guest-influenced, so a clean refusal.
    BadRegion(RegionError),
    /// An object was allocated overlapping a live object. The GPGA allocator must free
    /// first; silently replacing would hide the collision class `#14` is about.
    ObjectOverlap {
        /// The region that was refused.
        region: GpgaRegion,
    },
    /// ★ **One viewer tried to map the same GPGA at two of its own offsets.**
    ///
    /// Refused, and the refusal is a design position rather than an implementation limit:
    /// the decided design says *"the same GPGA can be mapped in multiple windows"*, not
    /// *"twice in one window"*, and a self-alias makes the viewer's own coverage
    /// ambiguous about which of its offsets an invalidation refers to. ⊘ Lifting this
    /// would mean replacing the per-viewer [`IntervalMap`] with an interval **tree**;
    /// that is a container change, and it is named here so nobody has to rediscover it.
    SelfAlias {
        /// The viewer that already covers it.
        viewer: ViewerId,
        /// The region it already covers.
        region: GpgaRegion,
    },
    /// ★★ **THE DUAL, refused: an isolate view would have seen another isolate's
    /// object.**
    ///
    /// Boundary 2 (`gpga_address_space.md` §9.1: RM grants *objects*, not ranges, so an
    /// isolate holding a handle can map any offset in it). Refused whole — an allocation
    /// that half-lands is a state neither side can reason about. Windows are exempt by
    /// design; see [`ViewerKind::isolate`].
    ForeignObject {
        /// The object whose owner is wrong for this view.
        object: ObjectId,
        /// The isolate that owns the object.
        object_owner: IsolateId,
        /// The view that would have seen it.
        viewer: ViewerId,
        /// The isolate that view belongs to.
        viewer_owner: IsolateId,
    },
    /// ★★★ **The content of this region came through a transport we cannot observe**, so
    /// no view of it can be vouched for. Correction 3, made a refusal rather than a
    /// silently stale view.
    UnwitnessedContent {
        /// The affected run.
        region: GpgaRegion,
        /// How it got that way.
        transport: UnwitnessedTransport,
    },
    /// ★★ **The plan was built against an older index** (R5: re-validate after re-lock).
    ///
    /// Deliberately coarse — *any* intervening change invalidates *any* outstanding plan.
    /// A finer check would have to decide which changes commute, and getting that wrong
    /// is a half-applied fan-out. It is a **retryable** refusal: re-plan and re-apply, the
    /// shape `SharedDevice::verb_op` already retries with a bounded counter.
    PlanStale {
        /// The generation the plan was built against.
        planned_at: u64,
        /// The generation the index is at now.
        now: u64,
    },
    /// A slice's coordinates were malformed. Structurally unreachable from a well-formed
    /// region, and refused rather than unwrapped, because the arithmetic is on
    /// guest-influenced numbers.
    BadSlice(HostSliceError),
}

impl From<RegionError> for ViewFault {
    fn from(e: RegionError) -> Self {
        Self::BadRegion(e)
    }
}

impl From<HostSliceError> for ViewFault {
    fn from(e: HostSliceError) -> Self {
        Self::BadSlice(e)
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────
// Changes and plans — the three-pass shape
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★ **What happened to GPGA**, as a plain owned value carrying **IDs only**.
///
/// This is what an act phase (rank 1, holding the issuing proc's lock) may produce. It
/// holds no reference into the index and no reference into a `Proc`, which is exactly what
/// lets it cross a lock boundary to the applier — the same discipline
/// `kayfabe_fwd::PtWrite` follows for a latched page-table write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectChange {
    /// A new object occupies `region`.
    Allocated {
        /// Where.
        region: GpgaRegion,
        /// Whose handle namespace it lives in.
        owner: IsolateId,
    },
    /// The object is freed. ★ Every viewer loses it, and the coverage goes with it — see
    /// [`ViewerIndex::apply`].
    Freed {
        /// Which object.
        object: ObjectId,
    },
    /// The object moves to `to`, vacating wherever it was.
    Remapped {
        /// Which object.
        object: ObjectId,
        /// Its new region.
        to: GpgaRegion,
    },
    /// ★★ Content in `region` was written by a transport the port cannot observe.
    UnwitnessedWrite {
        /// The affected region.
        region: GpgaRegion,
        /// Which transport.
        transport: UnwitnessedTransport,
    },
}

/// The fan-out of one [`ObjectChange`], computed but **not yet committed**.
///
/// Built by [`ViewerIndex::plan`] (a pure `&self` read that contacts no viewer) and
/// consumed by [`ViewerIndex::apply`]. It records the index generation it was built
/// against so that `apply` can refuse a plan the world moved out from under (R5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewUpdatePlan {
    change: ObjectChange,
    planned_at: u64,
    /// Per viewer, in ascending [`ViewerId`] order for determinism.
    deliveries: Vec<(ViewerId, Vec<ViewUpdate>)>,
}

impl ViewUpdatePlan {
    /// The change this plan is for.
    #[must_use]
    pub const fn change(&self) -> ObjectChange {
        self.change
    }

    /// The index generation it was planned against.
    #[must_use]
    pub const fn planned_at(&self) -> u64 {
        self.planned_at
    }

    /// How many viewers this change reaches. ★ The fan-out width, exposed so a test can
    /// assert it is the number of *viewers* and not the number of *pages*.
    #[must_use]
    pub fn fan_out(&self) -> usize {
        self.deliveries.len()
    }

    /// The updates planned for one viewer, in order.
    #[must_use]
    pub fn deliveries_for(&self, viewer: ViewerId) -> &[ViewUpdate] {
        self.deliveries
            .iter()
            .find(|(v, _)| *v == viewer)
            .map_or(&[], |(_, u)| u.as_slice())
    }
}

/// What [`ViewerIndex::apply`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Applied {
    /// The object the change created, if it created one.
    pub object: Option<ObjectId>,
    /// How many viewers were enqueued to.
    pub viewers_updated: usize,
    /// ★ How many viewers went [`ViewState::Desynced`] because their queue was full.
    /// Counted, never silent.
    pub viewers_desynced: usize,
}

// ─────────────────────────────────────────────────────────────────────────────────────
// The index
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★★ **The GPGA viewer index.**
///
/// Objects are the authority, GPGA is the key, and every question is asked per region. See
/// the module docs for the design, the three corrections and the lock resolution.
///
/// Concurrency (decision #17): plain owned data, no interior mutability. Queries are
/// `&self`; `plan` is `&self`; `apply`/`drain` are `&mut self`. It is device-global state
/// — rank 0 — because a GPGA object is visible to viewers belonging to other procs.
#[derive(Debug, Default)]
pub struct ViewerIndex {
    /// aperture -> (GPGA range -> the object there). Range-keyed (correction 2).
    objects: BTreeMap<Aperture, IntervalMap<ObjectEntry>>,
    /// aperture -> (GPGA range -> how its content got there). Only ever holds
    /// [`Witness::Unwitnessed`] runs; a range absent from here is witnessed.
    unwitnessed: BTreeMap<Aperture, IntervalMap<UnwitnessedTransport>>,
    viewers: BTreeMap<ViewerId, Viewer>,
    next_viewer: u32,
    next_object: u64,
    /// Bumped by every mutation. R5's staleness token for [`ViewUpdatePlan`].
    generation: u64,
}

impl ViewerIndex {
    /// An empty index: no objects, no viewers.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current generation. Every mutation bumps it; a [`ViewUpdatePlan`] is only valid
    /// at the generation it was planned against.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// How many objects the index holds, across all apertures.
    #[must_use]
    pub fn object_count(&self) -> usize {
        self.objects.values().map(IntervalMap::len).sum()
    }

    // ── Viewers ──────────────────────────────────────────────────────────────────────

    /// Register a view. It starts with **no coverage**: a viewer that has been registered
    /// but has mapped nothing sees nothing, which is the honest starting state.
    pub fn add_view(&mut self, kind: ViewerKind) -> ViewerId {
        let id = ViewerId(self.next_viewer);
        self.next_viewer += 1;
        self.viewers.insert(
            id,
            Viewer {
                kind,
                cover: BTreeMap::new(),
                pending: VecDeque::new(),
                state: ViewState::Live,
            },
        );
        self.generation += 1;
        id
    }

    /// ★★★ **Map `region` into `viewer` at `view_off` — and hand back everything it
    /// already contains.**
    ///
    /// This is *test 2's direction*, the most-forgotten one: a **new** view must be
    /// seeded with the objects that are already there, not merely subscribed to future
    /// changes. Making the seed the return value (and queueing it too) means a caller
    /// cannot accidentally get the subscription without the seed — there is no ordering
    /// in which a fresh view is silently empty.
    ///
    /// The returned updates are also enqueued, so a viewer that drains rather than reads
    /// the return value gets the same picture.
    ///
    /// # Errors
    /// - [`ViewFault::UnknownViewer`].
    /// - [`ViewFault::SelfAlias`] — this viewer already covers part of `region`.
    /// - [`ViewFault::ForeignObject`] — an isolate view would have seen another isolate's
    ///   object (the dual).
    /// - [`ViewFault::UnwitnessedContent`] — part of `region`'s content is unvouchable.
    pub fn map_into_view(
        &mut self,
        viewer: ViewerId,
        region: GpgaRegion,
        view_off: u64,
    ) -> Result<Vec<ViewUpdate>, ViewFault> {
        let kind = self.viewer_kind(viewer)?;

        // (1) SELF-ALIAS — refused before anything is inserted.
        {
            let v = self
                .viewers
                .get(&viewer)
                .ok_or(ViewFault::UnknownViewer(viewer))?;
            if let Some(cov) = v.cover.get(&region.aperture)
                && let Some((s, l, _)) = cov
                    .spans(region.base, region.len)
                    .into_iter()
                    .find_map(|(s, l, m)| m.map(|_| (s, l, ())))
            {
                return Err(ViewFault::SelfAlias {
                    viewer,
                    region: GpgaRegion::new(region.aperture, s, l)?,
                });
            }
        }

        // (2) THE DUAL and (3) the witness — both refuse the whole mapping.
        let seed = self.contents(region);
        let seed = self.vouch(viewer, kind, &seed)?;

        // (4) Commit. Nothing below can fail.
        let updates: Vec<ViewUpdate> = seed
            .into_iter()
            .map(|s| ViewUpdate::Shows {
                view_off: view_off + (s.region.base - region.base),
                region: s.region,
                occupant: s.occupant,
            })
            .collect();
        {
            let v = self
                .viewers
                .get_mut(&viewer)
                .ok_or(ViewFault::UnknownViewer(viewer))?;
            // `insert` cannot fail here: the region is well-formed by construction and the
            // self-alias check above refused any overlap with this viewer's coverage.
            let _ = v.cover.entry(region.aperture).or_default().insert(
                region.base,
                region.len,
                view_off,
            );
            for u in &updates {
                push_bounded(v, *u);
            }
        }
        self.generation += 1;
        Ok(updates)
    }

    /// Remove `viewer` entirely. Its coverage and its undelivered updates go with it.
    ///
    /// # Errors
    /// [`ViewFault::UnknownViewer`].
    pub fn remove_view(&mut self, viewer: ViewerId) -> Result<(), ViewFault> {
        self.viewers
            .remove(&viewer)
            .ok_or(ViewFault::UnknownViewer(viewer))?;
        self.generation += 1;
        Ok(())
    }

    /// What kind of view this is.
    ///
    /// # Errors
    /// [`ViewFault::UnknownViewer`].
    pub fn viewer_kind(&self, viewer: ViewerId) -> Result<ViewerKind, ViewFault> {
        self.viewers
            .get(&viewer)
            .map(|v| v.kind)
            .ok_or(ViewFault::UnknownViewer(viewer))
    }

    /// Whether this view is still known-correct.
    ///
    /// # Errors
    /// [`ViewFault::UnknownViewer`].
    pub fn viewer_state(&self, viewer: ViewerId) -> Result<ViewState, ViewFault> {
        self.viewers
            .get(&viewer)
            .map(|v| v.state)
            .ok_or(ViewFault::UnknownViewer(viewer))
    }

    /// How many updates this view has not yet drained.
    ///
    /// # Errors
    /// [`ViewFault::UnknownViewer`].
    pub fn pending_len(&self, viewer: ViewerId) -> Result<usize, ViewFault> {
        self.viewers
            .get(&viewer)
            .map(|v| v.pending.len())
            .ok_or(ViewFault::UnknownViewer(viewer))
    }

    /// Every registered viewer, ascending.
    pub fn viewers(&self) -> impl Iterator<Item = (ViewerId, ViewerKind)> {
        self.viewers.iter().map(|(&id, v)| (id, v.kind))
    }

    /// ★ **DRAIN — one viewer, alone.** Takes everything queued for `viewer` and leaves it
    /// empty. This is the third pass; it is a **pull**, which is what makes a hanging
    /// viewer harmless to everyone else.
    ///
    /// A [`ViewState::Desynced`] viewer still drains what it has — the state says *"there
    /// were more"*, and hiding the ones that survived would help nobody.
    ///
    /// # Errors
    /// [`ViewFault::UnknownViewer`].
    pub fn drain(&mut self, viewer: ViewerId) -> Result<Vec<ViewUpdate>, ViewFault> {
        let v = self
            .viewers
            .get_mut(&viewer)
            .ok_or(ViewFault::UnknownViewer(viewer))?;
        Ok(v.pending.drain(..).collect())
    }

    /// Clear a viewer's [`ViewState::Desynced`] mark. The caller is asserting it has
    /// rebuilt the view from [`ViewerIndex::view_contents`]; nothing here can check that,
    /// which is why it is an explicit call and not a side effect of draining.
    ///
    /// # Errors
    /// [`ViewFault::UnknownViewer`].
    pub fn resynced(&mut self, viewer: ViewerId) -> Result<(), ViewFault> {
        let v = self
            .viewers
            .get_mut(&viewer)
            .ok_or(ViewFault::UnknownViewer(viewer))?;
        v.state = ViewState::Live;
        Ok(())
    }

    // ── Queries: always per region, never per address ────────────────────────────────

    /// ★★★ **What does this region CONTAIN?** — a total, ascending, non-overlapping
    /// partition of `region` into maximal runs of constant answer.
    ///
    /// This and [`ViewerIndex::viewers_of`] are the only questions the index answers, and
    /// both take a region. There is deliberately no `owner_of(addr)`.
    ///
    /// An empty return is impossible for a well-formed region: the partition is total, so
    /// an unoccupied region comes back as one span with `occupant: None` — a null page,
    /// not a miss.
    #[must_use]
    pub fn contents(&self, region: GpgaRegion) -> Vec<GpgaSpan> {
        let objs = self.objects.get(&region.aperture);
        let unwit = self.unwitnessed.get(&region.aperture);

        // Two partitions of the same region, intersected into one. Both are total, so the
        // intersection is too — which is the property that stops a run being dropped.
        let mut cuts: Vec<u64> = vec![region.base, region.end()];
        if let Some(m) = objs {
            for (s, l, _) in m.spans(region.base, region.len) {
                cuts.push(s);
                cuts.push(s.saturating_add(l));
            }
        }
        if let Some(m) = unwit {
            for (s, l, _) in m.spans(region.base, region.len) {
                cuts.push(s);
                cuts.push(s.saturating_add(l));
            }
        }
        cuts.retain(|&c| c >= region.base && c <= region.end());
        cuts.sort_unstable();
        cuts.dedup();

        let mut out = Vec::with_capacity(cuts.len().saturating_sub(1));
        for w in cuts.windows(2) {
            let (start, end) = (w[0], w[1]);
            debug_assert!(end > start, "cuts are deduped and sorted");
            let Ok(run) = GpgaRegion::new(region.aperture, start, end - start) else {
                continue;
            };
            let occupant = objs
                .and_then(|m| m.lookup(start))
                .map(|(_, _, e)| e)
                .map(|e| Occupant {
                    object: e.id,
                    owner: e.owner,
                    extent: extent_of(e.region, run),
                });
            let witness = unwit
                .and_then(|m| m.lookup(start))
                .map_or(Witness::Witnessed, |(_, _, t)| Witness::Unwitnessed(*t));
            out.push(GpgaSpan {
                region: run,
                occupant,
                witness,
            });
        }
        out
    }

    /// ★★★ **WHO CAN SEE this region?** — every viewer covering any part of it, with the
    /// exact sub-range each one covers and where in that view it appears.
    ///
    /// ★ *"Including partial maps"* is the whole point of returning [`ViewerSlice`]s
    /// rather than ids: a window that covers the last 4 KiB of a 2 MiB object appears with
    /// that 4 KiB, not with the object.
    ///
    /// # Complexity — stated, because correction 2 is about exactly this
    ///
    /// `O(V · (log R + k))` where `V` is the number of registered viewers, `R` a viewer's
    /// mapped regions and `k` the regions actually overlapping. It is proportional to
    /// **regions**, never to pages: a 12 GiB framebuffer with one mapping costs the same
    /// as a 4 KiB one with one mapping.
    #[must_use]
    pub fn viewers_of(&self, region: GpgaRegion) -> Vec<ViewerSlice> {
        let mut out = Vec::new();
        for (&id, v) in &self.viewers {
            let Some(cov) = v.cover.get(&region.aperture) else {
                continue;
            };
            for (s, l, mapped) in cov.spans(region.base, region.len) {
                let Some(&view_base) = mapped else { continue };
                // The covering range may start before the query. Recover its own start so
                // the view offset is right for a partial overlap.
                let Some((c_start, _, _)) = cov.lookup(s) else {
                    continue;
                };
                let Ok(run) = GpgaRegion::new(region.aperture, s, l) else {
                    continue;
                };
                out.push(ViewerSlice {
                    viewer: id,
                    kind: v.kind,
                    region: run,
                    view_off: view_base + (s - c_start),
                });
            }
        }
        out
    }

    /// Everything one view currently shows, ascending — the rebuild source for a
    /// [`ViewState::Desynced`] viewer, and the authority a stale queue is not.
    ///
    /// # Errors
    /// - [`ViewFault::UnknownViewer`].
    /// - [`ViewFault::UnwitnessedContent`] — ★★ part of what this view shows was written
    ///   by a transport we cannot observe, so the view cannot be vouched for. **This is
    ///   correction 3's refusal**: the caller is told by name, under miss = fault, rather
    ///   than handed a picture that might be wrong.
    pub fn view_contents(&self, viewer: ViewerId) -> Result<Vec<ViewUpdate>, ViewFault> {
        let v = self
            .viewers
            .get(&viewer)
            .ok_or(ViewFault::UnknownViewer(viewer))?;
        let mut out = Vec::new();
        for (&aperture, cov) in &v.cover {
            for (s, l, &view_base) in cov.iter() {
                let run = GpgaRegion::new(aperture, s, l)?;
                for span in self.contents(run) {
                    if let Witness::Unwitnessed(transport) = span.witness {
                        return Err(ViewFault::UnwitnessedContent {
                            region: span.region,
                            transport,
                        });
                    }
                    let Some(occupant) = span.occupant else {
                        continue;
                    };
                    out.push(ViewUpdate::Shows {
                        view_off: view_base + (span.region.base - s),
                        region: span.region,
                        occupant,
                    });
                }
            }
        }
        Ok(out)
    }

    // ── The three-pass update ────────────────────────────────────────────────────────

    /// ★★★ **PLAN (pure, `&self`).** Compute the whole fan-out of `change` — which viewers,
    /// which slices — **without touching a single viewer** and without mutating anything.
    ///
    /// This is the pass that makes the lock constraint go away: it is the plan, not the
    /// index, that crosses from the issuing proc's rank-1 phase to the applier.
    ///
    /// # Errors
    /// - [`ViewFault::UnknownObject`] — `Freed`/`Remapped` named an object that is gone.
    /// - [`ViewFault::ObjectOverlap`] — an `Allocated`/`Remapped` collides with a live
    ///   object.
    /// - [`ViewFault::ForeignObject`] — the dual: an isolate view would see another
    ///   isolate's object.
    /// - [`ViewFault::BadRegion`] / [`ViewFault::BadSlice`].
    pub fn plan(&self, change: &ObjectChange) -> Result<ViewUpdatePlan, ViewFault> {
        let mut deliveries: BTreeMap<ViewerId, Vec<ViewUpdate>> = BTreeMap::new();
        match *change {
            ObjectChange::Allocated { region, owner } => {
                self.check_free(region)?;
                // The id the apply will mint. Recorded in the plan so the deliveries name
                // the same object the commit creates; `PlanStale` guarantees no other
                // apply took it in between.
                let id = ObjectId(self.next_object);
                self.fan_out_shows(&mut deliveries, region, id, owner, region)?;
            }
            ObjectChange::Freed { object } => {
                let e = self.object(object)?;
                self.fan_out_revokes(&mut deliveries, e.region, object);
            }
            ObjectChange::Remapped { object, to } => {
                let e = self.object(object)?;
                // Overlap is checked against everything EXCEPT the object itself, which is
                // about to vacate — a remap onto a range one already owns is legal.
                for span in self.contents(to) {
                    if let Some(o) = span.occupant
                        && o.object != object
                    {
                        return Err(ViewFault::ObjectOverlap { region: to });
                    }
                }
                self.fan_out_revokes(&mut deliveries, e.region, object);
                self.fan_out_shows(&mut deliveries, to, object, e.owner, to)?;
            }
            ObjectChange::UnwitnessedWrite { region, transport } => {
                for s in self.viewers_of(region) {
                    deliveries
                        .entry(s.viewer)
                        .or_default()
                        .push(ViewUpdate::Unwitnessed {
                            view_off: s.view_off,
                            region: s.region,
                            transport,
                        });
                }
            }
        }
        Ok(ViewUpdatePlan {
            change: *change,
            planned_at: self.generation,
            deliveries: deliveries.into_iter().collect(),
        })
    }

    /// ★★★ **APPLY (`&mut self`).** Re-validate the plan (R5) and commit it: mutate the
    /// object map, mutate every affected viewer's coverage, and **enqueue** each viewer's
    /// updates.
    ///
    /// ⊘ **No viewer is called into.** That is the hanging-isolate property: a viewer that
    /// never drains delays nobody. Its queue is bounded, and overflow marks it
    /// [`ViewState::Desynced`] and is **counted** in [`Applied::viewers_desynced`].
    ///
    /// ★★ The coverage mutation happens here, not at drain time, so a freed object stops
    /// being visible through [`ViewerIndex::view_contents`] immediately — a viewer cannot
    /// keep resolving it by refusing to read its queue. That is the difference between a
    /// stale view and a use-after-free.
    ///
    /// # Errors
    /// - [`ViewFault::PlanStale`] — the index moved since the plan was built. Retryable.
    /// - Everything [`ViewerIndex::plan`] can return: the checks are re-run here, because
    ///   a plan is a hint and the law has to hold at the site that depends on it.
    pub fn apply(&mut self, plan: &ViewUpdatePlan) -> Result<Applied, ViewFault> {
        if plan.planned_at != self.generation {
            return Err(ViewFault::PlanStale {
                planned_at: plan.planned_at,
                now: self.generation,
            });
        }
        let mut applied = Applied::default();

        // ── PASS 1: validate everything, mutate nothing. ─────────────────────────────
        match plan.change {
            ObjectChange::Allocated { region, .. } => self.check_free(region)?,
            ObjectChange::Freed { object } => {
                self.object(object)?;
            }
            ObjectChange::Remapped { object, to } => {
                self.object(object)?;
                for span in self.contents(to) {
                    if let Some(o) = span.occupant
                        && o.object != object
                    {
                        return Err(ViewFault::ObjectOverlap { region: to });
                    }
                }
            }
            ObjectChange::UnwitnessedWrite { .. } => {}
        }
        // The dual, re-checked against the plan's own deliveries.
        for (viewer, updates) in &plan.deliveries {
            let kind = self.viewer_kind(*viewer)?;
            for u in updates {
                if let ViewUpdate::Shows { occupant, .. } = u {
                    self.check_not_foreign(*viewer, kind, occupant)?;
                }
            }
        }

        // ── PASS 2: apply. Nothing below may fail. ───────────────────────────────────
        match plan.change {
            ObjectChange::Allocated { region, owner } => {
                let id = ObjectId(self.next_object);
                self.next_object += 1;
                let _ = self.objects.entry(region.aperture).or_default().insert(
                    region.base,
                    region.len,
                    ObjectEntry { id, owner, region },
                );
                applied.object = Some(id);
            }
            ObjectChange::Freed { object } => {
                self.retire(object);
            }
            ObjectChange::Remapped { object, to } => {
                let Some(e) = self.retire(object) else {
                    // Unreachable: pass 1 resolved it and nothing since could remove it.
                    return Err(ViewFault::UnknownObject(object));
                };
                let _ = self.objects.entry(to.aperture).or_default().insert(
                    to.base,
                    to.len,
                    ObjectEntry {
                        id: object,
                        owner: e.owner,
                        region: to,
                    },
                );
                applied.object = Some(object);
            }
            ObjectChange::UnwitnessedWrite { region, transport } => {
                self.mark_unwitnessed(region, transport);
            }
        }

        // Coverage is NOT removed on revoke: a window keeps covering the addresses it
        // covers whether or not an object is there. What goes away is the OBJECT, and
        // `view_contents` reads the object map, so the freed object stops being visible
        // the instant the map loses it — no queue involved.
        for (viewer, updates) in &plan.deliveries {
            let Some(v) = self.viewers.get_mut(viewer) else {
                continue;
            };
            let before = v.state;
            for u in updates {
                push_bounded(v, *u);
            }
            applied.viewers_updated += 1;
            if before == ViewState::Live && v.state == ViewState::Desynced {
                applied.viewers_desynced += 1;
            }
        }
        self.generation += 1;
        Ok(applied)
    }

    // ── Internals ────────────────────────────────────────────────────────────────────

    /// Resolve an object id to its entry. ★ This is a lookup by **identity**, not by
    /// address — it is not the reverse resolution the governing rule forbids.
    fn object(&self, id: ObjectId) -> Result<ObjectEntry, ViewFault> {
        self.objects
            .values()
            .flat_map(IntervalMap::iter)
            .find(|(_, _, e)| e.id == id)
            .map(|(_, _, e)| *e)
            .ok_or(ViewFault::UnknownObject(id))
    }

    /// Refuse if any part of `region` is already occupied.
    fn check_free(&self, region: GpgaRegion) -> Result<(), ViewFault> {
        if self.contents(region).iter().any(|s| s.occupant.is_some()) {
            return Err(ViewFault::ObjectOverlap { region });
        }
        Ok(())
    }

    fn retire(&mut self, id: ObjectId) -> Option<ObjectEntry> {
        for m in self.objects.values_mut() {
            let base = m
                .iter()
                .find(|(_, _, e)| e.id == id)
                .map(|(_, _, e)| e.region.base);
            if let Some(base) = base {
                return m.remove_at(base).map(|(_, e)| e);
            }
        }
        None
    }

    /// Record that `region`'s content came through an unobservable transport, coalescing
    /// with any overlapping record. Coalescing rather than inserting keeps the map's
    /// non-overlap invariant without ever losing a marked byte.
    fn mark_unwitnessed(&mut self, region: GpgaRegion, transport: UnwitnessedTransport) {
        let m = self.unwitnessed.entry(region.aperture).or_default();
        let mut base = region.base;
        let mut end = region.end();
        // Everything that touches [base, end) — including a range that merely abuts it —
        // is absorbed, so repeated marks do not fragment the map.
        loop {
            let hit = m
                .iter()
                .find(|&(s, l, _)| s <= end && s.saturating_add(l) >= base)
                .map(|(s, l, _)| (s, l));
            let Some((s, l)) = hit else { break };
            base = base.min(s);
            end = end.max(s.saturating_add(l));
            m.remove_at(s);
        }
        let _ = m.insert(base, end - base, transport);
    }

    /// The dual, as a predicate. Windows are exempt by design; see [`ViewerKind::isolate`].
    fn check_not_foreign(
        &self,
        viewer: ViewerId,
        kind: ViewerKind,
        occupant: &Occupant,
    ) -> Result<(), ViewFault> {
        let Some(viewer_owner) = kind.isolate() else {
            return Ok(());
        };
        if occupant.owner == viewer_owner {
            return Ok(());
        }
        Err(ViewFault::ForeignObject {
            object: occupant.object,
            object_owner: occupant.owner,
            viewer,
            viewer_owner,
        })
    }

    /// Check a would-be seed against the dual and the witness, both refusing whole.
    fn vouch(
        &self,
        viewer: ViewerId,
        kind: ViewerKind,
        spans: &[GpgaSpan],
    ) -> Result<Vec<GpgaSpanOccupied>, ViewFault> {
        let mut out = Vec::new();
        for s in spans {
            if let Witness::Unwitnessed(transport) = s.witness {
                return Err(ViewFault::UnwitnessedContent {
                    region: s.region,
                    transport,
                });
            }
            let Some(occupant) = s.occupant else { continue };
            self.check_not_foreign(viewer, kind, &occupant)?;
            out.push(GpgaSpanOccupied {
                region: s.region,
                occupant,
            });
        }
        Ok(out)
    }

    fn fan_out_shows(
        &self,
        deliveries: &mut BTreeMap<ViewerId, Vec<ViewUpdate>>,
        region: GpgaRegion,
        id: ObjectId,
        owner: IsolateId,
        object_region: GpgaRegion,
    ) -> Result<(), ViewFault> {
        for s in self.viewers_of(region) {
            let occupant = Occupant {
                object: id,
                owner,
                extent: extent_of(object_region, s.region),
            };
            self.check_not_foreign(s.viewer, s.kind, &occupant)?;
            deliveries
                .entry(s.viewer)
                .or_default()
                .push(ViewUpdate::Shows {
                    view_off: s.view_off,
                    region: s.region,
                    occupant,
                });
        }
        Ok(())
    }

    fn fan_out_revokes(
        &self,
        deliveries: &mut BTreeMap<ViewerId, Vec<ViewUpdate>>,
        region: GpgaRegion,
        object: ObjectId,
    ) {
        for s in self.viewers_of(region) {
            deliveries
                .entry(s.viewer)
                .or_default()
                .push(ViewUpdate::Revoked {
                    view_off: s.view_off,
                    region: s.region,
                    object,
                });
        }
    }
}

/// A [`GpgaSpan`] known to be occupied — the intermediate `vouch` hands back so the commit
/// path does not re-unwrap an `Option` it already checked.
struct GpgaSpanOccupied {
    region: GpgaRegion,
    occupant: Occupant,
}

/// ★★ **Which part of `object` the run `run` is — as an ownership regime.**
///
/// [`HostExtent::Whole`] exactly when the run IS the object; [`HostExtent::Slice`]
/// otherwise, carrying the offset into the object and the run's length. This is the reuse
/// `gpga_address_space.md` §8.2 asks for: the question *"does this view hold the whole
/// thing or a piece of it"* is the same question a reclaim site asks, so it gets the same
/// type rather than a parallel `offset: u64`.
///
/// A slice that cannot be constructed (structurally impossible from a well-formed run
/// inside a well-formed object) degrades to `Whole` rather than panicking — but the
/// callers that can surface it prefer [`ViewFault::BadSlice`].
fn extent_of(object: GpgaRegion, run: GpgaRegion) -> HostExtent {
    if run.base == object.base && run.len == object.len {
        return HostExtent::Whole;
    }
    HostSlice::new(run.base.wrapping_sub(object.base), run.len)
        .map_or(HostExtent::Whole, HostExtent::Slice)
}

/// Enqueue one update, or mark the viewer [`ViewState::Desynced`] if its queue is full.
///
/// ★ The update is **dropped** on overflow, and that is safe only because the state change
/// says so: the index remains authoritative and the viewer's contract becomes *"rebuild
/// from [`ViewerIndex::view_contents`]"*. Dropping without the mark would be the stale-view
/// bug this whole module exists to prevent.
fn push_bounded(v: &mut Viewer, u: ViewUpdate) {
    if v.pending.len() >= MAX_PENDING_UPDATES {
        v.state = ViewState::Desynced;
        return;
    }
    v.pending.push_back(u);
}

kayfabe_util::assert_send_sync!(
    ViewerIndex,
    GpgaAddr,
    GpgaRegion,
    GpgaSpan,
    Occupant,
    ObjectId,
    ViewerId,
    ViewerKind,
    ViewerSlice,
    ViewState,
    ViewUpdate,
    ViewUpdatePlan,
    ObjectChange,
    ViewFault,
    Witness,
    UnwitnessedTransport,
    Applied,
);
