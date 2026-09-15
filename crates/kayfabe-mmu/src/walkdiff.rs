//! ★★★★★ **THE HOST-SIDE DIFF — what changed between two walks of the guest's page tables.**
//!
//! The walk kernel (`cuda/walk/`) reports **the current mapping set**, not a delta
//! (`the_walk_kernel_report_format.md` §w723). This module turns two such sets into the
//! instruction list kayfabe acts on.
//!
//! # ⊘⊘⊘ WHY THIS IS IN RUST AND NOT IN THE KERNEL
//!
//! `[w722]` The diff was written in CUDA first, as a **merge join over whole runs**, which is the
//! obvious reading of *"linear, no hashing, no allocation"*. It **does not close**: for one new run
//! covering several old ones it emits a `Remap` followed by `Unmap`s of the runs it just replaced,
//! and **applying that in order deletes the mapping it just made**.
//!
//! ⚠ **All nine single-step delta tests passed against it.** Only an accumulating round-trip found
//! it. ⇒ Diff logic is subtle enough to get wrong, so it belongs beside the model, in a language
//! where a mistake is a panic rather than silent garbage.
//!
//! # ★★★ THE PROPERTY, and it is the one worth testing
//!
//! ```text
//!     apply(prev, diff(prev, cur)) == cur          for all prev, cur
//! ```
//!
//! ★ And **order-independence**: the [`MapOp::Unmap`] set and the [`MapOp::Map`]/[`MapOp::Remap`]
//! set are **disjoint in `(va, class)` by construction**, so applying the ops in any order gives
//! the same result. That is a property of the algorithm, not of the caller's discipline — which
//! is exactly what the whole-run version lacked.
//!
//! # The algorithm — per class, at SEGMENT granularity
//!
//! Runs of different page sizes can cover the same VA (a dual PDE's two halves both describe the
//! same 2 MiB), so a diff keyed on VA alone conflates them. ⇒ Group by **page-size class** first;
//! within a class the runs are disjoint and ascending, so the two sides can be walked together and
//! cut at every boundary either side introduces.
//!
//! For each resulting segment: covered by **prev only** ⇒ `Unmap`; **cur only** ⇒ `Map`; **both,
//! differing** ⇒ `Remap`; **both, identical** ⇒ nothing.

use std::collections::BTreeMap;

/// A contiguous run of pages mapping ascending VAs to ascending GPGAs with identical flags —
/// the unit the walk kernel reports, and the unit this module diffs.
///
/// ⊘ `len` is a whole multiple of the class's page size; a run is never partial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    /// Start virtual address, page-aligned.
    pub va: u64,
    /// Start guest physical (framebuffer) address the run maps to.
    pub gpga: u64,
    /// Length in bytes.
    pub len: u64,
    /// Decoded flags — aperture, read-only, atomic-disable, volatile. ⊘ **Not** the page size;
    /// that is [`Run::class`], because two runs differing only in page size are different
    /// mappings and must not be coalesced or compared as one.
    pub flags: u32,
    /// The page-size class. Runs are diffed **within** a class, never across.
    pub class: PageClass,
}

/// Which leaf size a run is made of. Distinct classes may cover the same VA simultaneously.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PageClass {
    /// 4 KiB.
    P4K,
    /// 64 KiB.
    P64K,
    /// 2 MiB.
    P2M,
    /// 512 MiB — Ampere and later.
    P512M,
}

impl PageClass {
    /// The class's page size in bytes.
    #[must_use]
    pub fn bytes(self) -> u64 {
        match self {
            PageClass::P4K => 4096,
            PageClass::P64K => 64 * 1024,
            PageClass::P2M => 2 * 1024 * 1024,
            PageClass::P512M => 512 * 1024 * 1024,
        }
    }
}

/// One instruction for the publisher.
///
/// ⊘ `Remap` is kept distinct from `Unmap` + `Map` deliberately: the host can re-point a binding
/// **without a window in which the VA is unmapped**, which is the difference between a re-point
/// and a transient fault the guest can observe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapOp {
    /// The guest now maps this range and did not before.
    Map(Run),
    /// The guest no longer maps this range. Only `va`, `len` and `class` are **binding**; `gpga`
    /// and `flags` carry what the range used to map to.
    ///
    /// ★ `[w741]` They are populated rather than zeroed for one reason: [`coalesce_ops`] joins two
    /// ops only when they are adjacent in VA **and** in GPGA, which is the criterion the whole
    /// module now uses. A zeroed `gpga` makes every unmap unmergeable with its own neighbour, so
    /// a shrink that drops two runs at once cost two `munmap`s where one does the job.
    Unmap(Run),
    /// Same range, different target or flags.
    Remap(Run),
}

impl MapOp {
    /// The run this op names.
    #[must_use]
    pub fn run(&self) -> &Run {
        match self {
            MapOp::Map(r) | MapOp::Unmap(r) | MapOp::Remap(r) => r,
        }
    }
}

/// The GPGA a run maps `va` to. ⊘ A run is linear, so this is arithmetic, not a lookup.
fn gpga_at(r: &Run, va: u64) -> u64 {
    r.gpga + (va - r.va)
}

