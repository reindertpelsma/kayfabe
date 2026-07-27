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
//! ## ★ The portability contract — restated here because it was stated nowhere below §4.1
//!
//! > **A second hypervisor backend must cost exactly one adapter crate: no trait
//! > change, no core change.**
//!
//! That is the *whole* claim this crate makes, and it is the one property that decays
//! silently — the first backend's vocabulary drifts inward one identifier at a time,
//! and by the time a second adapter is attempted the port describes one hypervisor's
//! API rather than a hypervisor's capabilities. Two mechanisms hold it:
//!
//! - **The trait names capabilities, never mechanisms.** [`Vmm`] has no method whose
//!   meaning requires knowing which VMM is underneath. Where a capability *is*
//!   backend-conditional, the trait says so per item — see [`IrqSpec::IntxLevel`].
//! - **A CI gate** (`l1_os_shell.md` §6.0, decision #39): these 11 pure crates plus
//!   `kayfabe-rt` are grepped for one hypervisor's **API identifiers** — lock names,
//!   memory-region and device-tree prefixes, deferred-callback types, and the two
//!   English idioms that are pure vendor jargon. It deliberately does *not* match the
//!   vendor's *name*, so naming the adapter crates above stays writable and no
//!   allowlist is ever needed. **The pattern itself is deliberately not reproduced
//!   here**: unlike the `*_unsafe.rs` naming gate, this one is not writable-by-construction
//!   about itself, so its list has exactly one home (`.github/workflows/ci.yml`) and
//!   one prose home (§6.0) — and both are outside the gate's own scope. Copying it into
//!   a gated crate trips it, which is the correct outcome and how this was found.
//!
//! **The foreign-lock class** (`l1_os_shell.md` §6.3) is the sharpest instance. Stated
//! portably: *no lock the VMM owns — one we neither construct nor rank — may be
//! acquired beneath one of our locks, and our entry paths may arrive with one already
//! held.* QEMU's instance is a whole-machine lock; cloud-hypervisor's is a per-device
//! mutex its bus takes across the entire MMIO callback. The rule is the class; the
//! adapters own the instances.
//!
//! **Threading contract** (§4.1 + decision #17): the adapter provides the
//! synchronization; the core provides safety under *any* strategy. [`Device`]'s entry
//! points take **`&self`** and the trait is `Send + Sync`, so an adapter may serialize
//! whole-device (the simplest valid strategy — one lock around the device) *or* shard
//! per-`Proc` exactly as `kayfabe_rt::SharedDevice` already does (ranked locks, `&self`
//! methods, disjoint per-proc planes). See [`Device`] for why `&self` and not
//! `&mut self`. Isolate I/O completes via [`CoreEvent`]s on the adapter's executor,
//! never by re-entry from an isolate thread. [`Vmm`] itself is a *passed argument*, not
//! core-owned state: it is `Send` (handed between adapter threads) but carries no `Sync`
//! bound — the adapter owns its synchronization.

use core::ops::Range;
use core::time::Duration;
use kayfabe_util::Instant;
use std::collections::BTreeMap;

