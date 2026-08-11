//! # kayfabe-mmu — the address plane
//!
//! Owns THE ADDRESS TABLE (`mode2_address_table.md`, the one-table-of-truth
//! directive): **one authoritative, forward-populated VA→phys table per VAS, keyed by
//! PDB** — never by client handle (lesson L7: many clients share one VAS; a
//! GSP-managed `hVASpace=0` channel has no client-keyed VAS at all).
//!
//! The rules, as API shape (they are enforced *by construction*, testing strategy
//! `taddr_forward_only`):
//!
//! - **Forward-populate only.** [`AddressTable::bind`] is the only way in — called at
//!   bind time (RPC populate source) or at a CE-PT-write commit point (#13's populate
//!   source). There is no exec-time reverse-resolve entry point in this API.
//! - **MISS = FAULT, with NO deferring case at this layer.** [`AddressTable::resolve`]
//!   returns [`AddressFault::Miss`] on a miss. There is no fallback walk (a torn
//!   multi-level walk = wrong phys = cross-context leak — MISS=FAULT is a *security*
//!   property, arch doc §4.3.5).
//!
//!   ★ The core's miss taxonomy (`kayfabe_core` crate docs) allows one other answer —
//!   DEFER, when the fact is *not yet knowable* — and **no site in this crate qualifies**.
//!   That is worth stating rather than leaving to inference: this table IS the guest's
//!   TLB, and a TLB has no "later". A resolve happens because hardware is about to touch
//!   the address; an unbound VA is a fault on real silicon at that instant, and answering
//!   "wait, it may be declared soon" would be answering a question nobody asked. The
//!   deferring belongs one layer up, in DERIVATION (`Gpu::sync_rpc_mappings` defers a
//!   mapping whose PDB has not been declared) — and deferring there is precisely what
//!   lets this layer be absolute.
//! - **Unmap eager**, map lazy, reclaim deferred ([`AddressTable::unbind`]).
//!
//! ★ **corrected 2026-07-27** (was: *"Also here (skeleton this milestone): the GMMU **walk
//! algorithm**…"*, which overstated — found by the whitepaper's verification pass). What
//! [`walker`] actually contains today is **the seam and nothing behind it**: the [`walker::FbRead`]
//! page-table byte-source trait and the [`walker::WalkResult`] outcome enum. No walk loop, no
//! constructor, no implementor — 41 lines. The algorithm itself is still to be written, and
//! [`walker`]'s own module doc states the requirements it must meet: regime-independent core
//! logic against [`kayfabe_arch::GmmuFmt`], used only at forward-populate commit points
//! (decode-dirtied-PT-pages, #13), never as a resolve fallback.
//!
//! Concurrency (decision #17): plain owned data, no interior mutability;
//! [`AddressTable::resolve`]/[`AddressTable::iter`] are `&self` (concurrent reads
//! safe), bind/unbind are `&mut self` (caller-exclusive). `Send + Sync`
//! compile-time-asserted below; full contract in `kayfabe-core`'s crate docs.

pub mod gpga;
pub mod reach;
pub mod walker;

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuVa, Pdb};
use kayfabe_isolate::{HostHandle, IsolateId};
use kayfabe_util::IntervalMap;

/// Why a `[offset, offset+len)` byte range inside a host object was refused at
/// construction. Two variants because they are different mistakes and a caller that
/// reports "bad slice" for both has told nobody anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSliceError {
    /// `len == 0`. A binding backed by no bytes is not a binding; it is a handle with
    /// an opinion.
    Empty,
    /// `offset + len` wraps `u64`. Guest-influenced arithmetic never panics here
    /// (boundary-1 posture) — it refuses.
    Wraps,
}

/// ★★★ **A byte range inside a host memory object** — the arena sub-allocation unit
/// (`gpga_address_space.md` §8.2).
///
/// The fields are **private and there is exactly one constructor**, and that is the
/// whole point of the type: *"an offset with no length"* is not a state you can write
/// down. [`HostSlice::new`] additionally refuses the two ranges that are not ranges —
/// empty, and wrapping `u64` — so a `HostSlice` that exists names real bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HostSlice {
    offset: u64,
    len: u64,
}

impl HostSlice {
    /// Bytes `[offset, offset+len)` of a host object.
    ///
    /// # Errors
    /// - [`HostSliceError::Empty`] — `len == 0`.
    /// - [`HostSliceError::Wraps`] — `offset + len` does not fit in `u64`.
    pub const fn new(offset: u64, len: u64) -> Result<Self, HostSliceError> {
        if len == 0 {
            return Err(HostSliceError::Empty);
        }
        if offset.checked_add(len).is_none() {
            return Err(HostSliceError::Wraps);
        }
        Ok(HostSlice { offset, len })
    }

    /// Byte offset of the slice within its object.
    #[must_use]
    pub const fn offset(self) -> u64 {
        self.offset
    }

    /// Length of the slice in bytes. Never zero.
    ///
    /// ★ No `is_empty` on purpose (clippy's pairing lint is allowed off here): a
    /// `HostSlice` **cannot** be empty — [`HostSliceError::Empty`] refuses that at
    /// construction — so an `is_empty` could only ever answer `false`, and a predicate
    /// that has one possible answer invites callers to branch on a question that is
    /// already settled by the type.
    #[must_use]
    #[allow(clippy::len_without_is_empty)]
    pub const fn len(self) -> u64 {
        self.len
    }

    /// One past the last byte, within the object. Never wraps (refused at construction).
    #[must_use]
    pub const fn end(self) -> u64 {
        self.offset + self.len
    }
}

/// ★★★ **WHICH PART of the host object a binding is backed by — and, inseparably, WHO
/// FREES IT** (`gpga_address_space.md` §8.2/§9.3).
///
/// The two cases are not two spellings of a coordinate pair; they are two **ownership
/// regimes**, which is why this is an exhaustively-matched enum and not an `offset`
/// field:
///
/// - [`HostExtent::Whole`] — the object was allocated for this binding and nothing else
///   refers to it. Reclaiming the binding is what frees it. This is every publication
///   the code makes today.
/// - [`HostExtent::Slice`] — the object is a **reservation arena** that outlives this
///   binding and serves other bindings at other offsets. Reclaiming the binding must
///   **not** free it; the bytes are owed back to the arena, and the arena's own owner
///   frees the object.
///
/// ★ **Why an enum rather than `offset: u64, len: u64` on [`HostBacking`].** With bare
/// coordinates, `offset == 0` is ambiguous — the sole owner of a small object and the
/// first slice of a large arena are the same value — so every reclaim site would have to
/// *re-derive* whether it may free, from information it does not have. `Whole`/`Slice`
/// puts the answer in the type: adding the second case turns every existing free site
/// into a compile error until it says which regime it is in, which is exactly the
/// retrofit §8.2 says to buy now rather than later.
///
/// ★ **Why `Whole` carries no length.** The object *is* the bound range, so its length
/// is the range's length, already held by the table. Copying it here would be a second
/// source of truth that can drift. `Slice` must carry its own because the object is
/// **bigger than the binding** — the coordinates are not recoverable from the range —
/// and because a `HostBacking` travels standalone (trace events, refusal payloads) where
/// no interval length is attached to it. [`AddressTable::bind`] pins `Slice`'s length to
/// the range it backs ([`AddressFault::SliceLenMismatch`]), so the duplication cannot
/// drift silently either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostExtent {
    /// The entire object, owned by this binding alone.
    Whole,
    /// Bytes of an arena object that outlives this binding.
    Slice(HostSlice),
}

