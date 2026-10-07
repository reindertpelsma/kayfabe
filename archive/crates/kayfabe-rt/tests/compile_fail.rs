//! ★★★★★ The single-writer rule's **type** half — `completion_observer.md` §8.4.
//!
//! §8 measured the writer census and found **zero** writers to the page `cuCtxCreate`
//! polls, and exactly **one** function in the whole tree that writes a completion semaphore
//! at all. §8.4 then said the part that matters: that property *"is currently **accidental**
//! — the release is a `ResolvedRelease` and never becomes a `CeSpan`, but that is asserted
//! nowhere and is one refactor from untrue."*
//!
//! ⊘ **Luck is not an argument, and here it is not even a safe one.** The C artifact's
//! `MC_SERVICE_INTERRUPTS` spin was read as a *missing* completion for two months and was a
//! **corrupted** one: a lagging second writer put stale payloads over a live value, UVM's
//! 32→64-bit wrap detector saw a backwards jump, the channel wedged. The fix, `ceb13f5`
//! (M5.38), was to **delete the second writer**. And it is fatal on FIRST occurrence, so
//! there is no run in which a second writer degrades gracefully first.
//!
//! ## What this suite proves, and what it does NOT
//!
//! It proves there is **no struct expression** for [`kayfabe_rt::cpu_ce::ResolvedRelease`]
//! outside `kayfabe-rt`: not by literal, not by functional update, not by `..`-elision. The
//! only way to obtain one is `resolve_releases`, which resolves every four-byte word of the
//! record through the guest's own page tables and refuses the whole record if any word
//! misses.
//!
//! ⊘ It does **not** prove there is no second writer. Rust's privacy unit is the crate, so
//! *"only `ceutils` may call this"* is not expressible; and — more importantly — a second
//! writer need never touch the type at all. `execute_ours` reaches the very same
//! `write_plane` primitive with a `CeSpan`, and a `CeSpan` carrying `CeSource::Constant(1)`
//! at a semaphore's `dst_place` is byte-for-byte a semaphore write with none of the
//! completion discipline (no resolve-all-first, no timestamp precondition, no
//! write-before-signal). **A type cannot see a caller that never names it.** That half is
//! `tests/tests/single_writer_census.rs`, and the two are complements rather than
//! alternatives — this is exactly the *"measure at the boundary, not inside"* pairing.
//!
//! ## Maintenance note
//!
//! `trybuild` compares full compiler stderr, so a rustc reword can turn this red with
//! nothing wrong. The fix is `TRYBUILD=overwrite cargo test -p kayfabe-rt --test
//! compile_fail` **after** confirming the error is still E0639 ("cannot create
//! non-exhaustive struct using struct expression"). ⊘ Never delete the row to make it green.

#[test]
fn a_completion_release_cannot_be_minted_outside_the_resolver() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
