//! ★★★★★ **ONE GPGA, MANY VIEWS — the registry that makes aliasing a FACT.**
//!
//! **Owner, 2026-09-09:** *"remember same gpga be mapped multiple times in vmm, multiple times
//! in same isolate, across isolates, in scratchpad, and just be allocated with no references."*
//!
//! Five states, one range, all legal and all simultaneous. The tree has been treating that
//! multiplicity as an accident to be discovered in a debugger; here it is a recorded fact.
//!
//! # ⊘ Why this exists at all — two defects, both already paid for
//!
//! - **A bare address carries no lifetime.** `SparseFb::release_join` *dropped the bytes a join
//!   held*; only `release_join_carrying_bytes` preserves them. Anything holding the old address
//!   kept a pointer to nothing.
//! - **An unrecorded alias corrupts silently.** The LLM's sixteen tokens of garbage were a frame
//!   host-backed at **one** VA while the guest aliased **seventeen** at **two**. Nothing printed
//!   the alias, so nothing could contradict the assumption that there was one.
//!
//! ⇒ Views are handed out as **guards**, and every live view of a range is enumerable.
//!
//! # ⊘ What this is NOT
//!
//! It does not map anything. It is the *bookkeeping* — who holds a view of what, in which
//! space — so the mapping layer can stay backing-specific while callers stay backing-agnostic.
//! Keeping the registry separate from the mapper is deliberate: the registry is pure and
//! testable without a VMM, an isolate, or a GPU.

use kayfabe_arch::ids::GpuId;
use std::collections::BTreeMap;

/// Where a view lives. The same GPGA may be viewed in several of these at once, and several
/// times within one of them.
///
/// # ⊘⊘⊘ THREE OF THESE FOUR SPACES ARE PER-GPU, AND ONLY ONE IS NOT
///
/// **Owner, 2026-09-13, on the counter/doorbell page being per-GPU:** *"this means you also
/// have scratchpad probably per gpu."* ★ Correct, and it generalises past the scratchpad.
/// A GPU's framebuffer address space starts at its own zero, so **GPGA `0x1000` on GPU 0 and
/// GPGA `0x1000` on GPU 1 name different memory**. A space that cannot say *which* GPU is a
/// space in which those two collide.
///
/// - [`ViewSpace::Vmm`] is the ONE genuinely GPU-blind space. There is one VMM process and one
///   host virtual address space; a view in it is located by a host VA that is unique on its own.
/// - [`ViewSpace::Scratchpad`] stages work **on a specific host GPU**.
/// - [`ViewSpace::GuestMmio`] is **BAR1**, and the owner's standing requirement is that *"each
///   gpu has its own bar0/1/2"*.
/// - [`ViewSpace::Isolate`] is a sandbox for **one `(proc, gpu)` pair** — that is exactly what
///   `kayfabe_isolate::IsolateId` is, and an isolate bound to `nvidia0` is a different process
///   from the same proc's isolate bound to `nvidia1`.
///
/// ⊘ **The pre-2026-09-13 shape gave the discriminator to the one axis that needed it least.**
/// `Isolate` carried a bare proc id and the other three were unit variants, so the table could
/// tell two procs apart on one GPU and could not tell two GPUs apart at all.
///
/// ⚠ And this type was **cited as the multi-GPU model while lacking a GPU axis**:
/// `kayfabe_mmu::refresh::WalkOutcome::Unsupported` argued peer memory was already structurally
/// anticipated because *"a peer mapping is a view of one GPU's memory in another GPU's space,
/// which is what `kayfabe_device::gpgaview::ViewSpace` models"*. It did not model it. ★ That is
/// this tree's recurring shape — **a citation checks that a claim is SOURCED, never that the
/// source says what the claim says** — arriving as a type that had been vouched for by a
/// neighbour and never asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ViewSpace {
    /// The VMM's own address space (a CPU-side view — fake FB returns host RAM; real FB maps
    /// the MMIO window).
    ///
    /// ⊘ **Deliberately GPU-blind.** One VMM process, one host VA space, shared by every
    /// emulated device. Adding a GPU here would make two names for one place.
    Vmm,
    /// One isolate's address space. Isolates are separate processes, so two isolates are
    /// unrelated spaces that may both view one range.
    ///
    /// ⊘ Keyed by `(proc, gpu)` — the pair `kayfabe_isolate::IsolateId` carries. Spelled as
    /// fields rather than that type because this crate sits below `kayfabe-isolate`; the pair
    /// must match it, and [`ViewSpace::isolate`] is the constructor that keeps them aligned.
    Isolate {
        /// The owning proc's `kayfabe_core::ProcId` value.
        proc: u32,
        /// Which GPU this isolate is the sandbox for.
        gpu: GpuId,
    },
    /// The scratchpad — where real GPU work is staged, **on one host GPU**.
    Scratchpad(GpuId),
    /// The guest's own MMIO aperture (BAR1) **of one emulated GPU**. ⊘ Fake FB may legitimately
    /// appear here at several addresses at once; that is the aliasing the census exists to print.
    GuestMmio(GpuId),
}

