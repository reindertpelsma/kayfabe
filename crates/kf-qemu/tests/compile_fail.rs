// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ kf-qemu's compile-fail rows (`v3-sec-rawaddr`, 2026-10-04; audit S1-03): the console's frame
//! view carries no address. trybuild compares full compiler stderr: regenerate with
//! `TRYBUILD=overwrite` after confirming the errors are the same errors; never delete a row.

/// The rows, by name, compared both ways with `tests/ui/`.
const REQUIRED_ROWS: &[&str] = &["frame_view_has_no_address.rs"];

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
    assert_eq!(
        found, REQUIRED_ROWS,
        "the matrix changed without REQUIRED_ROWS changing"
    );
}

#[test]
fn the_frame_view_carries_no_address() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
