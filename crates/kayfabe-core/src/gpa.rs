//! Per-process GPA arenas (arch doc §4.3.3).
//!
//! [`GpaSpace`] owns the device's guest-physical window; each `Proc` receives a
//! [`GpaArena`] — a contiguous sub-range with its own allocator. Two processes'
//! identical guest VAs therefore land in **disjoint GPA (and host-backing) ranges by
//! construction** — the `back_sys ALREADY-MAPPED` collision class (#14, C cracks
//! ⚠6/⚠10) cannot occur. Arenas are sparse *reservations* (the backing VMM
//! demand-faults), so per-proc cost is address space, not RAM.

use core::ops::Range;
use std::collections::BTreeMap;

use kayfabe_arch::ids::{Gpa, GpuId};

/// Errors from the GPA window/arena allocators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpaError {
    /// The window has no room for another arena.
    WindowExhausted,
    /// The arena has no room for the requested allocation.
    ArenaExhausted {
        /// Requested length.
        len: u64,
    },
}

/// ★ G7 — a release refused because the arena is **not this window's** (`l1_concurrency.md`
/// §12.19). Carries the arena back unchanged: a refused release must never be the thing
/// that loses the range, or the loud check would trade a collision for a leak.
#[derive(Debug)]
pub struct ForeignArena {
    /// The arena, returned untouched, so the caller can route it home or report it.
    pub arena: GpaArena,
    /// The target whose window refused it.
    pub refused_by: GpuId,
}

/// The device's guest-physical window; hands out per-proc arenas.
#[derive(Debug)]
pub struct GpaSpace {
    /// ★ G7: the target this window belongs to. Every arena it carves is stamped with
    /// it, so an arena **names its own window** and a release cannot be misrouted by a
    /// caller that got a map key wrong (`Spine::reap_retired` no longer takes one).
    owner: GpuId,
    window: Range<u64>,
    arena_len: u64,
    next: u64,
    /// Arenas released at a reap point, ready for recycling (LIFO).
    free: Vec<Range<u64>>,
    /// ★ G6: monotonic **generation**, bumped on every carve. It is what makes a
    /// recycled range a *different* arena: a [`GpaBlock`] minted by the range's previous
    /// occupant carries the old generation and is refused by the new one, so the
    /// free-a-stale-block-into-the-arena-that-replaced-it ABA cannot be constructed.
    /// Never reused (the same law as `ResId`).
    next_gen: u64,
}

impl GpaSpace {
    /// A window carving fixed-size arenas of `arena_len` bytes, owned by
    /// [`GpuId::ZERO`] (the realize-time target; further targets use
    /// [`GpaSpace::owned_by`]).
    #[must_use]
    pub fn new(window: Range<u64>, arena_len: u64) -> Self {
        assert!(arena_len > 0 && window.start < window.end);
        let next = window.start;
        GpaSpace {
            owner: GpuId::ZERO,
            window,
            arena_len,
            next,
            free: Vec::new(),
            next_gen: 0,
        }
    }

    /// Stamp this window (and everything it carves) with its owning target — MG-6's
    /// per-target windows. Called at the two places the core mints one.
    #[must_use]
    pub fn owned_by(mut self, owner: GpuId) -> Self {
        self.owner = owner;
        self
    }

    /// The target this window belongs to.
    #[must_use]
    pub fn owner(&self) -> GpuId {
        self.owner
    }

    /// Carve a disjoint arena: recycle a released one first, else cut fresh from
    /// the window. Recycling is what makes the device-lifecycle sustainable — the
    /// C paid for this with #80's GPA free-list after sequential process churn
    /// exhausted the shared window (`teardown_hardening_done`).
    pub fn carve(&mut self) -> Result<GpaArena, GpaError> {
        let generation = self.next_gen;
        self.next_gen += 1;
        if let Some(range) = self.free.pop() {
            return Ok(GpaArena {
                id: ArenaId {
                    owner: self.owner,
                    base: range.start,
                    generation,
                },
                owner: self.owner,
                cursor: range.start,
                range,
                free: BTreeMap::new(),
            });
        }
        let start = self.next;
        let end = start
            .checked_add(self.arena_len)
            .ok_or(GpaError::WindowExhausted)?;
        if end > self.window.end {
            return Err(GpaError::WindowExhausted);
        }
        self.next = end;
        Ok(GpaArena {
            id: ArenaId {
                owner: self.owner,
                base: start,
                generation,
            },
            owner: self.owner,
            range: start..end,
            cursor: start,
            free: BTreeMap::new(),
        })
    }

    /// This window's guest-physical range (used to mint a fresh, disjoint per-target
    /// window for another GPU — MG-6).
    #[must_use]
    pub fn window(&self) -> Range<u64> {
        self.window.clone()
    }

    /// The fixed arena size this window carves (cloned into a new target's geometry).
    #[must_use]
    pub fn arena_len(&self) -> u64 {
        self.arena_len
    }