/// ★ **The host materialization of one binding — allocation, placement and EXTENT,
/// together** (`l1_concurrency.md` §12.16 gap G1; `gpga_address_space.md` §8.2).
///
/// A published range is backed by THREE host facts: an allocated host memory object
/// ([`HostBacking::memory`]), the GPU VA it is mapped at inside the owning `Vas`'s own
/// host VAS ([`HostBacking::host_va`]), and **which part of that object it is**
/// ([`HostBacking::extent`]). Reclaiming it needs all three — `unmap_gpu_va(vas,
/// host_va)`, then `free(memory)` **only if the binding owns the whole object** — so
/// they are ONE value, not three fields anyone can assemble a subset of.
///
/// **Why a struct rather than a second `Option` on [`Binding`].** With
/// `memory: Option<HostHandle>` beside `host_va: Option<u64>`, the state
/// *"mapped somewhere, owning nothing freeable"* is representable — and that state
/// was precisely the G1 defect: `commit_publish` stored the mapped VA and dropped the
/// `HostHandle` on the floor, so the majority of allocated host bytes existed in no
/// core state and no reclaim path could ever name them. Folding the trio into one
/// `Option<HostBacking>` makes **bound-but-unfreeable unrepresentable**: you cannot
/// write a host VA into a binding without also writing the handle that frees it.
/// (House pattern — `GpaSpace::release(arena)`-by-value: prefer the type over the
/// runtime check.)
///
/// ★★ **The fields are private and the constructors are [`HostBacking::whole`] and
/// [`HostBacking::slice`].** That is the same discipline one level down: a slice cannot
/// be assembled without its owning handle, and an extent cannot be attached to a
/// backing that has no handle to attach it to.
///
/// ★★ **Owner scope is INHERITED, not invented** (§9.3). RM grants *objects*, not
/// ranges: an isolate holding an arena handle can map **any** offset in it, so a slice
/// handed across an isolate boundary is a reach over that isolate's whole reservation.
/// The scope is already carried by [`HostHandle`]'s [`IsolateId`], and
/// `Worker::execute`'s foreign-handle gate already refuses across it — so
/// [`HostBacking::owner`] and [`HostBacking::belongs_to`] delegate to it rather than
/// adding a second, separately-maintained notion of who owns what.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostBacking {
    /// ★★★★★ **Is this object the range's ONLY memory, or a second one shadowing memory the
    /// guest already reaches?** — the owner's forbidden #2, as a field. See
    /// [`BackingBytes`]; it has no default and no inference.
    bytes: BackingBytes,
    /// The host object this range is backed by, in the OWNING isolate's handle
    /// namespace (`(Proc, GpuId)`-scoped — a handle from another isolate is a different
    /// object, boundary 2). For [`HostExtent::Slice`] this is the **arena**.
    memory: HostHandle,
    /// The host GPU VA it is mapped at, inside the owning `Vas`'s own host VAS.
    /// Per-Vas host placement is #14's proven fix — see `kayfabe-fwd`.
    host_va: u64,
    /// Which part of `memory`, and therefore who frees it.
    extent: HostExtent,
}

/// ★★★★★ **Is this host object the ONLY memory for the range, or a SECOND memory
/// shadowing one the guest already reaches?** — `ce_executor_tree.md`'s **forbidden #2**
/// (*"landing the data where the guest cannot see it"*), as a fact a classifier can read.
///
/// # ⊘ The measurement that forced this field into existence
///
/// `[measured 2026-08-11]` `kayfabe_fwd::representability_of` classified a range as
/// `Representability::HostBacked` — *"a real engine may be pointed at it"* — on the sole
/// test `binding.host.is_some()`. That asks **"does a host object exist here"**, and the
/// question that decides correctness is **"does the guest reach these bytes some other
/// way"**. The two production chains differ on exactly that, and nothing recorded it:
///
/// | chain | `Binding::phys` | the guest's other path to those bytes | this field |
/// |---|---|---|---|
/// | `VerbPlan::Publish` (host sysmem) | a GPA carved from **our own arena** | **none** — we invented the memory | [`BackingBytes::SoleBacking`] |
/// | `VerbPlan::PublishVidmem` (host vidmem) | the **guest's own framebuffer offset** | **BAR1/BAR2 into the device's `SparseFb`**, continuously | [`BackingBytes::ShadowsGuestMemory`] |
///
/// ⇒ `w228` measured the second row directly: `placed_as_asked=true` **and blank**. An
/// engine pointed at that object reads zeros where the guest wrote and writes where the
/// guest cannot look — `#12` in the C artifact, which cost weeks — and it is
/// **self-concealing**: a run over a blank object logs identically to a correct one.
///
/// ⊘ **And a third chain is invisible here entirely.** `VerbPlan::PinGuestRam`, the one
/// crossing that genuinely shares memory with the guest, records into `Vas::guest_ram_pins`
/// and never writes `Binding::host` at all — so the classifier has never seen it.
///
/// ★ It is a **constructor argument with no default**, deliberately: the fact is knowable
/// only at the instant the backing is created, by the chain that created it, and is
/// unrecoverable afterwards. A field that could be filled in later would be filled in by a
/// guess.
///
/// ⚠ **What this does NOT claim.** `SoleBacking` does not assert the guest can read the
/// object; it asserts there is no *competing* memory for the range. That is the property
/// the executor choice actually needs — `Publish`'s own doc scopes it the same way
/// (*"correct for a range the guest has never written"*) — and conflating it with
/// "guest-visible" is what made an earlier draft of this gate refuse `Publish` too, in
/// contradiction of a green test that was right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackingBytes {
    /// ★ **This object is the ONLY memory the range has.** Either we invented the bytes
    /// (`Publish`'s arena-carved GPA, which the guest has no independent path to) or they
    /// are the guest's own pages mapped through — the shape `PinGuestRam` would declare if
    /// its result ever became a `Binding`. Either way there is no second memory for a
    /// writer to diverge into, so a real engine pointed here produces an end-state that is
    /// **the** end-state.
    SoleBacking,
    /// ★★★ **A SECOND memory, at an address the guest already reaches another way.** The
    /// address is the guest's, the bytes are ours, and the guest's own accesses go on
    /// landing in the *other* one — the emulated framebuffer, or guest RAM. The two diverge
    /// from the first write and nothing reconciles them.
    ///
    /// ⊘ **Fatal for anything the guest reads or polls, which is what a ring is.**
    ///
    /// ★★★ **It has NO production producer any more, and that is the fix rather than a
    /// gap.** This variant is the *name of the state ruling 3 forbids*, and it exists so
    /// that [`Binding::real_gpu_memory`] has something to refuse: a caller honest enough to
    /// declare a shadow is refused by that declaration, and a caller silent about it is
    /// refused by the [`Aperture::Vidmem`] test beside it. `commit_back_fb_leaf` — the one
    /// chain that used to construct it — now raises
    /// `kayfabe_fwd::FwdFault::RegionKindRefused` and hands its host objects back as
    /// orphans instead of binding them.
    ShadowsGuestMemory,
}