/// Error from a VMM capability call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmmError {
    /// The GPA range is not backed by guest memory **as a unit**: no region covers it,
    /// the range is un-formable (`gpa + len` leaves the 64-bit space), or it leaves the
    /// region it started in. Distinct from [`VmmError::NonRamGpa`], which is the case
    /// where something *is* there and that something is a device.
    BadGpa {
        /// Offending guest-physical address — the first byte that does not resolve.
        gpa: u64,
    },
    /// ★★★ **The GPA resolves to something that is NOT host RAM** — a device register
    /// window (another emulated device's BAR, the platform's own MMIO, our own trapped
    /// BARs). Refused rather than served.
    ///
    /// # Why this is a named variant and not a `BadGpa`
    ///
    /// [`Vmm::gpa_read`] / [`Vmm::gpa_write`] are the only **in-lock-legal** methods
    /// that take a *guest-chosen* address (`l1_os_shell.md` §6.1), and their legality
    /// rests entirely on the implementation being unable to reach VMM-global API
    /// (§6.3's foreign-lock class). The obvious implementation cannot honour that:
    ///
    /// > `[src] v10.2.0 system/physmem.c` — `address_space_write` takes only
    /// > `RCU_READ_LOCK_GUARD()` (`:3448`) and the direct-access branch is a plain
    /// > `memcpy` (`:3370-3376`), **but the same entry point calls
    /// > `prepare_mmio_access(mr)` (`:3250` write, `:3347` read) whenever the target is
    /// > not direct-access, and that acquires the VMM's global lock** (`:3196-3209`)
    /// > unless the target region opted out of it.
    ///
    /// So whether our "memcpy" is a memcpy or a **global-lock acquisition beneath one of
    /// our ranked locks** would be decided by which region the *guest's* address lands
    /// on: §6.3's ABBA inversion, constructed on demand, and invisible to all four of
    /// §6.3's enforcement layers. This variant is the refusal that keeps it
    /// unconstructible — see [`GuestRamMap`], which is where every backend must make it.
    ///
    /// **Do not "fix" this by relying on unassigned memory being harmless.** It happens
    /// to be at v10.2.0 — `io_mem_init` marks `io_mem_unassigned` lockless
    /// (`system/physmem.c:3010-3017`, *"Trivially thread-safe since memory accesses are
    /// rejected"*) — but every ordinary device region in the machine is **not** marked,
    /// and that is the reachable set. The rule is positive (*prove RAM*), never a
    /// deny-list of the regions we happened to think of.
    NonRamGpa {
        /// The first byte of the requested range that resolves to a device region.
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

// =================================================================================
// ★★ The guest-RAM map — the ONE place a guest-chosen GPA is proven to be RAM
// (`l1_os_shell.md` §6.1 / §6.3 / §10.1 item 6; `../reference/qemu_102_facilities.md`
// row 18 + §11.3)
// =================================================================================

/// Backend-scoped identity of one declared guest-physical region. Opaque: the
/// backend maps it to whatever it actually holds (a `Vec` index in the mock, a host
/// pointer into an installed window in a real adapter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RamRegionId(pub u64);

/// What a declared guest-physical region is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RegionKind {
    /// Host RAM. An access is a memcpy into memory we already own, and nothing else —
    /// which is exactly the property that makes [`Vmm::gpa_read`]/[`Vmm::gpa_write`]
    /// in-lock legal.
    Ram,
    /// A device register window: another emulated device's BAR, the platform's own
    /// MMIO, or one of **our** trapped BARs. Never memcpy-able, and on a real backend
    /// an access to one can take a lock we neither own nor rank.
    Device,
}

/// A guest-physical range **proven** to lie wholly inside one host-RAM region.
///
/// The only way to obtain one is [`GuestRamMap::resolve`]. It deliberately carries no
/// pointer and does not outlive the call that produced it — see [`GuestRamMap`]'s
/// "why no `RamGpa` token" note for why handing a resolved pointer across the port
/// would manufacture an R5 obligation this design does not otherwise have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RamSpan {
    /// The region the whole range lies in.
    pub region: RamRegionId,
    /// Byte offset of the range's first byte within that region's backing.
    pub offset: u64,
    /// Length in bytes (as requested).
    pub len: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Region {
    /// Exclusive end, in `u128` so a region may end at exactly 2^64.
    end: u128,
    id: RamRegionId,
    kind: RegionKind,
    /// Offset within `id`'s backing that this region's first byte corresponds to
    /// (non-zero only after a hole was punched in a larger declaration).
    base_off: u64,
}

