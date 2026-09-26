//! ★ The committed hardware reference table (`crates/kf-chip/data/hwref-580.159.04.tsv`) is what a
//! FRESH derivation from ogkm-580 prints — so the table the constant checks run against cannot drift
//! from the headers it claims to be generated from.
//!
//! ⊘ This does not re-implement a parser: it runs `tools/derive_hwref.sh --check`, which asks the
//! preprocessor which macros exist and the compiler what each evaluates to.
//!
//! ⚠ SKIPS (rather than fails) when ogkm-580 or gcc is absent — "we could not ask" is not "the answer
//! was yes", and a box without the oracle must not manufacture a green. Looked for, in order:
//! `$KF_OGKM_580`, `third_party/ogkm-580` (the pinned submodule, when initialised), and the research
//! clone the campaign's citations are against.

use std::path::{Path, PathBuf};
use std::process::Command;

fn ogkm_580() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let candidates = [
        std::env::var_os("KF_OGKM_580").map(PathBuf::from),
        Some(root.join("third_party/ogkm-580")),
        Some(PathBuf::from(
            "/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04",
        )),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|p| p.join("src/common/inc/swref/published/nv_ref.h").exists())
}

#[test]
fn the_committed_table_is_a_fresh_derivation_from_ogkm_580() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let Some(og) = ogkm_580() else {
        eprintln!("SKIP: no ogkm-580 tree (set KF_OGKM_580) — NOT a pass");
        return;
    };
    if Command::new("gcc").arg("--version").output().is_err() {
        eprintln!("SKIP: no gcc — NOT a pass");
        return;
    }
    let out = Command::new("bash")
        .arg(root.join("tools/derive_hwref.sh"))
        .arg("--check")
        .arg(root.join("crates/kf-chip/data/hwref-580.159.04.tsv"))
        .arg(&og)
        .output()
        .expect("bash runs");
    assert!(
        out.status.success(),
        "the committed table is stale — regenerate with `tools/derive_hwref.sh > crates/kf-chip/data/hwref-580.159.04.tsv`:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_table_names_the_ogkm_version_the_bench_runs() {
    let head = kf_chip::hwref::TABLE.lines().next().unwrap_or_default();
    assert!(head.contains("ogkm 580.159.04"), "{head}");
}