impl HostBacking {
    /// A backing that owns its whole object: reclaiming this binding frees `memory`.
    ///
    /// `bytes` states whether this object is the range's only memory or a second one
    /// shadowing memory the guest already reaches — see [`BackingBytes`]. It is an argument
    /// rather than a field to be set later, because only this caller knows.
    #[must_use]
    pub const fn whole(memory: HostHandle, host_va: u64, bytes: BackingBytes) -> Self {
        HostBacking {
            bytes,
            memory,
            host_va,
            extent: HostExtent::Whole,
        }
    }

    /// A backing over `slice` of the arena object `arena`, which **outlives** this
    /// binding: reclaiming this binding must not free `arena`.
    #[must_use]
    pub const fn slice(
        arena: HostHandle,
        host_va: u64,
        slice: HostSlice,
        bytes: BackingBytes,
    ) -> Self {
        HostBacking {
            bytes,
            memory: arena,
            host_va,
            extent: HostExtent::Slice(slice),
        }
    }

    /// ★★★★★ Whether this object is the range's only memory — the forbidden-#2 predicate.
    #[must_use]
    pub const fn bytes(self) -> BackingBytes {
        self.bytes
    }

    /// The host object — the whole object, or the arena a slice was cut from.
    #[must_use]
    pub const fn memory(self) -> HostHandle {
        self.memory
    }

    /// The host GPU VA this range is mapped at.
    #[must_use]
    pub const fn host_va(self) -> u64 {
        self.host_va
    }

    /// Which part of [`HostBacking::memory`] this binding covers.
    #[must_use]
    pub const fn extent(self) -> HostExtent {
        self.extent
    }

    /// The slice, if this backing is one. `None` means it owns the whole object.
    #[must_use]
    pub const fn as_slice(self) -> Option<HostSlice> {
        match self.extent {
            HostExtent::Whole => None,
            HostExtent::Slice(s) => Some(s),
        }
    }

    /// ★★ **Does reclaiming this binding free [`HostBacking::memory`]?**
    ///
    /// The single question every reclaim site must ask, asked once, here. `true` only
    /// for [`HostExtent::Whole`]: a slice's object is the arena, which serves other
    /// bindings at other offsets and is freed by its own owner — freeing it on the
    /// first slice's release would pull the backing out from under every sibling slice,
    /// which is the `ALREADY-MAPPED`/use-after-free class one level up.
    #[must_use]
    pub const fn frees_object(self) -> bool {
        matches!(self.extent, HostExtent::Whole)
    }

    /// The isolate whose handle namespace this backing lives in — [`HostHandle`]'s
    /// [`IsolateId`], not a parallel notion (§9.3).
    #[must_use]
    pub const fn owner(self) -> IsolateId {
        self.memory.isolate()
    }

    /// Is this backing usable on `isolate`'s connection? Delegates to
    /// [`HostHandle::belongs_to`], so a slice is scoped exactly as tightly as the arena
    /// object it names and no more loosely.
    #[must_use]
    pub const fn belongs_to(self, isolate: IsolateId) -> bool {
        self.memory.belongs_to(isolate)
    }
}

/// ★★★★★ **WHICH OF THE FOUR KINDS A GPGA REGION IS — decided where the mapping is BOUND,
/// never derived at a consumer** (owner ruling, 2026-08-11).
///
/// > *"A GPGA region is exactly ONE of four kinds: unallocated / fake framebuffer / real GPU
/// > memory / DMA-to-guest-physical."*
///
/// # ⊘ The defect this replaces
///
/// `[measured 2026-08-11, `docs/design/gpga_region_kind.md` §1.1]` nothing in the tree
/// carried a kind. `kayfabe_fwd::Representability` was recomputed per operand by a four-arm
/// match with **two unguarded arms pointing in opposite directions**:
///
/// | reached because | answered | routed to |
/// |---|---|---|
/// | `Binding::host == None` — i.e. **nobody decided** | `Fabricated` | our CPU executor |
/// | **no row at all** | `Untracked` | ⚠ **the real host GPU** |
///
/// ⇒ *"the guest's GR ring is fake framebuffer"* was never a decision anyone took; it was
/// what a range **fell through to**. Nothing distinguished *"we determined this is emulated
/// framebuffer"* from *"nothing has been said about this range"*.
///
/// # Kind 1 is the ABSENCE of a row, and that is why it is not a variant here
///
/// ⊘ A row reading *"unallocated"* would be a second spelling of *"no row"*, and the two
/// would drift the first time one of them was reachable and the other was not.
/// [`AddressTable::kind_at`] answers `None` for kind 1, and that `None` is the same `None`
/// [`AddressTable::binding_at`] gives — one fact, one representation.
///
/// ⚠ **Kind 1 is not neutral.** An absent row is what `Representability::Untracked` reads,
/// and `Untracked` routes to the **host GPU**. So *"we never decided"* is not a safe default
/// at either end: it is fiction at one and hardware at the other.
///
/// # ★ What decides, given that we are not present at allocation
///
/// ⊘ The owner's model says the kind is *"decided at allocation/bind"*. `[measured
/// 2026-08-11, `gpga_region_kind.md` §0.1]` **the allocation half has no transport in Mode 2**
/// — the guest's stock RM allocates video memory out of its own heap over the framebuffer we
/// advertise, and `NV01_MEMORY_SYSTEM` (0x003e) / `NV01_MEMORY_LOCAL_USER` (0x0040) reach us
/// **zero** times across every committed boot while a real-hardware interposer shows 24 per
/// CUDA run. So **bind is the only event we have**, and the decision has exactly two shapes,
/// which is exactly the two constructors of [`Binding`]:
///
/// - [`Binding::declared_by_guest`] — the guest's own page table (or its own RPC-declared
///   mapping) named an **aperture**, and for an unpublished region that aperture IS the
///   declaration: `Vidmem` is the framebuffer we fabricate, sysmem is the guest's own
///   physical pages. ⊘ `Peer` is neither, and is refused by name rather than fabricated.
/// - [`Binding::real_gpu_memory`] — *we* allocated a host object and mapped it at the guest's
///   own VA. Kind 3 is the only kind that can carry a [`HostBacking`], and it cannot exist
///   without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RegionKind {
    /// **Kind 2 — the fake framebuffer.** The bytes live in the device's emulated
    /// framebuffer (`kayfabe_device::SparseFb`, a map on the VMM's heap). No host object
    /// exists and none may: see [`RegionKindFault::FakeFbAtRealGpuVa`].
    ///
    /// ⊘ Ruling 2 scopes what this kind is *for*: **guest-KERNEL channels we emulate**,
    /// where we manage the pushbuffer / USERD / ring / semaphore. A guest **userspace**
    /// mapping landing here is the execution blocker, not the design.
    FakeFramebuffer,
    /// **Kind 3 — real GPU memory.** A host memory object, mapped into the owning `Vas`'s
    /// own host VAS at the identical address, and it is the range's ONLY memory. A real
    /// engine may be pointed at it.
    RealGpuMemory,
    /// **Kind 4 — DMA to guest-physical.** The guest's own physical pages. The number in
    /// [`Binding::phys`] is a GPA and [`Binding::is_guest_ram`] is true.
    GuestPhysDma,
}