/// # ★★★ The guest-physical region map — a **positive** RAM proof, in one place
///
/// Every backend's [`Vmm::gpa_read`]/[`Vmm::gpa_write`] MUST resolve through one of
/// these and MUST refuse what it will not prove. It lives in the **port crate**, not
/// in each adapter, for the reason `testing_doctrine.md` §6 rule 1 gives: a rule
/// stated only in an adapter's prose has no reader, and this particular rule is
/// invisible to all four of `l1_os_shell.md` §6.3's enforcement layers — the type
/// system, the lock ranks (a lock we do not take ourselves has no rank), the written
/// list of VMM functions an in-lock-legal method may call, and the CI greps.
///
/// It is the same shape as `kayfabe_core::rmgraph::RmGraph::undeclared_namespace`:
/// **one central check, argued exemptions, the citation inline.** The exemptions here
/// are exactly two, and both are stated in [`GuestRamMap::resolve`].
///
/// ## What it models
///
/// A declared region is `[start, start+len)` of one [`RegionKind`]. Declarations
/// **replace** whatever they overlap, because that is what a guest re-programming a
/// PCI BAR over a range does on a real backend — the device wins the flat view. So
/// the map is maintained by *installs we perform* (guest RAM at realize, our own
/// windows via [`Vmm::map_guest`]/[`Vmm::map_read_native`]) plus whatever topology the
/// adapter learns; it is never a query issued per access.
///
/// ## ★ Why there is no `RamGpa` token type
///
/// The tempting "make it unrepresentable" move is a `resolve` that mints a proof token
/// the caller then passes to a copy. **Do not.** A token that outlives the call is a
/// resolved backing held across a lock gap, so it would (a) require R5 re-validation
/// after every re-lock, since [`Vmm::unmap_guest`] runs lock-free on another thread,
/// and (b) be a use-after-free surface if anybody skipped that. Keeping resolve+copy
/// **indivisible inside one `Vmm` call** is what makes the R5 obligation not arise at
/// all: the core holds only a `u64`, and a `u64` cannot dangle. The unrepresentability
/// this design buys is therefore at the *call* boundary — no resolved backing crosses
/// the port — not in a token.
///
/// ## ★ The lock the fix introduces, named so it is ranked
///
/// A real adapter's map is mutated by the coarse memory plane (window install/remove,
/// `l1_os_shell.md` §6.7 — a "NO" row that runs lock-free) while it is *read* from
/// in-lock-legal accessors with rank 0 or rank 1 held. That needs synchronisation, and
/// it must be a **leaf**: a lock we construct and rank, below rank 1, that acquires
/// nothing beneath it and whose critical section is a bounded memcpy. Closing the
/// foreign-lock hazard by introducing an *unranked* lock of our own would trade one
/// invisible inversion for another.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuestRamMap {
    regions: BTreeMap<u64, Region>,
}

impl GuestRamMap {
    /// An empty map: **every** GPA refuses. The correct starting point — a backend
    /// declares what it installed.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The whole 64-bit guest-physical space as one RAM region.
    ///
    /// Exists because "all of it" is not expressible through [`Self::declare`] (a
    /// `u64` length cannot name 2^64 bytes) and because it is the honest spelling of
    /// the historical mock semantics — *every* GPA is RAM — which several tests depend
    /// on and which a test must be able to *narrow* rather than re-derive.
    #[must_use]
    pub fn all_ram(id: RamRegionId) -> Self {
        let mut m = Self::new();
        m.regions.insert(
            0,
            Region {
                end: 1u128 << 64,
                id,
                kind: RegionKind::Ram,
                base_off: 0,
            },
        );
        m
    }

    /// Declare `[start, start+len)` to be `kind`, backed by `id`. **Replaces** any
    /// overlapping declaration (a device window mapped over RAM wins, as it does in a
    /// real backend's flat view). A zero-length declaration is a no-op.
    ///
    /// Refuses a range that leaves the 64-bit space — a region that wraps is not a
    /// region, and accepting one would make [`Self::resolve`]'s arithmetic a lie.
    pub fn declare(
        &mut self,
        id: RamRegionId,
        kind: RegionKind,
        start: u64,
        len: u64,
    ) -> Result<(), VmmError> {
        let end = u128::from(start) + u128::from(len);
        if end > (1u128 << 64) {
            return Err(VmmError::Unsupported(
                "a guest-physical region that leaves the 64-bit space",
            ));
        }
        if len == 0 {
            return Ok(());
        }
        self.punch(start, end);
        self.regions.insert(
            start,
            Region {
                end,
                id,
                kind,
                base_off: 0,
            },
        );
        Ok(())
    }