impl ViewSpace {
    /// An isolate's space, by the same `(proc, gpu)` pair `kayfabe_isolate::IsolateId` carries.
    #[must_use]
    pub const fn isolate(proc: u32, gpu: GpuId) -> Self {
        Self::Isolate { proc, gpu }
    }

    /// Which GPU this space belongs to, or `None` for the one space that is shared by all of
    /// them.
    ///
    /// ⊘ `None` means **"belongs to every GPU"**, not "unknown". The VMM's address space is
    /// genuinely one space; a caller filtering views by GPU must therefore decide what to do
    /// with VMM views deliberately rather than have them silently fall on one side.
    #[must_use]
    pub const fn gpu(self) -> Option<GpuId> {
        match self {
            Self::Vmm => None,
            Self::Isolate { gpu, .. } | Self::Scratchpad(gpu) | Self::GuestMmio(gpu) => Some(gpu),
        }
    }
}

/// A live view of a GPGA range. Dropping it releases the view.
///
/// ⊘ `#[must_use]`: a view created and immediately dropped is almost always a bug at the call
/// site — the caller wanted the address, took the guard, and let the lifetime end underneath it.
#[derive(Debug)]
#[must_use = "a view guard released immediately makes the address it returned dangle"]
pub struct ViewGuard {
    space: ViewSpace,
    gpga: u64,
    len: u64,
    /// The address the view is reachable at IN ITS OWN SPACE. Meaningless across spaces.
    at: u64,
    /// ⊘ Whether this guard has been released. Read by [`ViewGuard::is_live`] rather than
    /// deleted: a guard that was released and then used is a real bug, and keeping the flag is
    /// what lets a caller assert against it instead of dereferencing a stale address.
    live: bool,
}

impl ViewGuard {
    /// The address this view is reachable at, in its own space.
    #[must_use]
    pub const fn at(&self) -> u64 {
        self.at
    }
    /// Which space this view lives in.
    #[must_use]
    pub const fn space(&self) -> ViewSpace {
        self.space
    }
    /// The GPGA this is a view OF.
    #[must_use]
    pub const fn gpga(&self) -> u64 {
        self.gpga
    }
    /// Bytes viewed.
    #[must_use]
    pub const fn len(&self) -> u64 {
        self.len
    }
    /// Whether this guard is still held. `false` after [`GpgaViews::release`].
    #[must_use]
    pub const fn is_live(&self) -> bool {
        self.live
    }
    /// ⊘ Clippy asks for this beside `len`; a zero-length view is refused at `map`, so it is
    /// always false — stated rather than omitted so nobody adds a zero-length path later.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// One recorded view, for the census.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    pub space: ViewSpace,
    pub at: u64,
    pub len: u64,
}

/// A range's backing, and every live view of it.
#[derive(Debug, Default, Clone)]
struct Entry {
    /// ⊘ A range may exist with NO views — the owner's fifth case, "just be allocated with no
    /// references". That is not a leak and must not be reaped as one.
    views: Vec<View>,
    /// Whether the backing was explicitly allocated. Distinguishes "allocated, unreferenced"
    /// from "never existed", which a `views.is_empty()` test alone cannot.
    allocated: bool,
}

/// Who holds a view of which GPGA, in which space.
#[derive(Debug, Default)]
pub struct GpgaViews {
    by_gpga: BTreeMap<u64, Entry>,
    /// Views whose `Drop` ran after the table was gone. Counted, never panicked — a teardown
    /// ordering issue must not abort a boot.
    orphan_releases: u64,
}