    /// Return a carved arena to the window for recycling. Takes the arena **by
    /// value**: releasing an arena a live `Proc` still owns is unrepresentable —
    /// only a reaped proc's arena (moved out of the `Proc` at the quiesce point,
    /// `Spine::reap_retired`) can arrive here.
    ///
    /// ## ★ What "safe" rests on, corrected (`l1_concurrency.md` §12.16)
    ///
    /// This doc used to say recycled GPAs are *"safe by construction: the dead proc's
    /// host mappings died with its isolate session, and its address tables died with
    /// its `Vas`es."* One clause of that was a construction and one was a claim about
    /// somebody else's `Drop` that the core neither performed nor checked. Separated:
    ///
    /// - **By construction (true, and still the reason for the by-value signature):**
    ///   the arena is unreachable from any live `Proc` by the time it arrives here, and
    ///   the dead proc's `Vas`es — hence its address tables — go with it. Nothing in
    ///   the core can resolve into the recycled range.
    /// - **Checked, since G3 (was assumed):** the reap only takes a proc whose every
    ///   isolate is **quiesced** (`Isolate::is_quiesced` — no worker checked out), so
    ///   no verb of that isolate can still be in flight, or still land, when the range
    ///   goes back on the free list. Before that check the reap could run inside
    ///   `verb_op`'s lock-free execute gap and recycle a range whose host mapping was
    ///   being written at that moment.
    /// - **An adapter OBLIGATION (not a core guarantee, and never was):** that the
    ///   isolate's own `Drop` actually tears the session down and releases its host
    ///   handles and GPU mappings. The core cannot verify it — the port is a trait, the
    ///   mock has no such `Drop`, and G1's `HostBacking` exists precisely so a reclaimer
    ///   can do it *explicitly* rather than rely on it happening. See the `Isolate` port
    ///   docs; the C relied on the same bulk backstop (`C: src/qemu/virtio_nvgpu.c:100-118`,
    ///   the #80 session reaper force-closing the session's host fds).
    ///
    /// With those three separated, the C's stale-backing / `ALREADY-MAPPED`-on-reuse
    /// class (#12 cont.29) still has nothing to be stale — but for stated reasons, one
    /// of which is somebody else's to keep.
    ///
    /// ## ★ G7 — the WINDOW half, which used to be a `debug_assert` (§12.19)
    ///
    /// By-value made *double* release unrepresentable; it said nothing about *which*
    /// window. Releasing target A's arena into target B's window was expressible in safe
    /// code and was checked only by a `debug_assert!` — **compiled out in release**. The
    /// recipient window then hands a range it does not own to the next proc that asks:
    /// with the core's disjoint per-target windows that is a permanent leak from A plus
    /// a proc holding GPAs inside another target's aperture, and the moment two windows
    /// are not disjoint (a nested/MIG-shaped geometry, which this type permits) it is a
    /// straight **double-issue of one range to two live procs** — the #14 collision class
    /// arriving through the recycling path the per-process design exists to make
    /// impossible.
    ///
    /// It is now a loud [`Result`], and the arena comes back in the error so a refusal
    /// cannot itself lose the range. A *type-level* fix — an arena that cannot even be
    /// passed to the wrong window — would need an invariant "brand" lifetime per window;
    /// the windows live in a runtime-keyed `BTreeMap<GpuId, GpuTarget>`, so every entry
    /// would share one lifetime parameter and brands could not tell target A from target
    /// B. Per-entry brands need existential lifetimes (`generativity`-style), which is
    /// not available in safe stable Rust here. So the structural half was put where the
    /// mistake actually happens instead: the arena is **stamped with its owning target**
    /// at carve time, and `Spine::reap_retired` routes it home by *its own* owner rather
    /// than by a map key the caller supplies — there is no longer a key to get wrong.
    ///
    /// # Errors
    /// [`ForeignArena`] if the arena was not carved by this window.
    pub fn release(&mut self, arena: GpaArena) -> Result<(), ForeignArena> {
        // Owner catches the misrouted release; containment catches the same-owner
        // impostor (two windows stamped alike) that owner alone would wave through.
        if arena.owner != self.owner
            || arena.range.start < self.window.start
            || arena.range.end > self.window.end
        {
            return Err(ForeignArena {
                arena,
                refused_by: self.owner,
            });
        }
        debug_assert_eq!(arena.range.end - arena.range.start, self.arena_len);
        debug_assert!(arena.range.end <= self.next, "never issued by this window");
        self.free.push(arena.range);
        Ok(())
    }
}

/// The identity of one *carve* of a range — window owner + base + **generation**.
///
/// The generation is what distinguishes a recycled range from the range it replaced, so
/// a [`GpaBlock`] cut by a dead proc can never be freed into the arena a later proc was
/// handed at the same address (§12.20's ABA).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArenaId {
    /// The target whose window carved it.
    pub owner: GpuId,
    /// The arena's base GPA.
    pub base: u64,
    /// Monotonic per-window carve counter; never reused.
    pub generation: u64,
}

/// ★ G6 — a live allocation inside one [`GpaArena`], and the **only** way to give it
/// back (`l1_concurrency.md` §12.20).
///
/// Deliberately neither `Copy` nor `Clone`: [`GpaArena::free`] takes it **by value**, so
/// a double free is not a runtime check that can be forgotten, it is a value that no
/// longer exists — the same discipline `GpaSpace::release(arena)` uses one level up. A
/// block also names the exact [`ArenaId`] it came from, so freeing it into a *different*
/// proc's arena — or into the arena that merely inherited its address — is a loud
/// refusal, not a silent double-issue.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "a GpaBlock is a live GPA reservation; drop it and the range leaks until \
              the whole arena is reaped — free it through GpaArena::free"]
pub struct GpaBlock {
    /// The allocation's guest-physical base.
    pub gpa: Gpa,
    /// Its length in bytes.
    pub len: u64,
    arena: ArenaId,
}

impl GpaBlock {
    /// The arena this block was cut from.
    #[must_use]
    pub fn arena(&self) -> ArenaId {
        self.arena
    }
}

/// A [`GpaArena::free`] refused because the block is not this arena's. Carries the block
/// back, for the same reason [`ForeignArena`] carries its arena back.
#[derive(Debug)]
pub struct ForeignBlock {
    /// The block, returned untouched.
    pub block: GpaBlock,
    /// The arena that refused it.
    pub refused_by: ArenaId,
}

/// One process's private slice of the guest-physical window.
///
/// Deliberately **not `Clone`** (§12.19): the by-value [`GpaSpace::release`] is what
/// makes a double release unrepresentable, and a `Clone` would hand that back — two
/// copies of one arena are two releases of one range, i.e. the same range issued to two
/// live procs.
///
/// ★ **Reclaimable since G6** (§12.20). It used to be bump-only — `alloc` and no `free`
/// — so reclamation existed only at whole-arena granularity, at proc reap. A long-lived
/// process that maps and unmaps repeatedly (the exact process this project exists for)
/// walked its cursor to the end and took a permanent `FwdFault::Arena` with no way back:
/// the C's #80 leak (`teardown_hardening_done`: *"Even a well-behaved guest leaked the
/// GPA window (no-free bump allocator) until all GPU mmaps failed"*), reproduced one
/// level down after being fixed one level up.
///
/// It is a **free list, not a collector**. The `RmGraph` already models DUP_OBJECT
/// refcounting from declared protocol facts, so liveness is *known*, not inferred, and
/// cross-`Proc` references cannot exist (a dup between two clients makes them one
/// `Proc`). A tracing GC would re-derive what the graph states and would make
/// reclamation non-deterministic, breaking the `CoreSnapshot` differential property.
#[derive(Debug, PartialEq, Eq)]
pub struct GpaArena {
    /// The arena's disjoint GPA range.
    pub range: Range<u64>,
    /// ★ G7: the target whose window carved this arena — its ticket home.
    owner: GpuId,
    /// ★ G6: this carve's identity; every block it cuts names it.
    id: ArenaId,
    cursor: u64,
    /// ★ G6: freed sub-ranges available for reuse, `start -> len`, kept **coalesced**
    /// with their neighbours. Coalescing is not an optimization here: without it a
    /// map/unmap loop at varying sizes fragments the arena into unusable slivers and
    /// exhausts it anyway, which is the same bug wearing a free list.
    free: BTreeMap<u64, u64>,
}

impl GpaArena {
    /// The target whose window carved this arena. A release routes on THIS, never on a
    /// key the caller remembered separately.
    #[must_use]
    pub fn owner(&self) -> GpuId {
        self.owner
    }

    /// This carve's identity — what a [`GpaBlock`] must match to be freed here.
    #[must_use]
    pub fn id(&self) -> ArenaId {
        self.id
    }