    /// Remove `[start, start+len)` from the map entirely (the window-teardown side of
    /// [`Vmm::unmap_guest`]). Afterwards those addresses resolve to nothing at all.
    pub fn undeclare(&mut self, start: u64, len: u64) {
        let end = (u128::from(start) + u128::from(len)).min(1u128 << 64);
        self.punch(start, end);
    }

    /// Keys of every region overlapping `[start, end)`.
    fn overlapping(&self, start: u64, end: u128) -> Vec<u64> {
        let mut out = Vec::new();
        if let Some((&k, r)) = self.regions.range(..=start).next_back()
            && r.end > u128::from(start)
        {
            out.push(k);
        }
        for (&k, _) in self.regions.range((
            core::ops::Bound::Excluded(start),
            core::ops::Bound::Unbounded,
        )) {
            if u128::from(k) >= end {
                break;
            }
            out.push(k);
        }
        out
    }

    /// Remove `[start, end)`, splitting/trimming whatever it cuts and carrying each
    /// remainder's offset into its own backing forward.
    fn punch(&mut self, start: u64, end: u128) {
        for k in self.overlapping(start, end) {
            let r = self.regions.remove(&k).expect("listed as overlapping");
            let (rs, re) = (u128::from(k), r.end);
            if rs < u128::from(start) {
                self.regions.insert(
                    k,
                    Region {
                        end: u128::from(start),
                        ..r
                    },
                );
            }
            if re > end {
                // `end <= 2^64` and `re <= 2^64`, so `re > end` implies `end < 2^64`.
                let new_start = u64::try_from(end).expect("end < 2^64 when re > end");
                let delta = u64::try_from(end - rs).expect("a region is at most 2^64 long");
                self.regions.insert(
                    new_start,
                    Region {
                        end: re,
                        id: r.id,
                        kind: r.kind,
                        base_off: r.base_off + delta,
                    },
                );
            }
        }
    }

