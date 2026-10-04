// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The hardware rows of the raw-address perimeter's merge bar (`docs/design/V3_RAWADDR_PERIMETER.md`
//! §8) are RUN by a gate the bench runs, not only written down (review of `v3-sec-rawaddr`,
//! 2026-10-04: `kf_cuda::selftest` had no caller under `crates/`, and no gate dropped an image
//! between its submit and its collect).
//!
//! A source check, because the rows need a GPU: `scripts/bench/v3_gates.sh` runs `kf-gate1..9`, so
//! a row is reachable when `kf-gate9`'s `main` calls it.

const GATE9: &str = include_str!("../src/bin/kf-gate9.rs");

/// `main`'s body (from its signature to the next top-level item).
fn main_body() -> &'static str {
    let at = GATE9.find("\nfn main() {").expect("kf-gate9 has a main");
    let rest = &GATE9[at + 1..];
    let end = rest.find("\n}\n").expect("main ends");
    &rest[..end]
}

#[test]
fn gate9_runs_h3_and_the_cuda_selftest() {
    let main = main_body();
    for call in ["dropped_image(&mut l)", "selftest(&mut l)"] {
        assert!(
            main.contains(call),
            "kf-gate9's main no longer calls `{call}`"
        );
    }
    for used in [
        "kf_cuda::selftest::bring_up_and_prove()",
        "kf_cuda::selftest::probe_after_sandbox(",
        "drop(img);\n    let c = k.wait(",
    ] {
        assert!(GATE9.contains(used), "kf-gate9 lost `{used}`");
    }
}