/// Why a `map` was refused. ⊘ Named rather than `Option`: "you asked for nothing" and "that
/// range does not exist" are different mistakes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewRefused {
    /// A zero-length view is meaningless and is almost always a computed-length bug upstream.
    ZeroLength,
}

impl GpgaViews {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `gpga` exists with a backing, with no views yet.
    ///
    /// ⊘ The owner's fifth case. An allocated range with zero views is a legitimate steady
    /// state, and telling it apart from "never existed" is why `allocated` is a field rather
    /// than an inference from an empty view list.
    pub fn allocate(&mut self, gpga: u64) {
        self.by_gpga.entry(gpga).or_default().allocated = true;
    }

    /// Take a view of `gpga` in `space`, reachable at `at`.
    ///
    /// # Errors
    /// [`ViewRefused::ZeroLength`] for `len == 0`.
    pub fn map(
        &mut self,
        gpga: u64,
        len: u64,
        space: ViewSpace,
        at: u64,
    ) -> Result<ViewGuard, ViewRefused> {
        if len == 0 {
            return Err(ViewRefused::ZeroLength);
        }
        let e = self.by_gpga.entry(gpga).or_default();
        e.views.push(View { space, at, len });
        Ok(ViewGuard {
            space,
            gpga,
            len,
            at,
            live: true,
        })
    }

    /// Release a view. Called by [`ViewGuard`]'s owner; see [`GpgaViews::release`].
    fn drop_view(&mut self, g: &ViewGuard) {
        let Some(e) = self.by_gpga.get_mut(&g.gpga) else {
            self.orphan_releases += 1;
            return;
        };
        if let Some(i) = e
            .views
            .iter()
            .position(|v| v.space == g.space && v.at == g.at && v.len == g.len)
        {
            e.views.remove(i);
        } else {
            self.orphan_releases += 1;
        }
    }

    /// Release a guard against this table.
    ///
    /// ⊘ Explicit rather than a `Drop` impl on the guard: the guard would need a back-reference
    /// to the table, which means shared ownership and a lock taken at drop time — on whatever
    /// thread happens to drop it, possibly a vCPU inside a trap. Releasing explicitly keeps the
    /// table's locking the caller's decision, which is the whole point of the blocking rules.
    pub fn release(&mut self, mut g: ViewGuard) {
        self.drop_view(&g);
        g.live = false;
    }

    /// Every live view of `gpga`.
    #[must_use]
    pub fn views_of(&self, gpga: u64) -> &[View] {
        self.by_gpga.get(&gpga).map_or(&[], |e| e.views.as_slice())
    }

    /// How many live views `gpga` has, across every space.
    #[must_use]
    pub fn refcount(&self, gpga: u64) -> usize {
        self.views_of(gpga).len()
    }

    /// Ranges viewed from more than one place — the aliasing the LLM corruption turned on.
    ///
    /// ⊘ Aliasing is **legal**, so this is a census and not a refusal. What was missing before
    /// was not a prohibition but the ability to say it was happening.
    #[must_use]
    pub fn aliased(&self) -> Vec<(u64, usize)> {
        self.by_gpga
            .iter()
            .filter(|(_, e)| e.views.len() > 1)
            .map(|(g, e)| (*g, e.views.len()))
            .collect()
    }

    /// Allocated ranges with no live view — legitimate, and worth printing so it is never
    /// mistaken for a leak.
    #[must_use]
    pub fn allocated_unviewed(&self) -> Vec<u64> {
        self.by_gpga
            .iter()
            .filter(|(_, e)| e.allocated && e.views.is_empty())
            .map(|(g, _)| *g)
            .collect()
    }