impl RegionKind {
    /// ★ Can a region of this kind carry a [`HostBacking`] — i.e. be mapped to a real GPU
    /// VA of an isolate?
    ///
    /// ⊘ **This is ruling 3 as a total function**, and it is consulted by the one
    /// constructor that can attach a backing, so it is not advice: *"no fake FB ever can be
    /// mapped to a real GPU VA of an isolate except the scratchpad"* (owner, 2026-08-11).
    /// The scratchpad carve-out is ruling 4 — `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` — which
    /// mints a **sysmem** object over host pages and therefore never asks this question
    /// about a `Vidmem` region.
    #[must_use]
    pub const fn may_be_host_mapped(self) -> bool {
        match self {
            RegionKind::FakeFramebuffer => false,
            RegionKind::RealGpuMemory | RegionKind::GuestPhysDma => true,
        }
    }
}

/// Why a [`Binding`] could not be constructed with the kind its caller asked for. Every
/// variant is a **decision that could not be taken truthfully**, never a malformed input —
/// [`AddressFault`] owns those.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKindFault {
    /// ★★★ **Owner ruling, 2026-08-11: fake framebuffer at a real GPU VA of an isolate.**
    ///
    /// A [`HostBacking`] was offered for a region that is kind 2 — either because its
    /// aperture is [`Aperture::Vidmem`] (there is no *other* video memory in this design;
    /// `no_real_phys_only_gpga_or_gpa`), or because the backing itself declares
    /// [`BackingBytes::ShadowsGuestMemory`], i.e. a SECOND memory at an address the guest
    /// goes on reading somewhere else.
    ///
    /// ⊘ **Both tests are here rather than one**, because they fail independently: the
    /// aperture catches a caller that is honest about the address and silent about the
    /// shadow, and `BackingBytes` catches a caller that is honest about the shadow over an
    /// aperture that looks innocent. `[measured 2026-08-11, `w228`]` the `PublishVidmem`
    /// chain is both at once — `placed_as_asked=true` **and blank**.
    FakeFbAtRealGpuVa {
        /// The aperture the caller named.
        aperture: Aperture,
    },
    /// [`Aperture::Peer`] — a second physical GPU's framebuffer, which this device does not
    /// back and no kind describes.
    ///
    /// ⊘ Refused rather than fabricated. Fabricating it is what the old fall-through did:
    /// a `Peer` binding with no host object became `Representability::Fabricated` and was
    /// handed to a CPU executor that then had to ask `DeclaredResidency` for a plane it
    /// answers `None` for. The refusal now happens at the decision, not two layers later.
    PeerHasNoKind,
}

/// Where a bound VA range points, in core terms — **and which of the four kinds it is**.
///
/// ★★ **The fields are private and there are exactly two constructors**, and that is the
/// point of the type: see [`RegionKind`]. A `Binding` that exists has had its kind decided
/// by the site that bound it, and the states ruling 3 forbids cannot be written down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    /// ★★★★★ Which of the four kinds this region is. Decided at construction; no default.
    kind: RegionKind,
    /// Physical/backing address (interpretation depends on `aperture`; for sysmem
    /// this is a guest-physical address).
    phys: u64,
    /// Aperture of the backing.
    aperture: Aperture,
    /// The host materialization, once the fwd plane has published this range
    /// (`None` = declared by the RPC/CE-capture source only — nothing host-side
    /// exists yet, and nothing host-side needs reclaiming). See [`HostBacking`] for
    /// why this is one `Option` over a pair and not a pair of `Option`s.
    ///
    /// ⊘ `Some` **only** for [`RegionKind::RealGpuMemory`] — [`RegionKind::may_be_host_mapped`]
    /// is checked by the one constructor that can set it.
    host: Option<HostBacking>,
}

impl Binding {
    /// ★★★ **The guest declared this range and nothing host-side exists behind it** — kinds
    /// 2 and 4, chosen by the aperture the guest's own page table (or its own RPC-declared
    /// mapping) named.
    ///
    /// ⊘ **This is a decision, not the old fall-through**, and the difference is worth
    /// stating because they produce the same answer for the two common apertures. The
    /// fall-through was *"a binding exists and it has no host object, therefore fiction"* —
    /// asked at **classify** time, over an address, by a consumer with no idea who bound it,
    /// and it swallowed [`Aperture::Peer`] silently. This is asked at **bind** time, of the
    /// only authority that exists in Mode 2 (§0.1: we are not present at allocation), and it
    /// **refuses** the aperture no kind describes.
    ///
    /// # Errors
    /// [`RegionKindFault::PeerHasNoKind`] — `aperture` is [`Aperture::Peer`].
    pub const fn declared_by_guest(phys: u64, aperture: Aperture) -> Result<Self, RegionKindFault> {
        let kind = match aperture {
            // The framebuffer this device advertises. `SparseFb` fabricates a zero page for
            // every address below `ChipProfile::fb_length`, so this is kind 2 by
            // construction — there is no "unallocated but vidmem" state to be in
            // (`gpga_region_kind.md` §2, established).
            Aperture::Vidmem => RegionKind::FakeFramebuffer,
            // `phys` is a guest-physical address; the guest's own pages.
            Aperture::SysmemCoherent | Aperture::SysmemNonCoherent => RegionKind::GuestPhysDma,
            Aperture::Peer => return Err(RegionKindFault::PeerHasNoKind),
        };
        Ok(Binding {
            kind,
            phys,
            aperture,
            host: None,
        })
    }

    /// ★★★ **We allocated a host memory object and mapped it at the guest's own VA** — kind
    /// 3, and the only kind that carries a [`HostBacking`].
    ///
    /// ⊘ The backing is a parameter and not a later assignment: kind 3 **without** an object
    /// is precisely the state the old fall-through called fiction, so a `RealGpuMemory`
    /// binding with `host: None` must not be writable at all.
    ///
    /// # Errors
    /// [`RegionKindFault::FakeFbAtRealGpuVa`] — ruling 3. `aperture` is [`Aperture::Vidmem`]
    /// (the emulated framebuffer is the only video memory in this design), or `host` declares
    /// [`BackingBytes::ShadowsGuestMemory`].
    ///
    /// [`RegionKindFault::PeerHasNoKind`] — `aperture` is [`Aperture::Peer`].
    pub const fn real_gpu_memory(
        phys: u64,
        aperture: Aperture,
        host: HostBacking,
    ) -> Result<Self, RegionKindFault> {
        // ⊘ **ONE derivation of the aperture's kind, not a second one here.** The aperture
        // is put through the same [`Binding::declared_by_guest`] the guest-declared path
        // uses, so `Peer` is refused by exactly the same rule and a `Vidmem` aperture is
        // recognised as kind 2 by exactly the same rule. A private `match aperture` on this
        // line would be a second reading that can drift from the first.
        let declared = match Binding::declared_by_guest(phys, aperture) {
            Ok(d) => d,
            Err(e) => return Err(e),
        };
        // ★★★ RULING 3, and both spellings of it. The aperture test catches a caller honest
        // about the address and silent about the shadow (`Vidmem` IS the framebuffer we
        // fabricate, so a host object at it is a second memory by definition); the
        // `BackingBytes` test catches a caller honest about the shadow over an aperture that
        // looks innocent. They fail independently.
        if !declared.kind.may_be_host_mapped()
            || matches!(host.bytes(), BackingBytes::ShadowsGuestMemory)
        {
            return Err(RegionKindFault::FakeFbAtRealGpuVa { aperture });
        }
        Ok(Binding {
            kind: RegionKind::RealGpuMemory,
            phys,
            aperture,
            host: Some(host),
        })
    }