    /// ★★★ **The one check.** Prove that `[gpa, gpa+len)` lies wholly inside a single
    /// declared **RAM** region, and say where.
    ///
    /// Refusals, exact and distinguishable (`testing_doctrine.md` §2 rule 3 — these two
    /// are near neighbours and must never start reporting as each other):
    ///
    /// - [`VmmError::NonRamGpa`] — the first offending byte lands in a
    ///   [`RegionKind::Device`] region. **This is the guest-steerable lock inversion**;
    ///   nothing else in the system reports it.
    /// - [`VmmError::BadGpa`] — the first offending byte is in *no* region, or the
    ///   range is un-formable (`gpa + len` leaves the 64-bit space), or it leaves the
    ///   region it started in.
    ///
    /// ## The two argued exemptions
    ///
    /// 1. **A zero-length access still names an address**, and is checked as if it were
    ///    one byte. Serving it would be harmless on any backend (a zero-length transfer
    ///    translates nothing), but the rule this function exists to make total is
    ///    *"every GPA we touch was proven RAM"*, and an exception whose safety argument
    ///    is per-backend is exactly the shape §6.3 says not to write down.
    /// 2. **A range must lie in ONE region.** Not because two RAM regions would be
    ///    a hazard — two memcpys are still two memcpys — but because a single backing per
    ///    access is what the §6.7 memslot design already guarantees (coarse windows, an
    ///    allocator inside), so a straddling descriptor names a shape no legitimate
    ///    guest produces. An adapter that ever needs to serve one must loop **and
    ///    re-prove each step**, never resolve once and copy across.
    pub fn resolve(&self, gpa: u64, len: u64) -> Result<RamSpan, VmmError> {
        let start = u128::from(gpa);
        let end = start + u128::from(len.max(1));
        if end > (1u128 << 64) {
            // Un-formable: the range leaves the address space. Never a wrap-around read.
            return Err(VmmError::BadGpa { gpa });
        }
        let Some((&k, r)) = self.regions.range(..=gpa).next_back() else {
            return Err(VmmError::BadGpa { gpa });
        };
        if r.end <= start {
            return Err(VmmError::BadGpa { gpa });
        }
        if r.kind == RegionKind::Device {
            return Err(VmmError::NonRamGpa { gpa });
        }
        if r.end < end {
            // Leaves its region. Name the first byte outside it, so a straddling
            // descriptor reports the boundary and a wholly-foreign one reports itself.
            let boundary = u64::try_from(r.end).expect("r.end < end <= 2^64");
            return match self.regions.get(&boundary).map(|n| n.kind) {
                Some(RegionKind::Device) => Err(VmmError::NonRamGpa { gpa: boundary }),
                _ => Err(VmmError::BadGpa { gpa: boundary }),
            };
        }
        Ok(RamSpan {
            region: r.id,
            offset: r.base_off + u64::try_from(start - u128::from(k)).expect("within a region"),
            len,
        })
    }
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
    /// MSI-X vector index. **The only variant the core emits** (`Msix(0)`, the
    /// `COMPLETION_VECTOR` edge) and therefore the only one every backend must support.
    Msix(u16),
    /// Legacy INTx line level.
    ///
    /// ★ **Backend-conditional, by design.** A cloud-hypervisor/rust-vmm adapter's
    /// legacy INTx path is a userspace IOAPIC that exists only on x86-64, so on
    /// **CH/aarch64 this variant is unimplementable** — such an adapter MUST return
    /// [`VmmError::Unsupported`] rather than silently dropping the injection. That is
    /// harmless today precisely because the core never emits it; it is written down so
    /// the first adapter to meet it treats it as a contract, not as a bug in the trait.
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
    /// A locked region faulted (the memory-lock primitive — `l1_os_shell.md` §6.7 item
    /// 5: the *implementation* is `UFFDIO_REGISTER` on our own window VMA inside
    /// `kayfabe-linux-raw`, not a [`Vmm`] method; only the fault **delivery** crosses
    /// this seam).
    RegionFault,
}