    /// One line for the boot report.
    #[must_use]
    pub fn census(&self) -> String {
        let aliased = self.aliased();
        let unviewed = self.allocated_unviewed();
        format!(
            "GPGA-VIEWS ranges={} aliased={} allocated_unviewed={} orphan_releases={}{} \
             ⊘ aliasing is LEGAL — this names it so an assumption of one view can be \
             contradicted; allocated_unviewed is a steady state, NOT a leak",
            self.by_gpga.len(),
            aliased.len(),
            unviewed.len(),
            self.orphan_releases,
            if aliased.is_empty() {
                String::new()
            } else {
                format!(
                    " [{}]",
                    aliased
                        .iter()
                        .take(6)
                        .map(|(g, n)| format!("{g:#x}×{n}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            }
        )
    }
}

#[cfg(test)]
mod one_gpga_many_views {
    //! ★★★★★ **THE OWNER'S FIVE CASES, 2026-09-09**, each a test:
    //!
    //! > *"same gpga be mapped multiple times in vmm, multiple times in same isolate, across
    //! > isolates, in scratchpad, and just be allocated with no references."*
    //!
    //! All five are legal and may hold simultaneously. A design that treats any of them as an
    //! error is wrong about the hardware, and one that cannot SEE them is how the LLM's
    //! aliasing corruption went unnoticed for a campaign.
    use super::*;

    const G: u64 = 0x9200_a900_0000;
    const G0: GpuId = GpuId(0);
    const G1: GpuId = GpuId(1);

    #[test]
    fn one_range_viewed_in_every_space_at_once() {
        let mut t = GpgaViews::new();
        let a = t.map(G, 0x1000, ViewSpace::Vmm, 0x7f00_0000).unwrap();
        let b = t.map(G, 0x1000, ViewSpace::Vmm, 0x7f10_0000).unwrap();
        let c = t
            .map(G, 0x1000, ViewSpace::isolate(1, G0), 0x40_0000)
            .unwrap();
        let d = t
            .map(G, 0x1000, ViewSpace::isolate(1, G0), 0x41_0000)
            .unwrap();
        let e = t
            .map(G, 0x1000, ViewSpace::isolate(2, G0), 0x40_0000)
            .unwrap();
        let f = t
            .map(G, 0x1000, ViewSpace::Scratchpad(G0), 0x10_0000)
            .unwrap();
        let h = t
            .map(G, 0x1000, ViewSpace::GuestMmio(G0), 0xe000_0000)
            .unwrap();

        // twice in the VMM, twice in ONE isolate, once in a SECOND isolate, scratchpad, MMIO
        assert_eq!(t.refcount(G), 7);
        assert_eq!(t.aliased(), vec![(G, 7)]);

        // ⊘ Two views in the same space at different addresses are DISTINCT, not a duplicate:
        // that is exactly the fake-FB-mapped-twice case, and collapsing them would recreate
        // the bug this table exists to expose.
        let vmm: Vec<_> = t
            .views_of(G)
            .iter()
            .filter(|v| v.space == ViewSpace::Vmm)
            .map(|v| v.at)
            .collect();
        assert_eq!(vmm, vec![0x7f00_0000, 0x7f10_0000]);

        for g in [a, b, c, d, e, f, h] {
            t.release(g);
        }
        assert_eq!(t.refcount(G), 0);
        assert_eq!(t.aliased(), vec![]);
    }

    /// The fifth case. ⊘ Zero views is a legitimate steady state and must be distinguishable
    /// from "never existed" — a reaper that cannot tell them apart frees live backings.
    #[test]
    fn allocated_with_no_references_is_a_state_not_a_leak() {
        let mut t = GpgaViews::new();
        t.allocate(G);
        assert_eq!(t.refcount(G), 0);
        assert_eq!(t.allocated_unviewed(), vec![G]);
        assert!(t.census().contains("allocated_unviewed=1"));

        // …and a never-allocated range is NOT reported as an unviewed allocation.
        assert!(!t.allocated_unviewed().contains(&0xdead_0000));
    }

    /// Releasing one view of a range must not disturb its siblings — the refcount bug that
    /// would surface as "the mapping vanished under a thread that still held it".
    #[test]
    fn releasing_one_view_leaves_the_others_alone() {
        let mut t = GpgaViews::new();
        let a = t.map(G, 0x1000, ViewSpace::isolate(1, G0), 0x1000).unwrap();
        let b = t.map(G, 0x1000, ViewSpace::isolate(1, G0), 0x2000).unwrap();
        t.release(a);
        assert_eq!(t.refcount(G), 1);
        assert_eq!(t.views_of(G)[0].at, 0x2000, "the survivor must be b, not a");
        t.release(b);
        assert_eq!(t.refcount(G), 0);
    }

    /// ⊘ A zero-length view is refused BY NAME. It is nearly always a computed length that came
    /// out zero upstream, and silently returning a valid-looking guard over nothing is how such
    /// a bug reaches a memcpy.
    // ★★★ **THE OWNER'S SIXTH CASE, 2026-09-13** — *"this means you also have scratchpad
    // probably per gpu"*, generalised: three of the four spaces are per-GPU, and the table
    // must keep two GPUs' views of the same NUMBER apart.

    #[test]
    fn the_same_gpga_number_on_two_gpus_is_two_distinct_views() {
        // ⊘ The whole point: GPGA `G` on GPU 0 and GPGA `G` on GPU 1 are DIFFERENT memory,
        // because each GPU's framebuffer starts at its own zero. Before the GPU axis existed
        // these two `map` calls were indistinguishable and the table said "one range, aliased".
        let mut t = GpgaViews::new();
        let a = t
            .map(G, 0x1000, ViewSpace::Scratchpad(G0), 0x10_0000)
            .unwrap();
        let b = t
            .map(G, 0x1000, ViewSpace::Scratchpad(G1), 0x10_0000)
            .unwrap();
        assert_ne!(
            a.space(),
            b.space(),
            "two GPUs' scratchpads are not one space"
        );

        let spaces: Vec<_> = t.views_of(G).iter().map(|v| v.space).collect();
        assert_eq!(
            spaces,
            vec![ViewSpace::Scratchpad(G0), ViewSpace::Scratchpad(G1)]
        );
    }

    #[test]
    fn every_per_gpu_space_separates_by_gpu_and_the_vmm_deliberately_does_not() {
        // Each per-GPU space must distinguish; `Vmm` must NOT, and that asymmetry is a
        // decision — one VMM process, one host VA space — not an oversight.
        assert_ne!(ViewSpace::Scratchpad(G0), ViewSpace::Scratchpad(G1));
        assert_ne!(ViewSpace::GuestMmio(G0), ViewSpace::GuestMmio(G1));
        assert_ne!(ViewSpace::isolate(7, G0), ViewSpace::isolate(7, G1));
        assert_eq!(ViewSpace::Vmm, ViewSpace::Vmm);

        assert_eq!(ViewSpace::Scratchpad(G1).gpu(), Some(G1));
        assert_eq!(ViewSpace::GuestMmio(G1).gpu(), Some(G1));
        assert_eq!(ViewSpace::isolate(7, G1).gpu(), Some(G1));
        // ⊘ `None` is "shared by all of them", never "unknown".
        assert_eq!(ViewSpace::Vmm.gpu(), None);
    }

    #[test]
    fn one_procs_two_isolates_on_two_gpus_are_two_spaces() {
        // `IsolateId` is `(proc, gpu)` precisely because one proc gets a SEPARATE sandbox
        // process per target GPU — `SandboxPolicy::for_gpu` binds only `nvidia{gpu}`. A view
        // space keyed on the proc alone would merge two different processes.
        let mut t = GpgaViews::new();
        let _a = t
            .map(G, 0x1000, ViewSpace::isolate(5, G0), 0x40_0000)
            .unwrap();
        let _b = t
            .map(G, 0x1000, ViewSpace::isolate(5, G1), 0x40_0000)
            .unwrap();
        assert_eq!(
            t.refcount(G),
            2,
            "same proc, same address, two GPUs ⇒ two views"
        );
        assert_eq!(t.aliased(), vec![(G, 2)]);
    }

    #[test]
    fn a_zero_length_view_is_refused_by_name() {
        let mut t = GpgaViews::new();
        assert_eq!(
            t.map(G, 0, ViewSpace::Vmm, 0x1000).err(),
            Some(ViewRefused::ZeroLength)
        );
        assert_eq!(t.refcount(G), 0);
    }

    /// ⊘ A release against a table that never knew the view is COUNTED, not panicked: teardown
    /// ordering must not abort a boot, and a silent ignore would hide a real accounting bug.
    #[test]
    fn an_unknown_release_is_counted_rather_than_ignored_or_fatal() {
        let mut t = GpgaViews::new();
        let g = t.map(G, 0x1000, ViewSpace::Vmm, 0x1000).unwrap();
        let mut other = GpgaViews::new();
        other.release(g);
        assert!(
            other.census().contains("orphan_releases=1"),
            "{}",
            other.census()
        );
    }
}