    /// ★★★★★ Which of the four kinds this region is — **read, never derived**.
    #[must_use]
    pub const fn kind(self) -> RegionKind {
        self.kind
    }

    /// Physical/backing address; interpretation depends on [`Binding::aperture`]. For
    /// sysmem ([`RegionKind::GuestPhysDma`]) this is a guest-physical address — see
    /// [`Binding::is_guest_ram`].
    #[must_use]
    pub const fn phys(self) -> u64 {
        self.phys
    }

    /// Aperture of the backing, as the guest declared it.
    #[must_use]
    pub const fn aperture(self) -> Aperture {
        self.aperture
    }

    /// The host materialization, if this range is published at all. `None` for every
    /// [`RegionKind::FakeFramebuffer`] region, always.
    #[must_use]
    pub const fn host(self) -> Option<HostBacking> {
        self.host
    }

    /// The host GPU VA this range is published at, if it is published at all.
    /// (Convenience over [`Binding::host`]; the #14 gate's predicate.)
    #[must_use]
    pub fn host_va(&self) -> Option<u64> {
        self.host.map(HostBacking::host_va)
    }

    /// The host memory object backing this range, if it is published at all — the
    /// handle a reclaim path must `free`. Its existence is G1's whole point.
    #[must_use]
    pub fn host_memory(&self) -> Option<HostHandle> {
        self.host.map(HostBacking::memory)
    }

    /// ★★★ **Is [`Binding::phys`] a GUEST-PHYSICAL address?** — i.e. may a consumer hand it
    /// to `Vmm::gpa_read`, or to a hypervisor layout that turns a GPA into a file offset?
    ///
    /// ⊘ **Only for sysmem.** For [`Aperture::Vidmem`] the number is an offset into this
    /// device's own framebuffer and for [`Aperture::Peer`] it is another device's; both share
    /// the number space with guest RAM and neither is reachable through the `Vmm`. That
    /// collision is not hypothetical — `[measured 2026-08-11, boot `w232c_6fcedac`]` the
    /// user proc's GPFIFO rings resolve `V:0x1024000` while its pushbuffers resolve
    /// `S:0x41335000`, in **one** address space, on **one** channel.
    ///
    /// ★ It is a method rather than a `match` at each consumer because the consumers live in
    /// crates that may not name [`kayfabe_arch::Aperture`] at all
    /// (`kayfabe-qemu-raw`'s manifest: *"the shim names no architecture"*), and a predicate
    /// they cannot express is a predicate they will skip.
    #[must_use]
    pub fn is_guest_ram(&self) -> bool {
        matches!(
            self.aperture,
            Aperture::SysmemCoherent | Aperture::SysmemNonCoherent
        )
    }
}

/// One run of [`AddressTable::spans`]' partition: `(va, len, answer)`, where the answer is
/// `None` for a hole and `Some((binding, offset-of-this-run-within-it))` for a covered run.
///
/// ⊘ The offset is half the value: a run generally starts *inside* a binding, so
/// `binding.phys` on its own is the address of a different byte. Returning it is what stops
/// a consumer resolving the run's own address from the binding's base — or, as
/// `execution_plane_increments.md` §14.14 records, not resolving it at all.
pub type BindingRun = (u64, u64, Option<(Binding, u64)>);

/// A resolution failure. Every variant is LOUD: callers propagate, they never guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressFault {
    /// No binding covers the VA — the guest's own TLB would have faulted too.
    Miss {
        /// The PDB whose table missed.
        pdb: Pdb,
        /// The faulting VA.
        va: GpuVa,
    },
    /// A bind attempted to overlap a live binding without an eager unbind first
    /// (the `ALREADY-MAPPED` collision class made loud).
    Overlap {
        /// The PDB whose table refused the bind.
        pdb: Pdb,
        /// Start of the attempted range.
        va: GpuVa,
    },
    /// A bind named a malformed range — zero length, or `va + len` wraps `u64`. The
    /// `va`/`len` are guest-controlled (a hostile `MapMemoryDma` or a hostile CE
    /// PT-write dst), so this is a clean loud fault, never an arithmetic panic
    /// (boundary-1 posture; regression-banked).
    Malformed {
        /// The PDB whose table refused the bind.
        pdb: Pdb,
        /// Start of the attempted (malformed) range.
        va: GpuVa,
    },
    /// ★★★ **A host-backed binding whose host GPU VA is not the VA it is bound at**
    /// (`#102`, `eight_blockers_resolved.md` §1).
    ///
    /// The address plane's one identity law: *every binding with a host backing satisfies
    /// `host_va == the VA it is bound at`*. It has to hold because the guest's own
    /// commands are what dereference these addresses — a forwarded pushbuffer names the
    /// guest VA and the host MMU resolves it in the host VAS. A binding that records
    /// "published, but over there" is a mapping the guest can never reach, and it fails
    /// **later and elsewhere**, as `Xid 31 FAULT_PDE` inside a copy engine.
    ///
    /// Refused at [`AddressTable::bind`] — the table's only entrance — so the state is
    /// not merely detectable, it never enters the table.
    HostVaMismatch {
        /// The PDB whose table refused the bind.
        pdb: Pdb,
        /// The VA the range is being bound at (what the host VA must equal).
        va: GpuVa,
        /// The host VA the binding actually carried.
        host_va: u64,
    },
    /// ★★ **A [`HostExtent::Slice`] whose length is not the length of the range it
    /// backs** (`gpga_address_space.md` §8.2).
    ///
    /// The slice's `len` is deliberately redundant with the bound range's `len` — it has
    /// to be, because a [`HostBacking`] travels standalone. Redundancy that is never
    /// checked is just drift waiting to happen, and the drift is not benign: the slice
    /// coordinates are what an arena's free path will use to work out which bytes came
    /// back, so a slice claiming fewer bytes than it holds silently strands the
    /// remainder, and one claiming more hands the next requester bytes that are still
    /// mapped. Refused at [`AddressTable::bind`] — the table's only entrance.
    ///
    /// Near neighbour of [`AddressFault::Malformed`] and deliberately distinct: that one
    /// means the **guest's range** is nonsense, this one means **our own bookkeeping**
    /// disagrees with itself.
    SliceLenMismatch {
        /// The PDB whose table refused the bind.
        pdb: Pdb,
        /// The VA the range is being bound at.
        va: GpuVa,
        /// The length of the range being bound.
        len: u64,
        /// The length the slice claimed.
        slice_len: u64,
    },
}

/// The forward-populated VA→backing table of ONE VAS (identified by its PDB).
///
/// This *is* the guest's GMMU TLB from the emulator's point of view: populated at
/// the guest's own publication points, invalidated by the guest's own invalidate
/// discipline, faulting where real hardware would fault.
#[derive(Debug, Default)]
pub struct AddressTable {
    map: IntervalMap<Binding>,
}