/// ★★★ **Diff two mapping sets into an instruction list.**
///
/// `prev` and `cur` need not be sorted; they are grouped and sorted here. Within a class the
/// caller's runs must be **non-overlapping** — the walk produces them that way, and an overlap
/// would mean the guest's own tables described one VA twice in one class, which the walk refuses.
///
/// Returns ops whose `Unmap` set is disjoint from the `Map`/`Remap` set in `(va, class)`, so they
/// may be applied in **any** order.
#[must_use]
pub fn diff(prev: &[Run], cur: &[Run]) -> Vec<MapOp> {
    let mut by_class: BTreeMap<PageClass, (Vec<Run>, Vec<Run>)> = BTreeMap::new();
    for r in prev {
        by_class.entry(r.class).or_default().0.push(*r);
    }
    for r in cur {
        by_class.entry(r.class).or_default().1.push(*r);
    }
    let mut out = Vec::new();
    for (_class, (mut p, mut c)) in by_class {
        p.sort_by_key(|r| r.va);
        c.sort_by_key(|r| r.va);
        diff_one_class(&p, &c, &mut out);
    }
    coalesce_ops(out)
}

/// ★★★★★ **THE COALESCING PASS — the owner's criterion, applied to the OUTPUT.**
///
/// > *"If two BAR PTEs are adjacent, in GPGA and in BAR VA, then it is ONE consolidated mmap and
/// > not several … the goal is to minimise the amount of mmaps or VA-space maps of an RM object
/// > to its minimum while remaining correct."*
///
/// # ⊘⊘ WHY THIS EXISTS — a real defect, found by the minimality oracle `[w741]`
///
/// [`diff_one_class`] cuts at **every** boundary either side introduces, and most of those
/// boundaries are not changes. Two `prev` runs replaced by one `cur` run produce **two** `Remap`s
/// describing one contiguous re-point:
///
/// ```text
///   prev = [0x1000 → 0xA000, 4K] [0x2000 → 0xB000, 4K]      (two runs: targets not contiguous)
///   cur  = [0x1000 → 0x50000, 8K]                           (one run)
///   was  ⇒ Remap(0x1000→0x50000, 4K), Remap(0x2000→0x51000, 4K)     TWO mappings
///   now  ⇒ Remap(0x1000→0x50000, 8K)                                ONE
/// ```
///
/// ⚠ **Every test in this module passed through that**, because every one of them asserted the
/// mapping **set** and none asserted the op **count**. The walk kernel's `kf_seg_emit` has joined
/// these since it was written; this module — its host-side alternate — did not, and the two
/// disagreed on the number of host mappings a given guest change costs.
///
/// ⊘ Adjacent pairs are sufficient: within a class the ops leave [`diff_one_class`] ascending in
/// VA, so two mergeable ops can never have a third between them, and two ops of different classes
/// can never merge at all.
fn coalesce_ops(ops: Vec<MapOp>) -> Vec<MapOp> {
    let mut out: Vec<MapOp> = Vec::with_capacity(ops.len());
    for op in ops {
        if let Some(last) = out.last_mut()
            && mergeable(last, &op)
        {
            let add = op.run().len;
            match last {
                MapOp::Map(r) | MapOp::Unmap(r) | MapOp::Remap(r) => r.len += add,
            }
            continue;
        }
        out.push(op);
    }
    out
}

/// Could these two ops have been one? The same instruction, the same mapping identity, and
/// contiguous in **both** VA and GPGA. ⊘ Public to the crate's tests as the *oracle*: an op list
/// where this holds for any adjacent pair is correct and **not minimal**.
#[must_use]
pub fn mergeable(a: &MapOp, b: &MapOp) -> bool {
    let same_kind = matches!(
        (a, b),
        (MapOp::Map(_), MapOp::Map(_))
            | (MapOp::Unmap(_), MapOp::Unmap(_))
            | (MapOp::Remap(_), MapOp::Remap(_))
    );
    let (x, y) = (a.run(), b.run());
    same_kind
        && x.class == y.class
        && x.flags == y.flags
        && x.va + x.len == y.va
        && x.gpga + x.len == y.gpga
}

/// Walk two sorted, disjoint run lists together, cutting at every boundary either side introduces.
fn diff_one_class(prev: &[Run], cur: &[Run], out: &mut Vec<MapOp>) {
    // Every VA at which coverage or content can change on either side.
    let mut edges: Vec<u64> = Vec::with_capacity((prev.len() + cur.len()) * 2);
    for r in prev.iter().chain(cur.iter()) {
        edges.push(r.va);
        edges.push(r.va + r.len);
    }
    edges.sort_unstable();
    edges.dedup();

    for w in edges.windows(2) {
        let (lo, hi) = (w[0], w[1]);
        let p = covering(prev, lo);
        let c = covering(cur, lo);
        match (p, c) {
            (None, None) => {}
            (Some(pr), None) => {
                out.push(MapOp::Unmap(seg(lo, hi, gpga_at(pr, lo), pr.flags, prev, lo)));
            }
            (None, Some(r)) => out.push(MapOp::Map(seg(lo, hi, gpga_at(r, lo), r.flags, cur, lo))),
            (Some(pr), Some(cr)) => {
                // ⊘ Compare the DERIVED target at this segment's start, not the runs' own `gpga`:
                // two runs starting at different VAs can agree perfectly over their overlap.
                if gpga_at(pr, lo) != gpga_at(cr, lo) || pr.flags != cr.flags {
                    out.push(MapOp::Remap(seg(lo, hi, gpga_at(cr, lo), cr.flags, cur, lo)));
                }
            }
        }
    }
}

