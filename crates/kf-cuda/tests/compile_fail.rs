// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ kf-cuda's compile-fail matrix (`v3-sec-rawaddr`, 2026-10-04; audit S1-03/S1-04/S1-10/S1-16;
//! OWNER_RULINGS §R gates 2, 4, 5; docs/design/V3_RAWADDR_PERIMETER.md §6 G2).
//!
//! Each file under `tests/ui/` is a use of kf-cuda that must NOT compile: the raw binding and the
//! device-address type unreachable, the launch structs unbuildable, a console frame with no address
//! accessor and the only destination of a GPU copy, memory-owning handles neither `Copy`, `Clone`
//! nor `Hash`, a composition holding its staging buffer, and an import that takes a borrowed fd.
//!
//! ⊘ What trybuild cannot express, and what covers it instead: that the perimeter's CHILD modules
//! cannot forge a raw range is crate-internal privacy (the fields are private to `driver_unsafe`'s
//! inline `raw` module) — held by the compiler on every build and listed by the export census
//! (`tests/perimeter_exports.rs`).
//!
//! Maintenance: `trybuild` compares full compiler stderr, so a rustc reword can turn a row red
//! without anything being wrong; regenerate with `TRYBUILD=overwrite` after confirming the errors
//! are the same errors. Never delete a row to make it green.

/// ★ The rows this matrix must contain, by name — compared both ways with `tests/ui/`, so a
/// deletion AND an unrecorded addition go red (`gates_quantified_over_a_list`).
const REQUIRED_ROWS: &[&str] = &[
    "composer_holds_the_staging.rs",
    "console_frame_has_no_addr.rs",
    "finish_needs_a_console_frame.rs",
    "image_is_not_clone.rs",
    "image_is_not_copy.rs",
    "image_is_not_hash.rs",
    "import_takes_a_borrowed_fd.rs",
    "no_device_address_type.rs",
    "no_launch_args_type.rs",
    "raw_binding_unreachable.rs",
];

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
        "the trybuild matrix changed without REQUIRED_ROWS changing: a missing row is a deleted \
         guard that leaves the suite green"
    );
}

#[test]
fn the_perimeter_cannot_be_reached_from_safe_code() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