impl AddressTable {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Forward-populate `[va, va+len)` → `binding` (bind-time RPC or CE-PT-write
    /// commit point). Overlap with a live binding is a loud fault: the guest must
    /// have unbound first (unmap eager); silently replacing would hide the #14
    /// collision class.
    pub fn bind(
        &mut self,
        pdb: Pdb,
        va: GpuVa,
        len: u64,
        binding: Binding,
    ) -> Result<(), AddressFault> {
        // ★★★ #102 — ADDRESS IDENTITY, at the table's only entrance. Checked before the
        // range is inserted, so a binding that claims a host publication somewhere other
        // than its own VA is not merely reportable — it is never in the table for a
        // resolve to hand out, and never in the ring gate for a doorbell to pass on.
        if let Some(h) = binding.host
            && h.host_va() != va.0
        {
            return Err(AddressFault::HostVaMismatch {
                pdb,
                va,
                host_va: h.host_va(),
            });
        }
        // ★★ §8.2 — EXTENT AGREEMENT. A slice's own length must be the length of the
        // range it backs; see [`AddressFault::SliceLenMismatch`] for why the redundancy
        // is checked rather than trusted. `Whole` has nothing to disagree with.
        if let Some(s) = binding.host.and_then(HostBacking::as_slice)
            && s.len() != len
        {
            return Err(AddressFault::SliceLenMismatch {
                pdb,
                va,
                len,
                slice_len: s.len(),
            });
        }
        self.map.insert(va.0, len, binding).map_err(|e| match e {
            kayfabe_util::IntervalError::Overlap { .. } => AddressFault::Overlap { pdb, va },
            // Zero-length / u64-wrapping range from hostile guest input: loud, not a panic.
            kayfabe_util::IntervalError::Empty | kayfabe_util::IntervalError::Wraps => {
                AddressFault::Malformed { pdb, va }
            }
        })
    }

    /// Eagerly drop the binding starting at `va`. Returns the dropped binding so the
    /// caller can retire its host backing (reclaim deferred).
    ///
    /// ★ G1 (§12.16): the returned [`Binding::host`] is what makes "retire its host
    /// backing" an executable sentence rather than an aspiration — it names both the
    /// mapping to undo and the object to free.
    pub fn unbind(&mut self, va: GpuVa) -> Option<(u64, Binding)> {
        self.map.remove_at(va.0)
    }

    /// Resolve `va` to its binding + offset within it. **MISS = FAULT** — there is no
    /// fallback and never will be (see crate docs).
    pub fn resolve(&self, pdb: Pdb, va: GpuVa) -> Result<(Binding, u64), AddressFault> {
        match self.map.lookup(va.0) {
            Some((start, _len, b)) => Ok((*b, va.0 - start)),
            None => Err(AddressFault::Miss { pdb, va }),
        }
    }

    /// Iterate bindings as `(va, len, &binding)` in ascending VA order.
    pub fn iter(&self) -> impl Iterator<Item = (u64, u64, &Binding)> {
        self.map.iter()
    }

    /// The binding covering `va`, **with the range it occupies** — `(start, len,
    /// binding)`.
    ///
    /// [`AddressTable::resolve`] answers the question hardware asks (*"what is at this
    /// address"*, offset included, MISS = FAULT) and deliberately hides the extent. A
    /// **populate** asks a different question — *"is what is already here the same shape
    /// as what I am about to write"* — and answering it from `resolve` means guessing the
    /// extent, which is how a leaf silently resizes its neighbour. Hence a second, total
    /// accessor rather than a wider `resolve`: the two callers want different things and
    /// one of them must not be tempted by a fault vocabulary it has no use for.
    #[must_use]
    pub fn binding_at(&self, va: GpuVa) -> Option<(u64, u64, Binding)> {
        self.map.lookup(va.0).map(|(s, l, b)| (s, l, *b))
    }

    /// ★★★ **Which of the owner's four kinds the region at `va` is** — `None` is **kind 1,
    /// unallocated**, and it is the same `None` [`AddressTable::binding_at`] gives.
    ///
    /// ⊘ There is deliberately no `RegionKind::Unallocated` variant to return here: a row
    /// saying *"nothing is here"* and the absence of a row are two spellings of one fact,
    /// and the pair drifts the moment one of them is reachable and the other is not. See
    /// [`RegionKind`].
    #[must_use]
    pub fn kind_at(&self, va: GpuVa) -> Option<RegionKind> {
        self.map.lookup(va.0).map(|(_, _, b)| b.kind())
    }

    /// ★★★ **The range algebra's one primitive** (`#102` stage C2,
    /// `eight_blockers_resolved.md` §12.3): partition `[va, va+len)` into the maximal
    /// runs over which this table's answer is CONSTANT — each either covered by exactly
    /// one binding (`Some((binding, offset-of-this-span-within-it))`) or a hole (`None`).
    ///
    /// ★★★ **The `u64` beside each binding is the span's OFFSET INTO IT**, and it is what
    /// makes the binding's address usable without a second lookup: a span generally starts
    /// *inside* a binding rather than at its base, so `binding.phys` alone is the address of
    /// a different byte. Returning it is not a convenience — `[measured 2026-08-08]` the
    /// executor that consumed this partition wrote at the span's **virtual** address because
    /// the physical one was not in hand here, which is `#12`'s where-mistake
    /// (`execution_plane_increments.md` §14.14 REFUTED 4). ⊘ The doc used to claim the offset
    /// was *"already inside that binding"*; it was not, and the sentence read as though it
    /// were, which is how the gap survived a review.
    ///
    /// This exists because §12.3's ruling is *"the operand ranges must be PARTITIONED,
    /// not classified whole"*: one privileged copy-engine request can cover fabricated
    /// and real memory at once, and classifying it by its start address answers for the
    /// wrong half of it. [`AddressTable::resolve`] is the point query; this is the
    /// range query, and it is deliberately here rather than reimplemented over
    /// [`AddressTable::iter`] at each call site — a partition that is not TOTAL is
    /// silently a dropped sub-copy.
    ///
    /// # Guarantees (all pinned by test)
    /// - Ascending, contiguous, non-overlapping, **no zero-length span**.
    /// - The spans cover the *effective* range EXACTLY — nothing implicit is dropped.
    /// - Total on hostile input: never panics, never allocates unboundedly.
    ///
    /// ★ **A wrapping range is CLIPPED at the top of the address space, never wrapped.**
    /// `va + len` is computed in `u128`; the effective end is `min(va+len, 2^64)`. A
    /// copy that ran off the top and resumed at address 0 is not something a real engine
    /// does, and honouring the wrap would let a hostile length reach a mapping at the
    /// BOTTOM of the space from a request aimed at the top. The clipped surplus
    /// addresses nothing, so it needs no span. (`len == 0`, likewise, yields no spans:
    /// an empty request is empty, not a fault — the guest is allowed to ask for nothing.)
    ///
    /// ★ **Delegates to [`kayfabe_util::IntervalMap::spans`]** — the algorithm moved onto
    /// the container so the GPGA viewer index ([`crate::gpga`]) shares it rather than
    /// growing a second one (`gpga_address_space.md` §5: *"Build it once"*). The
    /// guarantees above are the container's, restated here because this is the signature
    /// callers read.
    #[must_use]
    pub fn spans(&self, va: GpuVa, len: u64) -> Vec<BindingRun> {
        self.map
            .spans(va.0, len)
            .into_iter()
            .map(|(s, l, b)| (s, l, b.map(|(v, off)| (*v, off))))
            .collect()
    }

    /// ★★★ #102 — the identity law as a **whole-table walk**: the first host-backed
    /// binding whose host VA is not the VA it is bound at, or `None` if the table is
    /// clean.
    ///
    /// [`AddressTable::bind`] already refuses such a binding, so in a correct build this
    /// always answers `None`. It exists anyway, and is not redundant: `bind` proves the
    /// law about *every write that went through it*, and this proves it about the
    /// *table*. Those differ the moment anything reaches the map another way — a future
    /// bulk-populate path, a deserialize, a merge. A law with only an entry check is one
    /// refactor away from being a law about nothing.
    ///
    /// ★ It walks **both** entrance laws, for the same reason it walks the first one:
    /// [`AddressTable::bind`] also pins a slice's length to its range
    /// ([`AddressFault::SliceLenMismatch`]), and a law with only an entry check is one
    /// bulk-populate path away from being a law about nothing.
    ///
    /// # Errors
    /// [`AddressFault::HostVaMismatch`] or [`AddressFault::SliceLenMismatch`] naming the
    /// offending range.
    pub fn audit_identity(&self, pdb: Pdb) -> Result<(), AddressFault> {
        for (va, len, b) in self.map.iter() {
            let Some(h) = b.host else { continue };
            if h.host_va() != va {
                return Err(AddressFault::HostVaMismatch {
                    pdb,
                    va: GpuVa(va),
                    host_va: h.host_va(),
                });
            }
            if let Some(s) = h.as_slice()
                && s.len() != len
            {
                return Err(AddressFault::SliceLenMismatch {
                    pdb,
                    va: GpuVa(va),
                    len,
                    slice_len: s.len(),
                });
            }
        }
        Ok(())
    }
}

// The concurrency contract, compile-time-asserted (decision #17).
kayfabe_util::assert_send_sync!(
    AddressTable,
    Binding,
    HostBacking,
    HostExtent,
    HostSlice,
    HostSliceError,
    AddressFault,
    walker::WalkResult,
    walker::Translation,
    walker::TranslateFault,
    walker::PtPage,
    walker::DecodedLeaf,
    walker::PageDecode,
    walker::SubtreeDecode,
    walker::WalkFault,
    walker::DropReason,
    walker::LeafDisposition,
    walker::PopulateRefusal,
    walker::PopulateOutcome
);

#[cfg(test)]
mod tests {
    use super::*;