/// The run covering `va`, if any. Linear; the lists are short and this keeps the code obvious.
fn covering(runs: &[Run], va: u64) -> Option<&Run> {
    runs.iter().find(|r| va >= r.va && va < r.va + r.len)
}

/// Build the segment `[lo, hi)` carrying `gpga`/`flags`, taking the class from whichever side
/// supplied it.
fn seg(lo: u64, hi: u64, gpga: u64, flags: u32, from: &[Run], at: u64) -> Run {
    let class = covering(from, at).map_or(PageClass::P4K, |r| r.class);
    Run {
        va: lo,
        gpga,
        len: hi - lo,
        flags,
        class,
    }
}

/// Apply ops to a mapping set, for tests and for the publisher's own bookkeeping.
///
/// ★ The set is keyed by `(class, va)` at **page** granularity, which is what makes
/// order-independence checkable rather than merely asserted.
#[must_use]
pub fn apply(prev: &[Run], ops: &[MapOp]) -> Vec<Run> {
    let mut pages: BTreeMap<(PageClass, u64), (u64, u32)> = BTreeMap::new();
    for r in prev {
        explode(r, &mut pages);
    }
    for op in ops {
        match op {
            MapOp::Unmap(r) => {
                let ps = r.class.bytes();
                let mut va = r.va;
                while va < r.va + r.len {
                    pages.remove(&(r.class, va));
                    va += ps;
                }
            }
            MapOp::Map(r) | MapOp::Remap(r) => explode(r, &mut pages),
        }
    }
    coalesce(&pages)
}

fn explode(r: &Run, pages: &mut BTreeMap<(PageClass, u64), (u64, u32)>) {
    let ps = r.class.bytes();
    let mut va = r.va;
    while va < r.va + r.len {
        pages.insert((r.class, va), (gpga_at(r, va), r.flags));
        va += ps;
    }
}

/// Re-form runs from pages — consecutive VAs, consecutive GPGAs, identical flags.
fn coalesce(pages: &BTreeMap<(PageClass, u64), (u64, u32)>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for (&(class, va), &(gpga, flags)) in pages {
        let ps = class.bytes();
        match out.last_mut() {
            Some(last)
                if last.class == class
                    && last.flags == flags
                    && last.va + last.len == va
                    && last.gpga + last.len == gpga =>
            {
                last.len += ps;
            }
            _ => out.push(Run {
                va,
                gpga,
                len: ps,
                flags,
                class,
            }),
        }
    }
    out
}

