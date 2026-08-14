//! ★★★★★ **w323 — the deferral asymmetry as a type error.**
//!
//! The queue accepts `MapPublication` and nothing else. The row under `tests/ui/` hands it
//! a `Revocation` and **must not compile**.
//!
//! ## What it proves, and what it does not
//!
//! It proves there is **no route** from a revocation into the deferred lane: no `From`, no
//! `Deref`, no public field to unwrap, no `as_map()`. Those are the three dismantling routes
//! `tests/tests/host_class_role_wiring.rs` enumerates for a role type, and the runtime
//! complement — a census that watches them stay absent — is
//! `pubqueue::tests::the_revocation_type_has_no_route_into_the_queue`. A type guards the
//! boundary; a census guards the mint.
//!
//! ⊘ It does **not** prove that every revocation in the tree flows through `Revocation`.
//! Today's revocations are `VerbPlan::Release` chains reached through
//! `kayfabe_fwd::dispose_on` and `Proc::drop`, which are synchronous and stay so by design.
//! What this row makes impossible is a future patch *adding* them to the deferred lane
//! because it looked like the same kind of work.
//!
//! ## Maintenance note
//!
//! `trybuild` compares full compiler stderr, so a rustc reword can turn this red with
//! nothing wrong. Re-bless with `TRYBUILD=overwrite cargo test -p kayfabe-device --test
//! compile_fail` **after** confirming the error is still a type mismatch on `offer`'s
//! argument. Never delete the row to make it green.

#[test]
fn a_revocation_cannot_be_put_on_the_deferred_publication_lane() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
