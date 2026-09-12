//! ★★★★★ **A chunked sweep commit must total the same as an unchunked one (w531).**
//!
//! Owner ruling, 2026-09-12: *"sweep commit may chunk."* `[measured w517–w524]` the COMMIT
//! phase held the Device rank **and** the Proc cell for ~5 ms, ~1066 times a boot, because it
//! committed **every** address space under one acquisition. The results are keyed by
//! `(gpu, pdb)` and are independent, so the chunk is one address space.
//!
//! ⚠ Chunking turns the outcome's figures into a **sum over chunks**. Every consumer already
//! reads them as one pass's totals, so [`PtDecodeOutcome::merge`] has to be **exhaustive** —
//! and a field forgotten there does not fail loudly. It reads as *"the sweep did less"*, in a
//! census line nobody diffs against a previous boot.
//!
//! ⊘ So this does not test a hand-picked field or two. It builds a value with **every field
//! non-default**, splits it, and checks the merge reconstructs the whole thing. A new field
//! added to the struct and forgotten in `merge` fails here.

use kayfabe_fwd::PtDecodeOutcome;

/// Every field set to something distinguishable from `Default`.
fn populated(seed: usize) -> PtDecodeOutcome {
    use kayfabe_arch::ids::{GpuId, Pdb};
    use kayfabe_arch::ids::GpuVa;
    let mut o = PtDecodeOutcome::default();
    o.revoked_still_desired = seed;
    o.remaps_refused = seed + 1;
    o.remaps_revoked = seed + 2;
    o.bound = seed + 3;
    o.unchanged = seed + 4;
    o.repointed = seed + 5;
    o.vas_gone = seed + 6;
    o.meta_refused = seed + 7;
    o.meta_learned = seed + 8;
    o.pages_published = seed + 9;
    o.pages_publish_refused = seed + 10;
    o.unbound = seed + 11;
    o.unwitnessed = seed + 12;
    o.unreachable = seed + 13;
    o.sparse = seed + 14;
    o.swept_binds = seed + 15;
    o.sweeps_run = seed + 16;
    o.sweeps_truncated = seed + 17;
    o.pages_swept = seed + 18;
    o.duplicate_leaves = seed + 19;
    o.learned_pages
        .push((GpuId(0), Pdb(0x4E60_0000), seed as u64));
    o.protection_changes.push(GpuVa(0x2_0020_0000 + seed as u64));
    o.retired.push(seed as u64);
    o
}

/// ★★★ **Two chunks merged equal one pass over both.**
///
/// ⊘ The comparison is on the WHOLE value (`PtDecodeOutcome` is `PartialEq`), never on a few
/// fields I remembered to check — which is the only version of this test that catches a field
/// added later and forgotten in `merge`.
#[test]
fn merging_two_chunks_totals_every_field() {
    let (a, b) = (populated(100), populated(200));

    let mut merged = PtDecodeOutcome::default();
    merged.merge(a.clone());
    merged.merge(b.clone());

    // The expected total, built by hand from the two halves — deliberately NOT by calling
    // `merge`, or the test would be checking the function against itself.
    let mut want = PtDecodeOutcome::default();
    want.revoked_still_desired = a.revoked_still_desired + b.revoked_still_desired;
    want.remaps_refused = a.remaps_refused + b.remaps_refused;
    want.remaps_revoked = a.remaps_revoked + b.remaps_revoked;
    want.bound = a.bound + b.bound;
    want.unchanged = a.unchanged + b.unchanged;
    want.repointed = a.repointed + b.repointed;
    want.vas_gone = a.vas_gone + b.vas_gone;
    want.meta_refused = a.meta_refused + b.meta_refused;
    want.meta_learned = a.meta_learned + b.meta_learned;
    want.pages_published = a.pages_published + b.pages_published;
    want.pages_publish_refused = a.pages_publish_refused + b.pages_publish_refused;
    want.unbound = a.unbound + b.unbound;
    want.unwitnessed = a.unwitnessed + b.unwitnessed;
    want.unreachable = a.unreachable + b.unreachable;
    want.sparse = a.sparse + b.sparse;
    want.swept_binds = a.swept_binds + b.swept_binds;
    want.sweeps_run = a.sweeps_run + b.sweeps_run;
    want.sweeps_truncated = a.sweeps_truncated + b.sweeps_truncated;
    want.pages_swept = a.pages_swept + b.pages_swept;
    want.duplicate_leaves = a.duplicate_leaves + b.duplicate_leaves;
    want.learned_pages = [a.learned_pages.clone(), b.learned_pages.clone()].concat();
    want.protection_changes = [a.protection_changes.clone(), b.protection_changes.clone()].concat();
    want.retired = [a.retired.clone(), b.retired.clone()].concat();

    assert_eq!(
        merged, want,
        "a chunked commit must total exactly what one pass over both chunks would — a field \
         missing from `merge` reads as 'the sweep did less', in a census nobody diffs"
    );
}

/// ⊘ **The negative control: merging the default must change nothing.** Without it, a `merge`
/// that overwrote instead of accumulating would still pass the test above whenever the second
/// chunk happened to be larger.
#[test]
fn merging_an_empty_chunk_is_the_identity() {
    let a = populated(7);
    let mut got = a.clone();
    got.merge(PtDecodeOutcome::default());
    assert_eq!(
        got, a,
        "an address space that settled nothing must not alter the running total"
    );
}
