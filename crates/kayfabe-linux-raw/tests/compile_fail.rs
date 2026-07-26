//! ★ The compile-fail matrix — `l1_os_shell.md` §4.6, rows 1–10.
//!
//! Each file under `tests/ui/` is a dangerous pattern that **must not compile**. This is
//! the mechanical half of §4.2.1's five refusals; the rustdoc in `src/lib.rs` says which
//! of the five each row holds, and — importantly — which one no compiler can hold.
//!
//! ## What this suite proves, and what it does not
//!
//! It proves that *today's* API has no route to a host CPU address: no accessor returns a
//! borrow or a pointer, no `Deref`/`AsRef`/`Index` impl offers a sideways route to one, no
//! base address is readable as an integer, `Gpa` and `HostOffset` do not convert, an
//! offset has no unchecked arithmetic, a bounded object cannot outlive its mapping, a
//! region cannot cross a thread boundary, and there is no address-taking placement call.
//!
//! It does **not** prove that a future addition will not open one — a compile-fail test is
//! an assertion about the code as written — and it cannot see a *semantically* unbounded
//! bounded object (a region whose length field is right but whose backing was mapped
//! shorter). Those are the named review obligation in §11's exit gate. Types close four of
//! the five refusals; the fifth is a human, and pretending otherwise would be the exact
//! failure mode the design keeps cataloguing.
//!
//! ## Maintenance note, stated plainly
//!
//! `trybuild` compares full compiler stderr, so a rustc diagnostic reword can turn a row
//! red without anything being wrong. The fix is `TRYBUILD=overwrite cargo test -p
//! kayfabe-linux-raw --test compile_fail` **after** confirming the errors are still the
//! same errors. Never delete a row to make it green.

#[test]
fn dangerous_patterns_do_not_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