/// An event delivered back into the core on the device's serialized executor.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum CoreEvent {
    /// A timer/deferred-work callback scheduled via [`Vmm::defer`].
    Deferred(CoreEventKind),
    /// A guest access faulted on a **locked page** inside an installed window, and the
    /// accessing vCPU is parked until the core answers (`l1_os_shell.md` §6.7 item 5).
    ///
    /// The locking itself is a `kayfabe-linux-raw` capability (userfaultfd on the
    /// window's own VMA) and is deliberately **not** a [`Vmm`] method — no VMM
    /// cooperation is required on any backend. Only the fault delivery is here, because
    /// the core must see it on the same serialized executor as everything else.
    LockedRegionFault {
        /// The window whose page faulted (locking is **per page**; the slot names the
        /// containing window, never the granularity).
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
/// One instance per emulated GPU device; object-safe. **Seven** capability groups
/// (arch doc §4.1 — *"count is not the invariant; hypervisor-agnosticism is"*, which is
/// the licence under which the count moved from eight to seven):
///
/// 1. GPA read/write — the DMA plane.
/// 2. Memslot map/unmap — guest-physical mapping management.
/// 3. MMIO trap registration — the VMM routes trapped accesses to [`Device`].
/// 4. Interrupt injection.
/// 5. Guest-RAM export for isolates.
/// 6. Deferred work + virtual time.
/// 7. Read-native overlay (RAM-backed reads, write sub-range traps).
///
/// ★ **Group 8 was removed** (`l1_os_shell.md` §6.7 item 5 + §6.8): the memory-lock
/// primitive (revoke → fault+wait → update → restore) is implemented by
/// `UFFDIO_REGISTER` on **our own** window VMA, which needs no cooperation from any
/// hypervisor. Leaving it on the trait would have forced every adapter to contain the
/// identical userfaultfd code. It is a `kayfabe-linux-raw` capability; only its fault
/// delivery ([`CoreEvent::LockedRegionFault`]) crosses this seam.
///
/// The core never calls the OS or the VMM except through this trait (and
/// `RmBackend` inside isolates). That sentence is the invariant the whole rewrite
/// is judged on.
///
/// **In-lock legality is normative and per-method** (`l1_os_shell.md` §6.1): the
/// memcpy/clock/descriptor-shaped methods may be called under our locks; every
/// syscall-shaped method may not, and asserts so in the real impl. The "may" rows are
/// legal only because their *implementations* cannot reach VMM-global API — a binding
/// requirement on the adapter (§6.3's foreign-lock class), not a property of the name.
pub trait Vmm: Send {
    // --- 1. Guest-physical memory access -----------------------------------------

    /// Read guest-physical memory into `buf` (RPC queue, FB shadow, pushbuffer,
    /// page-table reads).
    ///
    /// # ★★★ NORMATIVE — this is in-lock legal, and ONLY under this implementation shape
    ///
    /// `gpa` is **guest-chosen** on every hot path that uses this (a GPFIFO entry names
    /// it), and this is the one in-lock-legal method that takes one. §6.1's "yes" is
    /// therefore conditional in a way the method name cannot express:
    ///
    /// > An implementation MUST resolve `[gpa, gpa+buf.len())` through a
    /// > [`GuestRamMap`] the adapter maintains from its **own** installs, and memcpy
    /// > into the resolved backing. It MUST refuse a range that does not resolve to
    /// > host RAM — [`VmmError::NonRamGpa`] for a device region,
    /// > [`VmmError::BadGpa`] for nothing-at-all — rather than falling through to a
    /// > generic address-space accessor. It MUST NOT call a VMM's general
    /// > read/write-anywhere API (on QEMU: `address_space_rw` / `_read` / `_write`,
    /// > which take the global lock for a non-direct-access target —
    /// > `[src] v10.2.0 system/physmem.c:3250`, `:3347` → `:3196-3209`).
    ///
    /// Resolution and copy MUST be indivisible within the call: nothing resolved may
    /// cross this port, so no R5 re-validation is owed and no resolved backing can
    /// dangle behind a concurrent [`Vmm::unmap_guest`]. See [`GuestRamMap`].
    fn gpa_read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), VmmError>;

    /// Write `buf` to guest-physical memory (semaphore/status writes into guest RAM).
    ///
    /// **The same normative contract as [`Vmm::gpa_read`] applies verbatim**, and the
    /// write direction is the sharper one: `prepare_mmio_access` is reached from the
    /// write path at `[src] v10.2.0 system/physmem.c:3250`, and a stray write into a
    /// device register window is a side effect on hardware, not merely a bad read.
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
    ///
    /// ★ **Unstated precondition, now stated** (`l1_os_shell.md` §4.4.1): guest RAM
    /// belongs to the VMM, and it is shareable with another process **only if the VM was
    /// LAUNCHED with a shareable backing** — `--memory shared=on` on cloud-hypervisor,
    /// `memory-backend-*,share=on` on QEMU. It is identical on both backends, so it is
    /// portability-neutral; it is written down because it is a **deployment fact that no
    /// code gate can catch**, and the entire isolate double-mmap design rests on it. An
    /// adapter launched without it MUST fail this call with [`VmmError::Unsupported`] at
    /// the first export — loudly, at device realize — never at first guest DMA.
    fn export_ram(&mut self, slice: Option<Range<u64>>) -> Result<RamHandle, VmmError>;

    // --- 6. Deferred work + time --------------------------------------------------

    /// Schedule `event` for delivery to [`Device::event`] after `after`, on the
    /// adapter's serialized executor.
    fn defer(&mut self, after: Duration, event: CoreEvent);

    /// Current virtual time. Deterministic under test (the mock advances it
    /// explicitly); never a wall-clock read in the core.
    fn now(&self) -> Instant;

    // --- 7. Read-native overlay ---------------------------------------------------

    /// Back `gpa..gpa+len` with RAM the core keeps current so guest READS are served
    /// without a VMM op, while `write_trap` (if any) still dispatches writes to
    /// [`Device::mmio_write`] — the rom-device pattern that killed the nested-virt
    /// poll storm (faked-reg class iv-a, decision #12).
    ///
    /// ★ **`write_trap`'s granularity is the HOST PAGE SIZE, not 4 KiB.** Read-native
    /// vs trapped is a *page* attribute in every hypervisor's mapping machinery, so the
    /// backend rounds `write_trap` outward to whole host pages — 4 KiB on x86-64, but
    /// **16 KiB or 64 KiB on arm64** (`portability_arm64.md`; the arch × VMM
    /// intersection). A caller that assumes 4 KiB gets *more* pages trapped than it
    /// asked for on arm64: correct, and quietly slower. Derive the sub-range from
    /// `HostPageSize` (`l1_os_shell.md` §5.2), never from a literal.
    fn map_read_native(
        &mut self,
        gpa: u64,
        len: u64,
        backing: HostRegion,
        write_trap: Option<Range<u64>>,
    ) -> Result<SlotId, VmmError>;
}

