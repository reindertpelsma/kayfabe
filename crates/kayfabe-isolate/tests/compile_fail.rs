//! ★★ The #14 ring-gate's compile-fail row — `ARCHITECTURE.md` invariant 5.
//!
//! The invariant claims *"a caller cannot choose an ungated door because none exists"*.
//! That was a statement about the production **call graph**, not about the types:
//! `VerbPlan` is a public enum and `Worker::execute` is a public method, so any crate
//! could hand-build a `VerbPlan::Doorbell` and hand it to a checked-out worker — whose
//! own gate is the foreign-handle check, **not** the #14 working-set check, which lived
//! entirely in `kayfabe_fwd::plan_doorbell`. `tests/tests/cross_proc_lifetime.rs` went
//! through that door, so it was reachable rather than hypothetical.
//!
//! The variant is now `#[non_exhaustive]` and the only constructor is
//! `VerbPlan::gated_doorbell`, which runs the gate. This suite is the evidence: the row
//! below **must not compile**. An impossibility that is only asserted in a doc comment
//! is exactly the green-instrument-on-an-unexercised-path failure this project keeps
//! cataloguing, one level up — nobody notices when it stops being true.
//!
//! ## What it proves, and what it does not
//!
//! It proves there is **no struct expression** for the variant outside this crate: not
//! by literal, not by functional-update, not by `..`-elided literal. It does **not**
//! prove that only `kayfabe-fwd` may call `gated_doorbell` — Rust's privacy unit is the
//! crate, so "only that crate may call this" is not expressible, and pretending
//! otherwise would be the same over-claim the invariant just had corrected. What the
//! constructor *does* enforce is that the gate RUNS, over whatever address plane the
//! caller supplies, for every VA the submission claims. The residual is that the
//! address plane itself is caller-supplied — a lie there is fabricating core state, and
//! it is commission rather than omission.
//!
//! ## Maintenance note
//!
//! `trybuild` compares full compiler stderr, so a rustc diagnostic reword can turn this
//! red without anything being wrong. The fix is `TRYBUILD=overwrite cargo test -p
//! kayfabe-isolate --test compile_fail` **after** confirming the error is still E0639
//! ("cannot create non-exhaustive variant using struct expression"). Never delete the
//! row to make it green.

#[test]
fn an_ungated_doorbell_plan_does_not_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