    const PDB: Pdb = Pdb(0x340_1000);

    /// testing strategy §2.4 `taddr_miss_is_fault`: a lookup miss is a loud fault,
    /// never an opportunistic walk.
    #[test]
    fn taddr_miss_is_fault() {
        let mut t = AddressTable::new();
        t.bind(
            PDB,
            GpuVa(0x2_0020_0000),
            0x10000,
            Binding::declared_by_guest(0x8000_0000, Aperture::SysmemCoherent)
                .expect("sysmem is kind 4"),
        )
        .unwrap();
        // In range: resolves with offset.
        let (b, off) = t.resolve(PDB, GpuVa(0x2_0020_4000)).unwrap();
        assert_eq!((b.phys(), off), (0x8000_0000, 0x4000));
        // Out of range: FAULT, carrying the identity needed for a loud diagnostic.
        assert_eq!(
            t.resolve(PDB, GpuVa(0x2_0030_0000)),
            Err(AddressFault::Miss {
                pdb: PDB,
                va: GpuVa(0x2_0030_0000)
            })
        );
    }

    /// `taddr_unmap_eager`: a removed range faults immediately; rebinding after an
    /// eager unbind succeeds; rebinding over a live range is a loud overlap fault.
    #[test]
    fn taddr_unmap_eager_and_overlap_loud() {
        let mut t = AddressTable::new();
        let bind = Binding::declared_by_guest(0x1000, Aperture::Vidmem).expect("vidmem is kind 2");
        t.bind(PDB, GpuVa(0x1000), 0x1000, bind).unwrap();
        assert_eq!(
            t.bind(PDB, GpuVa(0x1800), 0x1000, bind),
            Err(AddressFault::Overlap {
                pdb: PDB,
                va: GpuVa(0x1800)
            }),
            "re-point without unbind must be loud"
        );
        assert!(t.unbind(GpuVa(0x1000)).is_some());
        assert!(matches!(
            t.resolve(PDB, GpuVa(0x1000)),
            Err(AddressFault::Miss { .. })
        ));
        t.bind(PDB, GpuVa(0x1800), 0x1000, bind).unwrap();
    }

    /// ★★★ `#102` — **`audit_identity` fires.**
    ///
    /// [`AddressTable::bind`] refuses a host-backed binding whose host VA is not the VA
    /// it is bound at, which means **no caller of the public API can build a table this
    /// walk fails on** — so the walk cannot be fired from an integration test, and a
    /// control that cannot be fired is not a control. It is fired here, from inside the
    /// crate, by writing the private map directly.
    ///
    /// That is not cheating: it is *exactly* the situation the walk exists for. `bind`
    /// proves the law about every write that went through it; the walk proves it about
    /// the table. They differ the moment anything else reaches `self.map` — a bulk
    /// populate path, a deserialize, a merge — which is a plausible next commit, not a
    /// hypothetical.
    ///
    /// **Instrument check, performed 2026-07-30:** with `audit_identity`'s body replaced
    /// by `Ok(())`, this test fails on the negative assertion. Restored.
    #[test]
    fn taddr_audit_identity_catches_a_binding_that_bypassed_the_entrance() {
        let mem = kayfabe_isolate::HostHandle::new(
            kayfabe_isolate::IsolateId::new(1, kayfabe_arch::ids::GpuId::ZERO),
            9,
        );
        let honest = |va: u64| {
            Binding::real_gpu_memory(
                0x8000_0000 + va,
                Aperture::SysmemCoherent,
                HostBacking::whole(mem, va, BackingBytes::SoleBacking),
            )
            .expect("host sysmem is kind 3")
        };

        let mut t = AddressTable::new();
        for k in 0..4u64 {
            let va = 0x2_0020_0000 + k * 0x10000;
            t.bind(PDB, GpuVa(va), 0x10000, honest(va)).unwrap();
        }
        assert_eq!(t.audit_identity(PDB), Ok(()), "a clean table audits clean");

        // Bypass the entrance, the way a future populate path would.
        let rogue_va = 0x2_0080_0000u64;
        t.map
            .insert(
                rogue_va,
                0x1000,
                Binding {
                    // ⊘ A literal, not a constructor, and that is the premise: this row
                    // bypasses BOTH entrances. `Binding::real_gpu_memory` would refuse the
                    // `Vidmem` aperture (ruling 3) and `bind` would refuse the host VA.
                    kind: RegionKind::RealGpuMemory,
                    phys: 0x1234_0000,
                    aperture: Aperture::Vidmem,
                    // Published one page away from where it is bound — the exact
                    // state that reads as "mapped" everywhere in core state and
                    // resolves to nothing on the host GPU.
                    host: Some(HostBacking::whole(
                        mem,
                        rogue_va + 0x1000,
                        BackingBytes::SoleBacking,
                    )),
                },
            )
            .expect("the private map takes it — that is the whole premise");
        assert_eq!(
            t.audit_identity(PDB),
            Err(AddressFault::HostVaMismatch {
                pdb: PDB,
                va: GpuVa(rogue_va),
                host_va: rogue_va + 0x1000,
            }),
            "the walk names the offending range, so the diagnostic is actionable"
        );
    }