    /// Allocate `len` bytes at `align` (power of two): **reuse first**, then bump.
    ///
    /// Reuse is first-fit over the coalesced free list in ascending GPA order — a pure
    /// function of the arena's state, so the same request stream always yields the same
    /// addresses (decision #27; the differential property depends on it).
    ///
    /// # Errors
    /// [`GpaError::ArenaExhausted`] when neither a free range nor the bump cursor can
    /// satisfy the request.
    pub fn alloc(&mut self, len: u64, align: u64) -> Result<GpaBlock, GpaError> {
        assert!(align.is_power_of_two() && len > 0);
        let exhausted = GpaError::ArenaExhausted { len };

        // 1. Reuse: the lowest free range that can hold an aligned `len`.
        let fit = self.free.iter().find_map(|(&start, &flen)| {
            let a = start.checked_add(align - 1)? & !(align - 1);
            let e = a.checked_add(len)?;
            (e <= start.checked_add(flen)?).then_some((start, flen, a, e))
        });
        if let Some((start, flen, a, e)) = fit {
            self.free.remove(&start);
            if a > start {
                self.free.insert(start, a - start); // alignment head
            }
            if start + flen > e {
                self.free.insert(e, start + flen - e); // tail
            }
            return Ok(GpaBlock {
                gpa: Gpa(a),
                len,
                arena: self.id,
            });
        }

        // 2. Bump.
        let start = self
            .cursor
            .checked_add(align - 1)
            .map(|c| c & !(align - 1))
            .ok_or(exhausted)?;
        let end = start.checked_add(len).ok_or(exhausted)?;
        if end > self.range.end {
            return Err(exhausted);
        }
        // The alignment gap the bump skipped is real space; give it to the free list
        // rather than losing it (a 4K-aligned stream after an odd one would otherwise
        // shed a sliver per allocation).
        if start > self.cursor {
            self.insert_free(self.cursor, start - self.cursor);
        }
        self.cursor = end;
        Ok(GpaBlock {
            gpa: Gpa(start),
            len,
            arena: self.id,
        })
    }

    /// ★ G6 — return one allocation to this arena. Takes the block **by value**, so a
    /// double free is unrepresentable rather than merely detected:
    ///
    /// ```compile_fail
    /// # use kayfabe_core::gpa::GpaSpace;
    /// let mut space = GpaSpace::new(0..0x10000, 0x10000);
    /// let mut arena = space.carve().unwrap();
    /// let block = arena.alloc(0x1000, 0x1000).unwrap();
    /// arena.free(block).unwrap();
    /// arena.free(block).unwrap(); // ← the block was MOVED; this does not compile
    /// ```
    ///
    /// # Errors
    /// [`ForeignBlock`] if the block was cut by a different arena — including the
    /// **previous occupant of this very range** (generations, [`ArenaId`]), which is the
    /// ABA a range-only check would wave through.
    pub fn free(&mut self, block: GpaBlock) -> Result<(), ForeignBlock> {
        if block.arena != self.id {
            return Err(ForeignBlock {
                refused_by: self.id,
                block,
            });
        }
        debug_assert!(block.gpa.0 >= self.range.start && block.gpa.0 + block.len <= self.range.end);
        self.insert_free(block.gpa.0, block.len);
        Ok(())
    }

    /// Insert `[start, start+len)` into the free list, coalescing with the ranges either
    /// side of it (and swallowing the bump cursor back if the range ends at it).
    fn insert_free(&mut self, start: u64, len: u64) {
        let mut lo = start;
        let mut hi = start + len;
        // Merge the predecessor if it abuts.
        if let Some((&ps, &pl)) = self.free.range(..lo).next_back()
            && ps + pl == lo
        {
            self.free.remove(&ps);
            lo = ps;
        }
        // Merge the successor if it abuts.
        if let Some((&ns, &nl)) = self.free.range(hi..).next()
            && ns == hi
        {
            self.free.remove(&ns);
            hi = ns + nl;
        }
        // A free range that runs up to the bump cursor is better given back to the
        // cursor: it keeps the free list short and makes a full drain restore the arena
        // to exactly its pristine shape.
        if hi == self.cursor {
            self.cursor = lo;
            return;
        }
        self.free.insert(lo, hi - lo);
    }

    /// Bytes still handed out (allocated and not freed) — diagnostics, and the executable
    /// statement that a drain actually drains.
    #[must_use]
    pub fn live_bytes(&self) -> u64 {
        (self.cursor - self.range.start) - self.free.values().sum::<u64>()
    }

