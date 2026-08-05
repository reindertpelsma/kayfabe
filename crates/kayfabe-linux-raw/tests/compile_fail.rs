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
//! One row is not about addresses at all: `cache_policy_has_no_default.rs` holds the
//! **no-silent-default** property of `CachePolicy` (see `src/cache.rs`). A default cache
//! attribute is a blanket policy, and every cacheability bug the C spent days on was a
//! blanket policy being corrected after the fact.
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

/// ★★★ The rows this matrix must contain, **by name**.
///
/// ⊘ `compile_fail("tests/ui/*.rs")` is a **glob**, so deleting a row leaves the suite green
/// with fewer rows and nothing goes red. The maintenance note above says *"never delete a row
/// to make it green"* — which is a request to a human, not a gate. This list is the gate:
/// `gates_quantified_over_a_list` — *shortening the list weakens the gate with ZERO red tests;
/// a smaller universe is a smaller true statement.*
///
/// ★ Names, not a count. A count catches a deletion; names also catch a **rename**, which is
/// how a row silently stops testing what its §4.6 entry says it tests.
const REQUIRED_ROWS: &[&str] = &[
    "cache_policy_has_no_default.rs",
    "map_fixed_bare_address.rs",
    "no_base_address.rs",
    "no_borrow_escape.rs",
    "no_wide_volatile_load.rs",
    "offset_arithmetic.rs",
    "page_size_literal.rs",
    "region_is_not_send.rs",
    "view_outlives_region.rs",
];

/// ★ The universe is DERIVED from the directory and compared against [`REQUIRED_ROWS`] in
/// **both** directions, so this fails on a deletion *and* on an addition nobody recorded.
///
/// An added row is not an error the way a missing one is — but an unrecorded row means
/// `l1_os_shell.md` §4.6's table no longer enumerates the matrix, and that table is what the
/// exit gate is quantified over. Adding a row is one line here; leaving §4.6 stale is the
/// failure this catches.
#[test]
fn the_compile_fail_matrix_still_has_every_row_it_claims() {
    let mut found: Vec<String> = std::fs::read_dir("tests/ui")
        .expect("tests/ui exists")
        .filter_map(|e| {
            let n = e.ok()?.file_name().to_string_lossy().into_owned();
            n.ends_with(".rs").then_some(n)
        })
        .collect();
    found.sort();
    let mut want: Vec<String> = REQUIRED_ROWS.iter().map(|s| (*s).to_owned()).collect();
    want.sort();
    assert_eq!(
        found, want,
        "★★★ the trybuild matrix changed without REQUIRED_ROWS changing. A missing row is a \
         DELETED GUARD that leaves the suite green (the glob simply tests fewer files); an \
         extra row means `l1_os_shell.md` §4.6's table no longer enumerates the matrix the \
         exit gate is quantified over. Fix the list AND §4.6 together — never one alone."
    );
}

#[test]
fn dangerous_patterns_do_not_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