    fn mem(raw: u64) -> kayfabe_isolate::HostHandle {
        kayfabe_isolate::HostHandle::new(
            kayfabe_isolate::IsolateId::new(1, kayfabe_arch::ids::GpuId::ZERO),
            raw,
        )
    }

    /// §8.2 — the two ranges that are not ranges are refused **at construction**, by
    /// their exact variant. `Empty` and `Wraps` are different mistakes.
    ///
    /// **Instrument check, performed 2026-07-30 — WATCHED IT FAIL.** With both validation
    /// blocks deleted from [`HostSlice::new`]:
    ///
    /// ```text
    /// assertion `left == right` failed
    ///   left: Ok(HostSlice { offset: 4096, len: 0 })
    ///  right: Err(Empty)
    /// ```
    ///
    /// Restored.
    #[test]
    fn hostslice_refuses_the_two_non_ranges() {
        assert_eq!(HostSlice::new(0x1000, 0), Err(HostSliceError::Empty));
        assert_eq!(HostSlice::new(u64::MAX, 1), Err(HostSliceError::Wraps));
        assert_eq!(HostSlice::new(u64::MAX - 1, 2), Err(HostSliceError::Wraps));
        // …and the last representable range is accepted, so `end()` cannot overflow.
        assert_eq!(
            HostSlice::new(u64::MAX - 1, 1).map(HostSlice::end),
            Ok(u64::MAX)
        );
        let s = HostSlice::new(0x2_0000, 0x1000).expect("a real range");
        assert_eq!((s.offset(), s.len(), s.end()), (0x2_0000, 0x1000, 0x2_1000));
    }

    /// ★★ §8.2 — **`bind` refuses a slice whose length is not its range's.**
    ///
    /// **Instrument check, performed 2026-07-30 — WATCHED IT FAIL.** With the
    /// `SliceLenMismatch` block short-circuited in [`AddressTable::bind`]:
    ///
    /// ```text
    /// assertion `left == right` failed: our own bookkeeping must not disagree with itself
    ///   left: Ok(())
    ///  right: Err(SliceLenMismatch { pdb: Pdb(54530048), va: GpuVa(8592031744),
    ///                                len: 65536, slice_len: 4096 })
    /// ```
    ///
    /// Restored.
    #[test]
    fn taddr_bind_refuses_a_slice_that_disagrees_with_its_range() {
        let mut t = AddressTable::new();
        let va = GpuVa(0x2_0020_0000);
        let arena = mem(7);
        let short = HostSlice::new(0x8000, 0x1000).expect("real");
        assert_eq!(
            t.bind(
                PDB,
                va,
                0x10000,
                Binding::real_gpu_memory(
                    0x8000_0000,
                    Aperture::SysmemCoherent,
                    HostBacking::slice(arena, va.0, short, BackingBytes::SoleBacking),
                )
                .expect("host sysmem is kind 3"),
            ),
            Err(AddressFault::SliceLenMismatch {
                pdb: PDB,
                va,
                len: 0x10000,
                slice_len: 0x1000,
            }),
            "our own bookkeeping must not disagree with itself"
        );
        // …and the refusal is not a blanket one: the same slice at its own length binds.
        let honest = HostSlice::new(0x8000, 0x10000).expect("real");
        assert_eq!(
            t.bind(
                PDB,
                va,
                0x10000,
                Binding::real_gpu_memory(
                    0x8000_0000,
                    Aperture::SysmemCoherent,
                    HostBacking::slice(arena, va.0, honest, BackingBytes::SoleBacking),
                )
                .expect("host sysmem is kind 3"),
            ),
            Ok(())
        );
        assert_eq!(t.audit_identity(PDB), Ok(()));
    }

    /// ★★ The whole-table walk carries the slice law too, fired the only way it can be
    /// fired — by writing the private map, exactly as the `#102` arm above does.
    ///
    /// **Instrument check, performed 2026-07-30 — WATCHED IT FAIL.** With the
    /// `SliceLenMismatch` block short-circuited in `audit_identity`:
    ///
    /// ```text
    /// assertion `left == right` failed
    ///   left: Ok(())
    ///  right: Err(SliceLenMismatch { pdb: Pdb(54530048), va: GpuVa(8598323200),
    ///                                len: 4096, slice_len: 8192 })
    /// ```
    ///
    /// Restored.
    #[test]
    fn taddr_audit_identity_catches_a_slice_that_bypassed_the_entrance() {
        let mut t = AddressTable::new();
        let va = 0x2_0080_0000u64;
        t.map
            .insert(
                va,
                0x1000,
                Binding {
                    // ⊘ A literal for the same reason as the `#102` arm above: the row must
                    // bypass the entrance, and both entrances now refuse it.
                    kind: RegionKind::RealGpuMemory,
                    phys: 0x1234_0000,
                    aperture: Aperture::Vidmem,
                    host: Some(HostBacking::slice(
                        mem(9),
                        va,
                        HostSlice::new(0, 0x2000).expect("real"),
                        BackingBytes::SoleBacking,
                    )),
                },
            )
            .expect("the private map takes it — that is the whole premise");
        assert_eq!(
            t.audit_identity(PDB),
            Err(AddressFault::SliceLenMismatch {
                pdb: PDB,
                va: GpuVa(va),
                len: 0x1000,
                slice_len: 0x2000,
            })
        );
    }

    /// §8.2/§9.3 — the ownership regime and the owner scope are both readable from the
    /// backing, and the scope is [`HostHandle`]'s, not a second one.
    #[test]
    fn host_backing_states_who_frees_it_and_whose_it_is() {
        let a = kayfabe_isolate::IsolateId::new(1, kayfabe_arch::ids::GpuId::ZERO);
        let b = kayfabe_isolate::IsolateId::new(2, kayfabe_arch::ids::GpuId::ZERO);
        let arena = kayfabe_isolate::HostHandle::new(a, 0x5c00_0001);

        let whole = HostBacking::whole(arena, 0x1000, BackingBytes::SoleBacking);
        assert!(whole.frees_object(), "sole owner: its release frees it");
        assert_eq!(whole.as_slice(), None);
        assert_eq!(whole.extent(), HostExtent::Whole);

        let s = HostSlice::new(0x4000, 0x1000).expect("real");
        let slice = HostBacking::slice(arena, 0x1000, s, BackingBytes::SoleBacking);
        assert!(
            !slice.frees_object(),
            "a slice never frees the arena it was cut from"
        );
        assert_eq!(slice.as_slice(), Some(s));
        assert_eq!(slice.extent(), HostExtent::Slice(s));

        // The scope is inherited: same answer as the handle's own gate, both ways.
        for backing in [whole, slice] {
            assert_eq!(backing.owner(), a);
            assert!(backing.belongs_to(a));
            assert!(
                !backing.belongs_to(b),
                "a slice must not be nameable from another isolate"
            );
            assert_eq!(backing.belongs_to(b), backing.memory().belongs_to(b));
        }
    }
}