    /// True if nothing was ever allocated (used to enforce the early-arm merge
    /// discipline: merging two `Proc`s is only legal while the absorbed one is
    /// untouched — lesson L9).
    ///
    /// Deliberately still "has the cursor ever moved", **not** "is anything live": an
    /// absorbed proc that allocated and then freed has published host state and is not
    /// a candidate for an early merge. A fully-drained arena does restore
    /// `cursor == range.start` (see [`GpaArena::insert_free`]) — which is honest, because
    /// at that point the arena is genuinely pristine again.
    #[must_use]
    pub fn is_untouched(&self) -> bool {
        self.cursor == self.range.start && self.free.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// testing strategy §2.3 `t14_already_mapped_arena` (allocator half): two arenas
    /// are disjoint by construction; identical allocation sequences in each yield
    /// disjoint GPAs.
    #[test]
    fn t14_arena_disjoint_by_construction() {
        let mut space = GpaSpace::new(0x1_0000_0000..0x5_0000_0000, 0x2_0000_0000);
        let mut a = space.carve().unwrap();
        let mut b = space.carve().unwrap();
        assert!(a.range.end <= b.range.start || b.range.end <= a.range.start);
        // Identical request streams (two identical guest processes)…
        let ga = a.alloc(0x10000, 0x1000).unwrap();
        let gb = b.alloc(0x10000, 0x1000).unwrap();
        assert_ne!(ga, gb, "identical requests must land at disjoint GPAs");
        assert!(space.carve().is_err(), "window exhaustion is loud");
    }

    /// ★ Mutation-gate kill (`GpaArena::alloc` `end > range.end`→`==`/`>=`): the arena
    /// bound is inclusive of a perfect fill — an allocation whose end lands EXACTLY on
    /// `range.end` must SUCCEED (the last usable byte is available), while any allocation
    /// that would run one byte past must FAIL loudly. The prior suite only ever made
    /// small sub-arena allocations, so the exact-fill boundary was never exercised: a
    /// `>`→`>=` mutant (rejecting the perfect fill) and a `>`→`==` mutant (accepting an
    /// over-run) both survived. This pins the exact boundary from both sides.
    #[test]
    fn arena_alloc_boundary_exact_fill_ok_overrun_loud() {
        let mut space = GpaSpace::new(0..0x10000, 0x10000);
        let mut a = space.carve().unwrap();
        let len = a.range.end - a.range.start; // exactly the whole arena
        // Exact fill: end == range.end must SUCCEED (kills `>`→`>=`).
        let g = a
            .alloc(len, 0x1000)
            .expect("an exact-fill allocation is legal");
        assert_eq!(g.gpa, Gpa(a.range.start));
        // Now the arena is full; a further byte over-runs and must FAIL loudly
        // (kills `>`→`==`, which would accept a one-past-end allocation).
        assert!(
            matches!(a.alloc(1, 1), Err(GpaError::ArenaExhausted { .. })),
            "an allocation past the exact fill is a loud fault, never silently accepted",
        );

        // And an over-run in one shot (len one past the arena) is also loud — the `==`
        // mutant would only reject the exact fill, letting this strictly-larger request
        // through.
        let mut space2 = GpaSpace::new(0..0x10000, 0x10000);
        let mut b = space2.carve().unwrap();
        let over = (b.range.end - b.range.start) + 0x1000;
        assert!(
            matches!(b.alloc(over, 0x1000), Err(GpaError::ArenaExhausted { .. })),
            "an allocation strictly past range.end is a loud fault",
        );
    }

    // ==========================================================================
    // ★ G7 — the WRONG-WINDOW release (`l1_concurrency.md` §12.19)
    // ==========================================================================

    /// ★ **The owner's exact ask: do the cross recycle.** Release an arena belonging to
    /// GPU 0's window into GPU 1's window, with the geometry the core actually mints
    /// (disjoint per-target windows, MG-6), and then allocate from GPU 1.
    ///
    /// The property is deliberately **not** "the release was refused" — that would pass
    /// for the wrong reason the moment the refusal moved. It is the consequence:
    /// **a window only ever hands out ranges it owns.** So the misroute is attempted
    /// tolerantly and the assertions are on what GPU 1 subsequently gives to procs. With
    /// the guard gone, GPU 1's free list swallows the foreign range and hands it to the
    /// very next proc that asks — a proc whose GPAs live inside *another target's*
    /// aperture — while GPU 0 loses that range from its window for good.
    #[test]
    fn g7_a_window_only_ever_hands_out_ranges_it_owns() {
        let w0 = 0x1_0000_0000..0x3_0000_0000;
        let w1 = 0x3_0000_0000..0x5_0000_0000;
        let mut gpu0 = GpaSpace::new(w0.clone(), 0x1_0000_0000).owned_by(GpuId::ZERO);
        let mut gpu1 = GpaSpace::new(w1.clone(), 0x1_0000_0000).owned_by(GpuId(1));

        // A reaped proc's GPU-0 arena, misrouted into GPU 1's window.
        let stolen = gpu0.carve().expect("GPU0 carves");
        let stolen_range = stolen.range.clone();
        assert_eq!(stolen.owner(), GpuId::ZERO);
        let sent_home = gpu1.release(stolen);

        // ★ The property: everything GPU 1 hands out is inside GPU 1's window, and in
        // particular is never the range GPU 0 owns.
        for _ in 0..2 {
            let a = gpu1.carve().expect("GPU1 carves");
            assert!(
                a.range.start >= w1.start && a.range.end <= w1.end,
                "GPU1 handed a proc {:?}, which is NOT inside its own window {w1:?} — \
                 that is GPU0's cross-recycled arena {stolen_range:?}",
                a.range,
            );
            assert_eq!(
                a.owner(),
                GpuId(1),
                "and it is stamped with its real window"
            );
        }

        // The refusal handed the arena BACK, so routing it home is the next line — a
        // loud check that lost the range would trade a collision for a leak.
        let refused = sent_home.expect_err("GPU0's arena is not GPU1's to recycle");
        assert_eq!(refused.refused_by, GpuId(1), "the refusal names the window");
        assert_eq!(
            (refused.arena.range.clone(), refused.arena.owner()),
            (stolen_range.clone(), GpuId::ZERO),
            "the refused arena comes back unchanged, still owned by GPU0",
        );
        gpu0.release(refused.arena)
            .expect("its own window recycles it");
        assert_eq!(
            gpu0.carve().expect("GPU0 carves").range,
            stolen_range,
            "the #80 free list still works"
        );
    }

    /// ★ **The collision itself, and the exact geometry it needs.**
    ///
    /// Under the core's *disjoint* per-target windows a misrouted release cannot produce
    /// two live procs holding one range — the donor's cursor has already passed the
    /// range, so it is leaked rather than re-issued. The moment the two windows are
    /// **not** disjoint it is a straight double issue: the recipient hands the donated
    /// range out of its free list, then hands the *same* range out again as a fresh cut,
    /// because its own cursor never moved past it. Two live procs, byte-identical GPA
    /// ranges — the #14 collision class arriving through the recycling path.
    ///
    /// This is written down rather than argued because it is exactly why the check
    /// cannot lean on "the core mints disjoint windows": that is a property of one call
    /// site today, and `GpaSpace` is a public type whose next geometry (a MIG instance's
    /// window carved inside its parent GPU's — `multi_gpu_and_mig.md`) is precisely the
    /// nested shape.
    #[test]
    fn g7_a_cross_recycled_arena_would_be_one_range_in_two_live_hands() {
        // Two targets over the SAME range — a nested/mis-configured geometry the type
        // permits and the check must survive.
        let shared = 0x1_0000_0000..0x3_0000_0000;
        let mut gpu0 = GpaSpace::new(shared.clone(), 0x1_0000_0000).owned_by(GpuId::ZERO);
        let mut gpu1 = GpaSpace::new(shared.clone(), 0x1_0000_0000).owned_by(GpuId(1));

        let p1 = gpu0.carve().expect("proc P1's arena, on GPU0");
        let p1_range = p1.range.clone();
        let sent_home = gpu1.release(p1);

        // GPU1 now serves two procs.
        let p2 = gpu1.carve().expect("proc P2");
        let p3 = gpu1.carve().expect("proc P3");
        assert!(
            p2.range.end <= p3.range.start || p3.range.end <= p2.range.start,
            "two LIVE procs were handed OVERLAPPING GPA ranges: P2 {:?} vs P3 {:?} \
             (the cross-recycled arena was {p1_range:?})",
            p2.range,
            p3.range,
        );

        let refused = sent_home.expect_err("a foreign arena must not enter GPU1's list");
        assert_eq!(refused.refused_by, GpuId(1));
        assert_eq!(refused.arena.owner(), GpuId::ZERO);
    }

    // ==========================================================================
    // ★ G6 — the arena had `alloc` and no `free` (`l1_concurrency.md` §12.20)
    // ==========================================================================

    /// ★ **The reclamation property itself.** An arena of 64 KiB serves 4096 allocations
    /// of 4 KiB — 16 MiB, 256× its own size — because each is given back. Bump-only, the
    /// 17th request would already have been a permanent `ArenaExhausted`.
    ///
    /// The second half is the one a free list usually fails: **fragmentation**. The same
    /// arena then services a mixed-size stream with interleaved frees (so ranges are
    /// returned out of order and out of adjacency), which a non-coalescing free list
    /// shreds into unusable slivers — the same bug wearing a free list. A full drain must
    /// restore the arena to genuinely pristine, not merely "has some space left".
    #[test]
    fn g6_an_arena_serves_far_more_than_its_size_across_alloc_free_cycles() {
        let mut space = GpaSpace::new(0..0x1_0000, 0x1_0000);
        let mut a = space.carve().expect("carves");

        for i in 0..4096u64 {
            let b = a
                .alloc(0x1000, 0x1000)
                .unwrap_or_else(|e| panic!("cycle {i} exhausted a reclaimed arena: {e:?}"));
            a.free(b).expect("its own block");
        }
        assert_eq!(a.live_bytes(), 0);
        assert!(a.is_untouched(), "a fully drained arena is pristine again");

        // Mixed sizes, freed out of order — the fragmentation shape.
        for round in 0..64u64 {
            let mut held = Vec::new();
            for k in 0..4u64 {
                let len = 0x1000 * (1 + (k + round) % 3);
                held.push(
                    a.alloc(len, 0x1000)
                        .unwrap_or_else(|e| panic!("round {round} alloc {k}: {e:?}")),
                );
            }
            // Free the odd ones first, then the even ones: adjacency is only recoverable
            // by coalescing.
            let (odd, even): (Vec<_>, Vec<_>) =
                held.into_iter().enumerate().partition(|(i, _)| i % 2 == 1);
            for (_, b) in odd.into_iter().chain(even) {
                a.free(b).expect("its own block");
            }
        }
        assert_eq!(a.live_bytes(), 0, "every byte came back");
        assert!(
            a.is_untouched(),
            "and the arena is pristine, not merely non-empty"
        );
    }

    /// ★ **The ABA the owner asked for, scripted by ORDER rather than by timing.**
    ///
    /// A block outlives the arena it was cut from: the proc is reaped, its arena goes
    /// back to the window, and the window hands the **same range** to the next proc
    /// (LIFO — that is the #80 recycle working as designed). A range-only check would
    /// see a block that fits perfectly inside the new arena and take it, silently
    /// donating a live proc's range to a dead proc's bookkeeping. The [`ArenaId`]
    /// generation makes the recycled range a *different* arena, so the stale free is a
    /// loud refusal — and the new arena's own allocations are unaffected.
    #[test]
    fn g6_a_stale_block_cannot_be_freed_into_the_arena_that_replaced_its_range() {
        let mut space = GpaSpace::new(0..0x1_0000, 0x1_0000);
        let mut dead = space.carve().expect("proc A's arena");
        let stale = dead.alloc(0x1000, 0x1000).expect("proc A allocates");
        let stale_gpa = stale.gpa;
        let dead_id = dead.id();
        space.release(dead).expect("A is reaped");

        let mut live = space.carve().expect("proc B's arena");
        assert_eq!(live.range, 0..0x1_0000, "B was handed A's recycled range");

        // The stale free is attempted tolerantly, because the property is the
        // CONSEQUENCE, not the refusal: if the block gets in, B's free list holds a range
        // its bump cursor has not passed, so B hands that one range out TWICE.
        let refused = live.free(stale);
        let b1 = live.alloc(0x1000, 0x1000).expect("B allocates");
        let b2 = live.alloc(0x1000, 0x1000).expect("B allocates again");
        assert_ne!(
            b1.gpa, b2.gpa,
            "the stale free re-armed a range: B handed {:?} to two live allocations",
            b1.gpa
        );
        assert_eq!(
            live.live_bytes(),
            0x2000,
            "exactly what B asked for is live"
        );

        let refused = refused.expect_err("a dead proc's block is not a live proc's to reclaim");
        assert_eq!(refused.refused_by, live.id());
        assert_ne!(
            live.id(),
            dead_id,
            "the recycled range is a DIFFERENT arena"
        );
        assert_eq!(
            refused.block.arena(),
            dead_id,
            "the block still names its own arena"
        );
        assert_eq!(refused.block.gpa, stale_gpa);
    }

    /// ★ **A block belongs to ONE arena, so it cannot be freed into another proc's.**
    /// The #14 boundary applied to reclamation: without it, proc A returning a range into
    /// proc B's arena hands B a range inside A's — the G7 collision one level down.
    #[test]
    fn g6_a_block_cannot_be_freed_into_another_procs_arena() {
        let mut space = GpaSpace::new(0..0x2_0000, 0x1_0000);
        let mut a = space.carve().expect("proc A");
        let mut b = space.carve().expect("proc B");
        let block = a.alloc(0x1000, 0x1000).expect("A allocates");
        let gpa = block.gpa;

        // Tolerant again: the property is that B never issues A's range, not that the
        // call returned `Err`.
        let refused = b.free(block);

        // B never issues A's range…
        for _ in 0..4 {
            let x = b.alloc(0x1000, 0x1000).expect("B allocates");
            assert_ne!(x.gpa, gpa, "B handed out proc A's live range {gpa:?}");
            assert!(
                x.gpa.0 >= b.range.start && x.gpa.0 < b.range.end,
                "B handed out {:?}, outside its own arena {:?}",
                x.gpa,
                b.range
            );
        }
        // …and A can still reclaim it itself, because the refusal handed it back.
        let refused = refused.expect_err("A's range is not B's to reclaim");
        assert_eq!(refused.refused_by, b.id());
        assert_eq!(refused.block.arena(), a.id());
        a.free(refused.block).expect("its own arena takes it");
        assert_eq!(a.live_bytes(), 0);
    }

    /// ★ Mutation-gate kill (`GpaArena::is_untouched`→`true`): the early-arm merge
    /// discipline (lesson L9) depends on this predicate distinguishing a pristine arena
    /// from one that has carved even a single allocation — merging is legal ONLY while
    /// the absorbed proc is untouched. The prior suite never asserted it, so a
    /// "always untouched" mutant survived (it would let a touched proc be silently
    /// merge-absorbed). A fresh arena is untouched; after ANY alloc it is not.
    #[test]
    fn is_untouched_flips_on_first_allocation() {
        let mut space = GpaSpace::new(0..0x10000, 0x10000);
        let mut a = space.carve().unwrap();
        assert!(
            a.is_untouched(),
            "a freshly carved arena has allocated nothing"
        );
        let _held = a.alloc(0x1000, 0x1000).unwrap();
        assert!(
            !a.is_untouched(),
            "after one allocation the arena is touched"
        );
    }

    // ==========================================================================
    // ★★ ADDRESS OBSERVABILITY — the arena's ARITHMETIC, not merely its ledger
    //
    // Measured finding (mutation round 2026-07-27, base `1132ea8`): the first 37
    // reported outcomes were 37 MISSED / 0 CAUGHT, **every one of them inside this
    // file's address arithmetic** — `alloc`'s alignment mask (`align - 1`, `&`,
    // `!(align - 1)`), its free-list fit test (`<=`), its alignment-head test (`>`),
    // `release`'s window-bounds disjunction (`||`) and `live_bytes`' subtraction (`-`).
    //
    // The cause was measured too, not guessed: `GpaSpace` appears at 74 test sites, so
    // it is heavily exercised — but no test in the repository asserted **which address**
    // the arena returns. Every assertion was conservation-shaped (the ledger balances,
    // nothing leaks, nothing double-frees, `Outstanding == Reachable`), and
    // **conservation is invariant to address arithmetic**: an arena handing out
    // wrong-but-self-consistent addresses balances perfectly and passes all of it.
    //
    // That matters here specifically. In Mode 2 the GPA is what gets published into the
    // guest's page tables, so a wrong-but-consistent GPA is the guest reading the wrong
    // memory **while our ledger reports perfect balance** — the silent-corruption class,
    // under a green instrument (`testing_doctrine.md` §1, §2 "assert the exact thing").
    //
    // The tests below pin the four properties the arithmetic actually controls —
    // ALIGNMENT, CONTAINMENT, NON-OVERLAP and EXACT PLACEMENT ON REUSE — plus
    // `live_bytes` against a **second, dumber accounting** kept by the test.
    // ==========================================================================

    /// ★ The auditor: the test's own, independent model of what the arena has handed
    /// out. Deliberately a second implementation — an accounting function checked
    /// against itself is not checked at all, and [`GpaArena::live_bytes`] is one of the
    /// functions under test here.
    #[derive(Debug, Default)]
    struct Ledger {
        /// `gpa -> len` for every block the test still holds.
        held: BTreeMap<u64, u64>,
        /// High-water mark of every address ever issued — how the tests below prove the
        /// **reuse** path was actually taken rather than assuming it (an allocation that
        /// ends at or below the previous high-water mark came out of the free list).
        hwm: u64,
        /// Allocations served from below the high-water mark.
        reused: usize,
    }

    impl Ledger {
        /// Record one allocation, asserting every address property the arena's
        /// arithmetic controls.
        fn take(&mut self, arena: &GpaArena, b: &GpaBlock, align: u64, ctx: &str) {
            let (g, len) = (b.gpa.0, b.len);
            // ---- ALIGNMENT. A misaligned GPA goes straight into a guest page table.
            assert_eq!(g % align, 0, "{ctx}: {:?} is not {align:#x}-aligned", b.gpa);
            // ---- CONTAINMENT inside the arena's declared bounds.
            let end = g.checked_add(len).expect("a block that does not wrap u64");
            assert!(
                g >= arena.range.start && end <= arena.range.end,
                "{ctx}: block {g:#x}..{end:#x} leaves its arena {:?}",
                arena.range,
            );
            // ---- NON-OVERLAP with every other LIVE block: check the neighbour on each
            // side, which is sufficient because the set is kept disjoint inductively.
            if let Some((&ps, &pl)) = self.held.range(..=g).next_back() {
                assert!(
                    ps + pl <= g,
                    "{ctx}: {g:#x}..{end:#x} overlaps the live block {ps:#x}..{:#x}",
                    ps + pl,
                );
            }
            if let Some((&ns, &nl)) = self.held.range(g..).next() {
                assert!(
                    end <= ns,
                    "{ctx}: {g:#x}..{end:#x} overlaps the live block {ns:#x}..{:#x}",
                    ns + nl,
                );
            }
            assert_eq!(
                self.held.insert(g, len),
                None,
                "{ctx}: {g:#x} was issued twice while still live",
            );
            if end <= self.hwm {
                self.reused += 1;
            }
            self.hwm = self.hwm.max(end);
            self.audit(arena, ctx);
        }

        /// Record one free (call *after* [`GpaArena::free`] succeeded).
        fn give(&mut self, arena: &GpaArena, gpa: u64, ctx: &str) {
            assert!(
                self.held.remove(&gpa).is_some(),
                "{ctx}: freed {gpa:#x}, which the ledger never held",
            );
            self.audit(arena, ctx);
        }

        /// ★ The second accounting: `live_bytes` must equal the summed size of the
        /// blocks the TEST still holds, computed without touching the arena's cursor or
        /// its free list.
        fn audit(&self, arena: &GpaArena, ctx: &str) {
            assert_eq!(
                arena.live_bytes(),
                self.held.values().sum::<u64>(),
                "{ctx}: live_bytes disagrees with the independently summed live blocks",
            );
        }
    }

    /// ★★ **A freed range is reused at EXACTLY its own address.**
    ///
    /// This is the assertion the whole file was missing. `alloc`'s reuse branch computes
    /// the address as `(start + align - 1) & !(align - 1)` and accepts the range iff
    /// `a + len <= start + flen`; every arithmetic mutant of those two lines produces
    /// either a *different in-arena address* or a fall-through to the bump cursor, and
    /// **both are invisible to a conservation assertion** — the ledger balances either
    /// way. Pinning the exact address is what makes them visible.
    ///
    /// The freed block is deliberately NOT the one adjacent to the cursor: `insert_free`
    /// gives a cursor-abutting range straight back to the cursor, so freeing the last
    /// allocation never populates the free list at all and the reuse branch would not be
    /// reached (the shape every existing churn test in this file has).
    #[test]
    fn a_freed_range_is_reused_at_exactly_its_own_address() {
        // NON-ZERO base throughout: with an arena based at 0, `cursor - range.start` and
        // `cursor + range.start` are the same number and half of these checks would be
        // vacuous.
        const BASE: u64 = 0x4_0000_0000;
        let mut space = GpaSpace::new(BASE..BASE + 0x10_0000, 0x10_0000);
        let mut a = space.carve().expect("carves");
        let mut led = Ledger::default();

        let a0 = a.alloc(0x1000, 0x1000).expect("A");
        led.take(&a, &a0, 0x1000, "A");
        let b0 = a.alloc(0x1000, 0x1000).expect("B");
        led.take(&a, &b0, 0x1000, "B");
        let c0 = a.alloc(0x1000, 0x1000).expect("C");
        led.take(&a, &c0, 0x1000, "C");
        assert_eq!(
            (a0.gpa, b0.gpa, c0.gpa),
            (Gpa(BASE), Gpa(BASE + 0x1000), Gpa(BASE + 0x2000)),
            "a pristine arena bumps from its own base, packed",
        );

        // Free the MIDDLE one: C still pins the cursor, so B's range genuinely lands on
        // the free list instead of being swallowed back.
        let b_gpa = b0.gpa;
        a.free(b0).expect("its own block");
        led.give(&a, b_gpa.0, "free B");

        // ★ THE ASSERTION. The next same-shaped request must land on B's exact address.
        let d = a.alloc(0x1000, 0x1000).expect("D reuses B's hole");
        assert_eq!(
            d.gpa, b_gpa,
            "a first-fit free list must re-issue the freed range ITSELF, not a fresh \
             bump address and not a shifted one",
        );
        led.take(&a, &d, 0x1000, "D");

        // ★ A SPLIT reuse: free a 3-page range in the middle and take one page out of
        // it, then the rest — the head, then the tail, each at its exact address.
        let e = a.alloc(0x3000, 0x1000).expect("E");
        let e_gpa = e.gpa;
        led.take(&a, &e, 0x1000, "E");
        let f = a.alloc(0x1000, 0x1000).expect("F pins the cursor past E");
        led.take(&a, &f, 0x1000, "F");
        a.free(e).expect("its own block");
        led.give(&a, e_gpa.0, "free E");

        let g = a.alloc(0x1000, 0x1000).expect("G takes E's head");
        assert_eq!(g.gpa, e_gpa, "first fit takes the HEAD of the freed range");
        led.take(&a, &g, 0x1000, "G");
        let h = a.alloc(0x2000, 0x1000).expect("H takes E's tail");
        assert_eq!(
            h.gpa,
            Gpa(e_gpa.0 + 0x1000),
            "and the remainder stays on the free list at its own address",
        );
        led.take(&a, &h, 0x1000, "H");
    }

    /// ★★ **A free range that is too small must NOT be handed out** — the other
    /// direction of the same fit test, and the one that produces an *overlapping* GPA
    /// rather than a merely-different one.
    ///
    /// Inverting `a + len <= start + flen` makes the allocator accept exactly the ranges
    /// that do not fit. The returned address is then inside the arena, page-aligned, and
    /// perfectly accounted for — and it runs straight through the live block next door.
    /// Nothing conservation-shaped can see this: the ledger still balances.
    #[test]
    fn a_free_range_too_small_for_the_request_is_never_handed_out() {
        const BASE: u64 = 0x8_0000_0000;
        let mut space = GpaSpace::new(BASE..BASE + 0x10_0000, 0x10_0000);
        let mut a = space.carve().expect("carves");
        let mut led = Ledger::default();

        // One page hole with a LIVE block immediately after it.
        let p = a.alloc(0x1000, 0x1000).expect("P");
        led.take(&a, &p, 0x1000, "P");
        let q = a.alloc(0x1000, 0x1000).expect("Q — the hole");
        led.take(&a, &q, 0x1000, "Q");
        let r = a.alloc(0x1000, 0x1000).expect("R — the victim next door");
        led.take(&a, &r, 0x1000, "R");
        let (p_gpa, q_gpa, r_gpa) = (p.gpa, q.gpa, r.gpa);
        a.free(q).expect("its own block");
        led.give(&a, q_gpa.0, "free Q");

        // A TWO-page request cannot come out of a one-page hole.
        let big = a
            .alloc(0x2000, 0x1000)
            .expect("the two-page request is served");
        led.take(&a, &big, 0x1000, "big"); // ← non-overlap assertion bites here
        assert_ne!(
            big.gpa, q_gpa,
            "a 0x2000 request was served out of a 0x1000 hole, so it runs into {r_gpa:?}",
        );
        assert!(
            big.gpa.0 >= r_gpa.0 + 0x1000,
            "it must come from past the live blocks, not from inside them",
        );
        // …and the hole is still there, still exactly one page, still reusable.
        let small = a.alloc(0x1000, 0x1000).expect("a fitting request");
        assert_eq!(small.gpa, q_gpa, "the hole survived, at its own address");
        led.take(&a, &small, 0x1000, "small");
        assert_eq!(p_gpa, Gpa(BASE), "and nothing moved underneath it");
    }

    /// ★★ **The alignment head is REAL SPACE and must go back on the free list.**
    ///
    /// When a free range's start is not aligned to the request, `alloc` rounds up and
    /// hands the skipped head back (`if a > start`). Turning that test into `a == start`
    /// does two things at once, and only one of them is visible to conservation: the
    /// head is silently leaked (so `live_bytes` over-reports and the arena can never
    /// return to pristine), and an *aligned* fit inserts a zero-length free entry.
    ///
    /// This is also the sharpest `live_bytes` test in the file, because it is the only
    /// state with a **non-empty free list and a non-zero arena base** at the same time —
    /// which is what makes both of that function's subtractions non-vacuous.
    #[test]
    fn an_alignment_head_is_returned_to_the_free_list_not_leaked() {
        // BASE is 0x2000-aligned, so BASE + 0x1000 is NOT — the misaligned free-range
        // start the head path needs.
        const BASE: u64 = 0x8000_0000;
        let mut space = GpaSpace::new(BASE..BASE + 0x10_0000, 0x10_0000);
        let mut a = space.carve().expect("carves");
        let mut led = Ledger::default();

        let x = a.alloc(0x1000, 0x1000).expect("X");
        led.take(&a, &x, 0x1000, "X");
        let y = a
            .alloc(0x3000, 0x1000)
            .expect("Y — becomes the misaligned hole");
        led.take(&a, &y, 0x1000, "Y");
        let z = a.alloc(0x1000, 0x1000).expect("Z pins the cursor");
        led.take(&a, &z, 0x1000, "Z");
        let y_gpa = y.gpa;
        assert_eq!(
            y_gpa,
            Gpa(BASE + 0x1000),
            "the hole starts 0x2000-MISaligned"
        );
        a.free(y).expect("its own block");
        led.give(&a, y_gpa.0, "free Y");

        // A 0x2000-aligned page must round up INSIDE the hole, leaving a one-page head.
        let w = a.alloc(0x1000, 0x2000).expect("W");
        assert_eq!(
            w.gpa,
            Gpa(BASE + 0x2000),
            "the aligned allocation lands at the rounded-up address inside the hole",
        );
        // `Ledger::take` audits `live_bytes` here: with the head leaked it reads
        // 0x4000 against the ledger's 0x3000.
        led.take(&a, &w, 0x2000, "W");

        // ★ And the head is not merely accounted for, it is USABLE — the next unaligned
        // request must land on it, at its exact address.
        let head = a.alloc(0x1000, 0x1000).expect("the head is reusable");
        assert_eq!(
            head.gpa, y_gpa,
            "the skipped alignment head must be handed out again at its own address",
        );
        led.take(&a, &head, 0x1000, "head");
        // …and so must the tail.
        let tail = a.alloc(0x1000, 0x1000).expect("the tail is reusable");
        assert_eq!(tail.gpa, Gpa(BASE + 0x3000), "the tail, at its own address");
        led.take(&a, &tail, 0x1000, "tail");

        // A full drain restores the arena to genuinely pristine — impossible if any
        // fraction of a byte was leaked or a zero-length entry was left behind.
        for b in [x, z, w, head, tail] {
            let g = b.gpa.0;
            a.free(b).expect("its own block");
            led.give(&a, g, "drain");
        }
        assert_eq!(a.live_bytes(), 0, "every byte came back");
        assert!(
            a.is_untouched(),
            "a fully drained arena is pristine again — a leaked head or a stray \
             zero-length free entry both show up right here",
        );
    }

    /// ★★ **THE MEAN one, at unit granularity**: a long alloc/free churn at mixed sizes
    /// and mixed alignments, driven to exhaustion and back, with every address property
    /// asserted on **every** allocation and `live_bytes` audited against an independent
    /// sum after **every** operation.
    ///
    /// Deterministic (a fixed xorshift), so a failure is reproducible and the arena's
    /// pure-function property (decision #27: the same request stream always yields the
    /// same addresses) is not quietly given up.
    ///
    /// Non-vacuity is asserted, not assumed: the run must actually reach exhaustion, and
    /// it must actually serve allocations out of the free list (`Ledger::reused`).
    #[test]
    fn arena_addresses_stay_aligned_disjoint_and_contained_under_mean_churn() {
        const BASE: u64 = 0x1_0000_0000 + 0x8000; // non-zero, and not arena-aligned
        const ARENA: u64 = 0x20_0000; // 2 MiB = 512 pages
        let mut space = GpaSpace::new(BASE..BASE + 4 * ARENA, ARENA);
        let mut a = space.carve().expect("carves");
        let mut led = Ledger::default();
        let mut held: Vec<GpaBlock> = Vec::new();
        let mut rng: u64 = 0x243F_6A88_85A3_08D3;
        let mut roll = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        const ALIGNS: [u64; 4] = [0x1000, 0x2000, 0x4000, 0x8000];
        let mut exhausted = 0usize;
        let mut served = 0u64;

        for step in 0..4000u64 {
            let r = roll();
            // Bias towards allocating while the arena is empty-ish and towards freeing
            // once it is full, so the run spends its time near the exhaustion boundary —
            // the fragmented state, not the easy one.
            let want_alloc = held.len() < 8 || !r.is_multiple_of(4);
            if want_alloc {
                let len = (1 + (r >> 8) % 8) * 0x1000;
                let align = ALIGNS[((r >> 16) % 4) as usize];
                match a.alloc(len, align) {
                    Ok(b) => {
                        led.take(&a, &b, align, &format!("step {step} alloc"));
                        served += len;
                        held.push(b);
                    }
                    Err(e) => {
                        // Exhaustion is LOUD and names the request, exactly.
                        assert_eq!(
                            e,
                            GpaError::ArenaExhausted { len },
                            "step {step}: exhaustion must name the request it refused",
                        );
                        exhausted += 1;
                        // Make room, so the churn keeps running near the boundary.
                        for _ in 0..4 {
                            if held.is_empty() {
                                break;
                            }
                            let i = (roll() as usize) % held.len();
                            let b = held.swap_remove(i);
                            let g = b.gpa.0;
                            a.free(b).expect("its own block");
                            led.give(&a, g, &format!("step {step} relief"));
                        }
                    }
                }
            } else if !held.is_empty() {
                let i = (r as usize) % held.len();
                let b = held.swap_remove(i);
                let g = b.gpa.0;
                a.free(b).expect("its own block");
                led.give(&a, g, &format!("step {step} free"));
            }
        }

        // ---- the run reached the states it claims to test.
        assert!(
            exhausted > 0,
            "the churn never reached exhaustion — it tested the easy path only",
        );
        assert!(
            led.reused > 100,
            "only {} allocations came out of the free list; the reuse branch this test \
             exists to cover was barely reached",
            led.reused,
        );
        assert!(
            served > 8 * ARENA,
            "the churn served {served:#x} bytes out of a {ARENA:#x} arena — reclamation \
             was never actually load-bearing",
        );

        // ---- and it drains to genuinely pristine.
        while let Some(b) = held.pop() {
            let g = b.gpa.0;
            a.free(b).expect("its own block");
            led.give(&a, g, "final drain");
        }
        assert_eq!(a.live_bytes(), 0, "every byte came back");
        assert!(a.is_untouched(), "the drained arena is pristine again");
    }

    /// ★★ **A window refuses an arena that leaves it, even from a same-owner impostor.**
    ///
    /// [`GpaSpace::release`]'s guard is a three-way disjunction — wrong owner, start
    /// below the window, end above it — and its own doc comment says the containment
    /// halves exist to catch *"the same-owner impostor (two windows stamped alike) that
    /// owner alone would wave through"*. Nothing tested that: both existing G7 tests use
    /// **different** `GpuId`s, so the owner term alone decides them and either
    /// containment term can be broken without either test noticing.
    ///
    /// Asserted as the CONSEQUENCE, not as the refusal: a window only ever hands out
    /// ranges inside its own window. With the containment term gone, the recipient's
    /// free list swallows the foreign range and issues it to the very next proc that
    /// asks — a proc holding GPAs inside another window's aperture, while the donor
    /// leaks the range for good.
    #[test]
    fn g7_a_same_owner_window_still_refuses_an_arena_from_outside_it() {
        const LEN: u64 = 0x800_0000;
        // Two windows, stamped ALIKE (both default to `GpuId::ZERO`) and overlapping —
        // the nested/MIG-shaped geometry this public type permits.
        let home_w = 0x1000_0000..0x3000_0000;
        let recip_w = 0x1800_0000..0x3800_0000;
        let mut home = GpaSpace::new(home_w.clone(), LEN);
        let mut recip = GpaSpace::new(recip_w.clone(), LEN);
        assert_eq!(
            (home.owner(), recip.owner()),
            (GpuId::ZERO, GpuId::ZERO),
            "the impostor's premise: two windows that name the same owner",
        );

        // The recipient is already in use, so its cursor has moved — the state in which
        // a swallowed foreign range is issued rather than merely stored.
        let mine = recip.carve().expect("the recipient's own proc");
        assert_eq!(mine.range, 0x1800_0000..0x2000_0000);

        // A reaped proc's arena from the OTHER window, whose start is below the
        // recipient's window (and whose end is inside it — so ONLY the start term can
        // refuse it).
        let foreign = home.carve().expect("the donor's proc");
        let foreign_range = foreign.range.clone();
        assert_eq!(foreign_range, 0x1000_0000..0x1800_0000);
        assert!(
            foreign_range.start < recip_w.start && foreign_range.end <= recip_w.end,
            "the geometry the test needs: out of the window on exactly ONE side",
        );
        let sent = recip.release(foreign);

        // ★ THE PROPERTY: everything the recipient hands out is inside its own window.
        for _ in 0..3 {
            let got = recip.carve().expect("the recipient carves");
            assert!(
                got.range.start >= recip_w.start && got.range.end <= recip_w.end,
                "the recipient handed a proc {:?}, which is NOT inside its own window \
                 {recip_w:?} — that is the donor's cross-recycled arena {foreign_range:?}",
                got.range,
            );
        }

        // …and the refusal handed the arena back, so the donor can still recycle it.
        let refused = sent.expect_err("an arena from another window is not this one's");
        assert_eq!(refused.refused_by, GpuId::ZERO);
        assert_eq!(refused.arena.range, foreign_range);
        home.release(refused.arena)
            .expect("its own window recycles it");
        assert_eq!(
            home.carve().expect("the donor carves").range,
            foreign_range,
            "the #80 free list still works — a loud refusal must not cost the range",
        );
    }
}