/// # The core as seen by the adapter
///
/// The L1 shell (`kayfabe_rt::SharedDevice`, which owns the ranked locks) implements
/// this; a backend routes trapped MMIO and deferred events here.
///
/// ## ★ Why `&self` and not `&mut self` — decided while it was free
///
/// `&mut self` on an MMIO entry point forces **whole-device exclusivity per call**: the
/// adapter must hold one device-wide lock across every trapped access, and the core's
/// per-`Proc` sharding — the #14 fix, the reason `SharedDevice` exists at all — becomes
/// unreachable through the declared port. `kayfabe_rt::SharedDevice` already offers the
/// opposite shape (`&self`, ranked rank-0/rank-1 locks, disjoint per-proc planes), so
/// `&mut self` here would have made the port narrower than the implementation behind it.
///
/// The cost was invisible on the first backend and sharp on the second. A hypervisor
/// that dispatches MMIO through a **`&self`** bus callback (cloud-hypervisor's
/// `BusDeviceSync::read(&self, …)`) would have had exactly two options through a
/// `&mut self` port — wrap the device in another whole-device mutex and throw the
/// sharding away, or bypass the declared port — and the backend that loses is the one
/// with the *better* concurrency story. A backend that serializes everything under its
/// own global lock never feels it.
///
/// `&self` costs the whole-device adapter nothing: an implementor that wants blanket
/// exclusivity puts one lock inside itself and is written exactly as before.
///
/// `Send + Sync` is the same statement at the type level: these entry points are
/// re-entered from several vCPU threads concurrently, which *is* `Sync`, and stating it
/// on the trait is what lets an adapter register a `dyn Device` into a hypervisor bus
/// that demands it.
pub trait Device: Send + Sync {
    /// A trapped MMIO read of `size` bytes at `off` in `bar`.
    fn mmio_read(&self, vmm: &mut dyn Vmm, bar: BarId, off: u64, size: u8) -> u64;

    /// A trapped MMIO write of `size` bytes at `off` in `bar`.
    fn mmio_write(&self, vmm: &mut dyn Vmm, bar: BarId, off: u64, size: u8, val: u64);

    /// A deferred callback, lock fault, or isolate completion.
    fn event(&self, vmm: &mut dyn Vmm, ev: CoreEvent);
}

// The concurrency contract, compile-time-asserted (decision #17): every value type
// crossing the seam is `Send + Sync`; `dyn Vmm` is `Send` by supertrait (the
// threading contract in the crate docs explains why `Sync` is not required).
kayfabe_util::assert_send_sync!(
    VmmError,
    SlotId,
    BarId,
    HostRegion,
    RamRegionId,
    RegionKind,
    RamSpan,
    GuestRamMap,
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
// `Device` is entered concurrently from several vCPU threads, so it is `Send + Sync` by
// supertrait — asserted here so the bound cannot be dropped without this line failing.
kayfabe_util::assert_send_sync!(dyn Device);