/// Canonical form: exploded to pages and re-coalesced, so two descriptions of the same mapping
/// set compare equal regardless of how the walk happened to cut them into runs.
#[must_use]
pub fn canonical(runs: &[Run]) -> Vec<Run> {
    apply(runs, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(va: u64, gpga: u64, len: u64, flags: u32, class: PageClass) -> Run {
        Run { va, gpga, len, flags, class }
    }
    const K4: PageClass = PageClass::P4K;
    const K64: PageClass = PageClass::P64K;

    /// ★★★★★ **THE CLOSURE PROPERTY.** Everything else in this module is in service of it.
    fn assert_closes(prev: &[Run], cur: &[Run], what: &str) {
        let ops = diff(prev, cur);
        assert_eq!(
            canonical(&apply(prev, &ops)),
            canonical(cur),
            "{what}: applying the diff did not reproduce the new set\n  ops = {ops:#x?}"
        );
        // ★ Order-independence: the Unmap set and the Map/Remap set are disjoint by
        // construction, so a reversed application must give the same answer.
        let mut rev = ops.clone();
        rev.reverse();
        assert_eq!(
            canonical(&apply(prev, &rev)),
            canonical(cur),
            "{what}: the diff is ORDER-DEPENDENT, which the algorithm forbids"
        );
    }

    /// ⊘⊘⊘ **THE w722 BUG, as a regression test.** One new run covering several old ones. The
    /// whole-run merge join emitted `Remap` then `Unmap` of the runs it had just replaced, and
    /// applying that in order deleted the mapping it had just made. ⚠ Every single-step test
    /// passed against that version; only coverage-changing shapes like this one catch it.
    #[test]
    fn one_new_run_covering_several_old_ones_still_closes() {
        let prev = vec![
            r(0x1000, 0xA000, 0x1000, 0, K4),
            r(0x2000, 0xB000, 0x1000, 0, K4),
            r(0x3000, 0xC000, 0x1000, 0, K4),
        ];
        // One contiguous run over all three, at a different target.
        let cur = vec![r(0x1000, 0x50000, 0x3000, 0, K4)];
        assert_closes(&prev, &cur, "one run replacing three");
    }

    /// The mirror image: one old run becoming several new ones.
    #[test]
    fn one_old_run_splitting_into_several_still_closes() {
        let prev = vec![r(0x1000, 0x50000, 0x3000, 0, K4)];
        let cur = vec![
            r(0x1000, 0xA000, 0x1000, 0, K4),
            r(0x2000, 0xB000, 0x1000, 1, K4),
            r(0x3000, 0xC000, 0x1000, 0, K4),
        ];
        assert_closes(&prev, &cur, "three runs replacing one");
    }

    /// ⊘ **Two classes may cover the same VA** — a dual PDE's halves both describe one 2 MiB
    /// region. A diff keyed on VA alone conflates them; this asserts they stay separate.
    #[test]
    fn two_page_size_classes_over_one_va_are_not_conflated() {
        let prev = vec![r(0x20_0000, 0xA0_0000, 0x1000, 0, K4)];
        let cur = vec![
            r(0x20_0000, 0xA0_0000, 0x1000, 0, K4),
            r(0x20_0000, 0xF0_0000, 0x1_0000, 0, K64),
        ];
        let ops = diff(&prev, &cur);
        assert!(
            ops.iter().all(|o| !matches!(o, MapOp::Unmap(_))),
            "adding a 64K mapping must not UNMAP the 4K one at the same VA: {ops:#x?}"
        );
        assert_closes(&prev, &cur, "two classes at one VA");
    }

    /// Extending a run must `Map` only the new pages, and shrinking must `Unmap` only the tail —
    /// not `Remap` the whole extent.
    #[test]
    fn extending_and_shrinking_touch_only_the_difference() {
        let base = vec![r(0x1000, 0xA000, 0x2000, 0, K4)];
        let longer = vec![r(0x1000, 0xA000, 0x4000, 0, K4)];
        let ops = diff(&base, &longer);
        assert_eq!(ops.len(), 1, "extending should be one op: {ops:#x?}");
        assert!(matches!(ops[0], MapOp::Map(_)), "extending should MAP the tail");
        assert_eq!(ops[0].run().va, 0x3000, "and only the new pages");
        assert_closes(&base, &longer, "extend");
        assert_closes(&longer, &base, "shrink");
    }

    /// A deterministic pseudo-random mutation sequence, asserting closure at every step.
    /// ⊘ The vacuity guard matters: a sequence that never changes anything closes trivially.
    #[test]
    fn a_long_mutation_sequence_closes_at_every_step() {
        let mut seed = 0x243f_6a88_85a3_08d3u64;
        let mut rng = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut cur: Vec<Run> = Vec::new();
        let (mut changed, mut nonempty) = (0usize, 0usize);
        for step in 0..400 {
            let prev = cur.clone();
            let n = (rng() % 6) as usize;
            cur = (0..n)
                .map(|i| {
                    let class = if rng() % 3 == 0 { K64 } else { K4 };
                    let ps = class.bytes();
                    r(
                        (i as u64 + 1) * 0x10_0000 + (rng() % 4) * ps,
                        (rng() % 64) * ps,
                        ps * (1 + rng() % 3),
                        (rng() % 4) as u32,
                        class,
                    )
                })
                .collect();
            // ⊘ Drop overlaps within a class — the walk cannot produce them, so neither may this.
            cur.sort_by_key(|x| (x.class, x.va));
            cur.dedup_by(|b, a| a.class == b.class && b.va < a.va + a.len);
            if canonical(&prev) != canonical(&cur) {
                changed += 1;
            }
            if !diff(&prev, &cur).is_empty() {
                nonempty += 1;
            }
            assert_closes(&prev, &cur, &format!("step {step}"));
        }
        assert!(changed > 200, "vacuous: only {changed} steps changed the set");
        assert!(nonempty > 200, "vacuous: only {nonempty} steps produced ops");
    }

    // ══ COALESCING — THE OWNER'S CRITERION ═══════════════════════════════════════════════════
    //
    // ★★★★★ *"If two BAR PTEs are adjacent, in GPGA and in BAR VA, then it is ONE consolidated
    // mmap and not several … the goal is to minimise the amount of mmaps or VA-space maps of an
    // RM object to its minimum while remaining correct."*
    //
    // ⊘ Closure alone is not the bar, and nothing above noticed: an op list can reproduce `cur`
    // exactly and still cost twice the `mmap`s. Every case below asserts BOTH.

    /// ★★★ **THE MINIMALITY ORACLE.** No two emitted ops could have been one.
    ///
    /// ⊘ It reads the OUTPUT, so it is blind to *why* a join was lost — a missing coalescing pass,
    /// a boundary the differ cut and never re-joined, a flags field that stopped comparing equal.
    fn assert_minimal(ops: &[MapOp], what: &str) {
        for w in ops.windows(2) {
            assert!(
                !mergeable(&w[0], &w[1]),
                "{what}: two ops that are ONE mapping spelled as TWO:\n  {:#x?}\n  {:#x?}",
                w[0],
                w[1]
            );
        }
    }

    /// The fewest ops this delta can be spelled in: explode both sides to pages, classify every
    /// page, re-coalesce. ⊘ Built from the RULE, not from [`diff`] — a reference that called the
    /// implementation would agree by construction and measure nothing.
    fn ref_ops(prev: &[Run], cur: &[Run]) -> Vec<MapOp> {
        let (mut p, mut c) = (BTreeMap::new(), BTreeMap::new());
        for r in prev {
            explode(r, &mut p);
        }
        for r in cur {
            explode(r, &mut c);
        }
        let mut keys: Vec<(PageClass, u64)> = p.keys().chain(c.keys()).copied().collect();
        keys.sort();
        keys.dedup();
        let mut out = Vec::new();
        for k in keys {
            let (class, va) = k;
            let len = class.bytes();
            let op = match (p.get(&k), c.get(&k)) {
                (Some(&(gpga, flags)), None) => Some(MapOp::Unmap(Run { va, gpga, len, flags, class })),
                (None, Some(&(gpga, flags))) => Some(MapOp::Map(Run { va, gpga, len, flags, class })),
                (Some(a), Some(b)) if a != b => {
                    Some(MapOp::Remap(Run { va, gpga: b.0, len, flags: b.1, class }))
                }
                _ => None,
            };
            if let Some(o) = op {
                out.push(o);
            }
        }
        coalesce_ops(out)
    }

    /// Closure, order-independence, minimality, and the op budget — the whole bar, in one call.
    fn assert_good(prev: &[Run], cur: &[Run], what: &str) -> Vec<MapOp> {
        assert_closes(prev, cur, what);
        let ops = diff(prev, cur);
        assert_minimal(&ops, what);
        let want = ref_ops(prev, cur);
        assert!(
            ops.len() <= want.len(),
            "{what}: {} ops where {} suffice\n  got  = {ops:#x?}\n  want = {want:#x?}",
            ops.len(),
            want.len()
        );
        ops
    }

    /// ⊘⊘ **THE DEFECT THIS PASS WAS ADDED FOR, `[w741]`.** One `cur` run replacing two `prev`
    /// runs is ONE contiguous re-point; the segment differ cut it at the old boundary and never
    /// re-joined it. Closure held, so nothing in this module saw it.
    #[test]
    fn one_run_replacing_two_is_one_remap_not_two() {
        let prev = vec![
            r(0x1000, 0xA000, 0x1000, 0, K4),
            r(0x2000, 0xB000, 0x1000, 0, K4), // targets not contiguous ⇒ genuinely two runs
        ];
        let cur = vec![r(0x1000, 0x50000, 0x2000, 0, K4)];
        let ops = assert_good(&prev, &cur, "one run replacing two");
        assert_eq!(ops.len(), 1, "one contiguous re-point is ONE mmap: {ops:#x?}");
        assert!(matches!(ops[0], MapOp::Remap(_)));
        assert_eq!(ops[0].run().len, 0x2000);
    }

    /// ★★★ **THE KNOWN-POSITIVE for the minimality oracle.** The pre-`w741` output — the same
    /// segments, without [`coalesce_ops`] — must FAIL the oracle. Without this the oracle could
    /// be vacuous and every case above would still be green.
    #[test]
    fn the_uncoalesced_diff_is_shown_to_be_non_minimal() {
        let prev = vec![
            r(0x1000, 0xA000, 0x1000, 0, K4),
            r(0x2000, 0xB000, 0x1000, 0, K4),
        ];
        let cur = vec![r(0x1000, 0x50000, 0x2000, 0, K4)];
        let mut raw = Vec::new();
        diff_one_class(&prev, &cur, &mut raw);
        assert_eq!(raw.len(), 2, "the pre-w741 shape is two segments");
        assert!(
            raw.windows(2).any(|w| mergeable(&w[0], &w[1])),
            "the known-positive did not fire: the un-coalesced diff must be non-minimal, or the \
             oracle is not testing anything"
        );
        // And it still CLOSES — which is exactly why no correctness oracle could ever see it.
        assert_eq!(canonical(&apply(&prev, &raw)), canonical(&cur));
    }

    #[test]
    fn enlarge_at_end_maps_only_the_new_tail() {
        let base = vec![r(0x1000, 0xA000, 0x2000, 0, K4)];
        let grown = vec![r(0x1000, 0xA000, 0x4000, 0, K4)];
        let ops = assert_good(&base, &grown, "enlarge at end");
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], MapOp::Map(_)));
        assert_eq!((ops[0].run().va, ops[0].run().len), (0x3000, 0x2000));
    }

    /// ★ Not the mirror of the case above in the code: growing DOWNWARDS makes the new pages the
    /// head of the run, and the target must extend downwards with them.
    #[test]
    fn enlarge_at_start_maps_only_the_new_head() {
        let base = vec![r(0x3000, 0xC000, 0x2000, 0, K4)];
        let grown = vec![r(0x1000, 0xA000, 0x4000, 0, K4)];
        let ops = assert_good(&base, &grown, "enlarge at start");
        assert_eq!(ops.len(), 1, "one map of the new head: {ops:#x?}");
        assert!(matches!(ops[0], MapOp::Map(_)));
        assert_eq!((ops[0].run().va, ops[0].run().len), (0x1000, 0x2000));
    }

    #[test]
    fn shrink_at_end_unmaps_only_the_lost_tail() {
        let base = vec![r(0x1000, 0xA000, 0x4000, 0, K4)];
        let small = vec![r(0x1000, 0xA000, 0x2000, 0, K4)];
        let ops = assert_good(&base, &small, "shrink at end");
        assert_eq!(ops.len(), 1, "one unmap, not one per page: {ops:#x?}");
        assert!(matches!(ops[0], MapOp::Unmap(_)));
        assert_eq!((ops[0].run().va, ops[0].run().len), (0x3000, 0x2000));
    }

    #[test]
    fn shrink_at_start_unmaps_only_the_lost_head() {
        let base = vec![r(0x1000, 0xA000, 0x4000, 0, K4)];
        let small = vec![r(0x3000, 0xC000, 0x2000, 0, K4)];
        let ops = assert_good(&base, &small, "shrink at start");
        assert_eq!(ops.len(), 1, "one unmap, not one per page: {ops:#x?}");
        assert!(matches!(ops[0], MapOp::Unmap(_)));
        assert_eq!((ops[0].run().va, ops[0].run().len), (0x1000, 0x2000));
    }

    #[test]
    fn dropping_a_whole_run_leaves_its_neighbour_untouched() {
        let prev = vec![
            r(0x1000, 0xA000, 0x3000, 0, K4),
            r(0x10_000, 0x9_0000, 0x3000, 0, K4),
        ];
        let cur = vec![r(0x1000, 0xA000, 0x3000, 0, K4)];
        let ops = assert_good(&prev, &cur, "drop a whole run");
        assert_eq!(ops.len(), 1, "one unmap; the survivor is not re-pointed: {ops:#x?}");
        assert!(matches!(ops[0], MapOp::Unmap(_)));
        assert_eq!((ops[0].run().va, ops[0].run().len), (0x10_000, 0x3000));
    }

    #[test]
    fn adding_a_run_between_two_others_touches_neither() {
        let prev = vec![
            r(0x1000, 0xA000, 0x2000, 0, K4),
            r(0x10_000, 0x9_0000, 0x2000, 0, K4),
        ];
        let mut cur = prev.clone();
        cur.push(r(0x8000, 0x7_0000, 0x2000, 0, K4));
        let ops = assert_good(&prev, &cur, "add between runs");
        assert_eq!(ops.len(), 1, "one map: {ops:#x?}");
        assert!(matches!(ops[0], MapOp::Map(_)));
        assert_eq!((ops[0].run().va, ops[0].run().len), (0x8000, 0x2000));
    }

    /// ★★★★★ **THE CASE THE OWNER NAMED.** Two runs with a hole between them whose targets
    /// already line up across it. Filling the hole must leave ONE mapping — and the delta is just
    /// the hole, because the host extends what it has rather than re-pointing anything.
    #[test]
    fn filling_a_hole_that_makes_two_runs_adjacent_yields_one_mapping() {
        let prev = vec![
            r(0x1000, 0xA000, 0x2000, 0, K4),
            r(0x4000, 0xD000, 0x2000, 0, K4), // 0xA000+0x3000 == 0xD000: already collinear
        ];
        let mut cur = prev.clone();
        cur.push(r(0x3000, 0xC000, 0x1000, 0, K4));
        let ops = assert_good(&prev, &cur, "fill the hole");
        assert_eq!(ops.len(), 1, "the delta is just the hole: {ops:#x?}");
        assert!(matches!(ops[0], MapOp::Map(_)));
        assert_eq!((ops[0].run().va, ops[0].run().len), (0x3000, 0x1000));
        // ⊘ And the STATE the host now holds is ONE run, not three that add up.
        let after = canonical(&apply(&prev, &ops));
        assert_eq!(after.len(), 1, "filling the hole must leave one mapping: {after:#x?}");
        assert_eq!((after[0].va, after[0].len, after[0].gpga), (0x1000, 0x5000, 0xA000));
    }

    #[test]
    fn punching_a_hole_splits_the_run_and_unmaps_only_the_hole() {
        let prev = vec![r(0x1000, 0xA000, 0x5000, 0, K4)];
        let cur = vec![
            r(0x1000, 0xA000, 0x2000, 0, K4),
            r(0x4000, 0xD000, 0x2000, 0, K4),
        ];
        let ops = assert_good(&prev, &cur, "split in the middle");
        assert_eq!(ops.len(), 1, "one unmap of the hole: {ops:#x?}");
        assert!(matches!(ops[0], MapOp::Unmap(_)));
        assert_eq!((ops[0].run().va, ops[0].run().len), (0x3000, 0x1000));
        assert_eq!(canonical(&apply(&prev, &ops)).len(), 2, "the run must now be two");
    }

    // ══ THE COMBINATORIAL STRESS ═════════════════════════════════════════════════════════════

    /// One page-indexed mapping state per class — the shape the mutations act on.
    #[derive(Clone, PartialEq, Eq)]
    struct St {
        pg: Vec<Vec<Option<(u64, u32)>>>, // [class][page index]
    }
    const SN: usize = 192;
    const SCLASS: [PageClass; 2] = [PageClass::P4K, PageClass::P64K];
    const SBASE: u64 = 0x1_0000_0000;

    impl St {
        fn new() -> St {
            St { pg: vec![vec![None; SN]; SCLASS.len()] }
        }
        fn runs(&self) -> Vec<Run> {
            let mut out: Vec<Run> = Vec::new();
            for (ci, class) in SCLASS.iter().copied().enumerate() {
                let ps = class.bytes();
                let mut open: Option<Run> = None;
                for i in 0..SN {
                    match self.pg[ci][i] {
                        None => {
                            if let Some(rr) = open.take() {
                                out.push(rr);
                            }
                        }
                        Some((gpga, flags)) => {
                            let va = SBASE + (i as u64) * ps;
                            match open.as_mut() {
                                Some(rr)
                                    if rr.flags == flags
                                        && rr.va + rr.len == va
                                        && rr.gpga + rr.len == gpga =>
                                {
                                    rr.len += ps;
                                }
                                _ => {
                                    if let Some(rr) = open.take() {
                                        out.push(rr);
                                    }
                                    open = Some(Run { va, gpga, len: ps, flags, class });
                                }
                            }
                        }
                    }
                }
                if let Some(rr) = open.take() {
                    out.push(rr);
                }
            }
            out
        }
        /// Maximal run extents as page-index pairs, per class — what the mutations steer by.
        fn extents(&self, ci: usize) -> Vec<(usize, usize)> {
            let ps = SCLASS[ci].bytes();
            let mut out: Vec<(usize, usize)> = Vec::new();
            for i in 0..SN {
                let Some((g, f)) = self.pg[ci][i] else { continue };
                let cont = i > 0
                    && out.last().is_some_and(|&(_, hi)| hi == i)
                    && self.pg[ci][i - 1].is_some_and(|(pg, pf)| pg + ps == g && pf == f);
                if cont {
                    out.last_mut().unwrap().1 = i + 1;
                } else {
                    out.push((i, i + 1));
                }
            }
            out
        }
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % (n as u64)) as usize
        }
    }

    const MUT: [&str; 10] = [
        "add", "drop", "repoint", "reflag", "grow_end", "grow_start", "shrink_end",
        "shrink_start", "split", "merge",
    ];

    /// Apply one mutation; `false` when the state offered no instance of it, which the vacuity
    /// guard turns into a NAMED failure rather than a silent gap.
    fn mutate(s: &mut St, r: &mut Rng, kind: usize) -> bool {
        let ci = r.below(SCLASS.len());
        let ps = SCLASS[ci].bytes();
        let ex = s.extents(ci);
        let pick = |r: &mut Rng| -> Option<(usize, usize)> {
            if ex.is_empty() { None } else { Some(ex[r.below(ex.len())]) }
        };
        match kind {
            0 => {
                let lo = r.below(SN - 1);
                let len = 1 + r.below(12).min(SN - lo - 1);
                let base = (1 + r.below(64) as u64) * ps * 16;
                let fl = r.below(4) as u32;
                for i in lo..lo + len {
                    s.pg[ci][i] = Some((base + (i - lo) as u64 * ps, fl));
                }
                true
            }
            1 => {
                let Some((lo, hi)) = pick(r) else { return false };
                for i in lo..hi {
                    s.pg[ci][i] = None;
                }
                true
            }
            2 => {
                let Some((lo, hi)) = pick(r) else { return false };
                let off = r.below(hi - lo);
                let n = 1 + r.below(hi - lo - off);
                let d = (1 + r.below(32) as u64) * ps;
                for i in lo + off..lo + off + n {
                    s.pg[ci][i] = s.pg[ci][i].map(|(g, f)| (g + d, f));
                }
                true
            }
            3 => {
                let Some((lo, hi)) = pick(r) else { return false };
                let off = r.below(hi - lo);
                let n = 1 + r.below(hi - lo - off);
                let flip = 1u32 + r.below(3) as u32; // never a no-op
                for i in lo + off..lo + off + n {
                    s.pg[ci][i] = s.pg[ci][i].map(|(g, f)| (g, f ^ flip));
                }
                true
            }
            4 | 5 => {
                let start = if ex.is_empty() { return false } else { r.below(ex.len()) };
                for d in 0..ex.len() {
                    let (lo, hi) = ex[(start + d) % ex.len()];
                    let n = 1 + r.below(3);
                    if kind == 4 {
                        if hi + n > SN || (hi..hi + n).any(|i| s.pg[ci][i].is_some()) {
                            continue;
                        }
                        let (g, f) = s.pg[ci][hi - 1].unwrap();
                        for i in hi..hi + n {
                            s.pg[ci][i] = Some((g + (i - hi + 1) as u64 * ps, f));
                        }
                    } else {
                        if lo < n || (lo - n..lo).any(|i| s.pg[ci][i].is_some()) {
                            continue;
                        }
                        let (g, f) = s.pg[ci][lo].unwrap();
                        if g < (lo - (lo - n)) as u64 * ps {
                            continue;
                        }
                        for i in lo - n..lo {
                            s.pg[ci][i] = Some((g - (lo - i) as u64 * ps, f));
                        }
                    }
                    return true;
                }
                false
            }
            6 | 7 => {
                let start = if ex.is_empty() { return false } else { r.below(ex.len()) };
                for d in 0..ex.len() {
                    let (lo, hi) = ex[(start + d) % ex.len()];
                    if hi - lo < 2 {
                        continue;
                    }
                    let n = 1 + r.below(hi - lo - 1);
                    let rng = if kind == 6 { hi - n..hi } else { lo..lo + n };
                    for i in rng {
                        s.pg[ci][i] = None;
                    }
                    return true;
                }
                false
            }
            8 => {
                let start = if ex.is_empty() { return false } else { r.below(ex.len()) };
                for d in 0..ex.len() {
                    let (lo, hi) = ex[(start + d) % ex.len()];
                    if hi - lo < 3 {
                        continue;
                    }
                    s.pg[ci][lo + 1 + r.below(hi - lo - 2)] = None;
                    return true;
                }
                false
            }
            // ★★★ The owner's case: growth that makes two runs become EXACTLY adjacent.
            _ => {
                if ex.len() < 2 {
                    return false;
                }
                let start = r.below(ex.len() - 1);
                for d in 0..ex.len() - 1 {
                    let k = (start + d) % (ex.len() - 1);
                    let ((_, ahi), (blo, bhi)) = (ex[k], ex[k + 1]);
                    if blo <= ahi || blo - ahi > 16 {
                        continue;
                    }
                    let (g, f) = s.pg[ci][ahi - 1].unwrap();
                    for i in ahi..bhi {
                        s.pg[ci][i] = Some((g + (i - ahi + 1) as u64 * ps, f));
                    }
                    return true;
                }
                false
            }
        }
    }

    /// ★★★★★ **SHRINK + ENLARGE + DROP + ADD + SPLIT + MERGE, ALL IN ONE STEP.**
    ///
    /// ⊘ Every case above changes ONE thing — the blind spot the w722 bug lived in, where nine
    /// single-step tests passed against a diff that lost mappings. This generates new instances of
    /// that shape instead of pinning the one that was found by hand.
    ///
    /// Three oracles: closure + order-independence (the mapping set is right), the op budget
    /// (a page-wise reference delta, re-coalesced — the kernel may not exceed it), and the
    /// minimality oracle (no two ops could have been one).
    ///
    /// ⚠ Seeded, and the seed is in every failure message.
    #[test]
    fn a_combinatorial_mutation_stream_is_correct_and_minimal() {
        let seed = 0x243f_6a88_85a3_08d3u64;
        let mut rng = Rng(seed);
        let mut st = St::new();
        for _ in 0..4 {
            mutate(&mut st, &mut rng, 0);
        }
        let mut fired = [0usize; 10];
        let (mut nonempty, mut merges, mut splits) = (0usize, 0usize, 0usize);
        let (mut maps, mut unmaps, mut remaps) = (0usize, 0usize, 0usize);

        for step in 0..500 {
            let prev_st = st.clone();
            let before = st.runs().len();
            let n = 3 + rng.below(6);
            for _ in 0..n {
                let k = rng.below(MUT.len());
                if mutate(&mut st, &mut rng, k) {
                    fired[k] += 1;
                }
            }
            let (prev, cur) = (prev_st.runs(), st.runs());
            let after = cur.len();
            let pages = |s: &St| s.pg.iter().flatten().filter(|p| p.is_some()).count();
            if after < before && pages(&st) > pages(&prev_st) {
                merges += 1;
            }
            if after > before && pages(&st) < pages(&prev_st) {
                splits += 1;
            }

            let what = format!("seed=0x{seed:x} step={step}");
            let ops = assert_good(&prev, &cur, &what);
            if !ops.is_empty() {
                nonempty += 1;
            }
            for o in &ops {
                match o {
                    MapOp::Map(_) => maps += 1,
                    MapOp::Unmap(_) => unmaps += 1,
                    MapOp::Remap(_) => remaps += 1,
                }
            }
            // ★ The state the host now holds must itself be minimal: a minimal delta applied to a
            // fragmented model still leaves a fragmented model.
            let held = apply(&prev, &ops);
            assert_eq!(held, canonical(&cur), "{what}: the held state is not the canonical one");
        }

        // ⊘ THE VACUITY GUARDS. A stress that never generated a merge proves nothing about
        // merging and would look exactly as green as one that did.
        for (k, name) in MUT.iter().enumerate() {
            assert!(fired[k] > 0, "mutation '{name}' NEVER fired: the stress does not cover it");
        }
        assert!(nonempty > 250, "vacuous: only {nonempty} steps produced any op");
        assert!(maps > 0 && unmaps > 0 && remaps > 0, "vacuous: {maps}/{unmaps}/{remaps}");
        assert!(merges > 0, "vacuous: no step ever made two runs become one");
        assert!(splits > 0, "vacuous: no step ever split a run");
    }

    /// ★★★ **THE KNOWN-POSITIVE.** A whole-run merge join — the w722 implementation — must FAIL
    /// closure on the shape above. Without this, a green suite would not prove the property is
    /// being tested at all.
    #[test]
    fn the_superseded_whole_run_merge_is_shown_to_break_closure() {
        // The bug, reproduced exactly: pair runs by start VA; emit Remap for a changed pair and
        // Unmap for every prev run the new one swallowed.
        fn broken(prev: &[Run], cur: &[Run]) -> Vec<MapOp> {
            let mut out = Vec::new();
            for c in cur {
                out.push(MapOp::Remap(*c));
            }
            for p in prev {
                if !cur.iter().any(|c| c.va == p.va && c.len == p.len) {
                    out.push(MapOp::Unmap(*p));
                }
            }
            out
        }
        let prev = vec![
            r(0x1000, 0xA000, 0x1000, 0, K4),
            r(0x2000, 0xB000, 0x1000, 0, K4),
            r(0x3000, 0xC000, 0x1000, 0, K4),
        ];
        let cur = vec![r(0x1000, 0x50000, 0x3000, 0, K4)];
        let got = canonical(&apply(&prev, &broken(&prev, &cur)));
        assert_ne!(
            got,
            canonical(&cur),
            "the known-positive did not fire: the whole-run merge must LOSE mappings here, or \
             this suite is not testing closure at all"
        );
        // And the real one closes on the same input.
        assert_closes(&prev, &cur, "known-positive control");
    }
}
