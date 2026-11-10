//! ★ The committed hardware reference table (`crates/kf-chip/data/hwref-580.159.04.tsv`) is what a
//! FRESH derivation from ogkm-580 prints — so the table the constant checks run against cannot drift
//! from the headers it claims to be generated from.
//!
//! ⊘ This does not re-implement a parser: it runs `tools/derive_hwref.sh --check`, which asks the
//! preprocessor which macros exist and the compiler what each evaluates to.
//!
//! ⚠ SKIPS (rather than fails) when ogkm-580 or gcc is absent AND `KF_REQUIRE_ORACLES` is unset —
//! "we could not ask" is not "the answer was yes", and cargo counts a printed SKIP as a pass. With
//! `KF_REQUIRE_ORACLES` set to anything but empty or `0` (the stripped-tree CI job and the boxes set
//! `1`), an absent oracle or compiler is a FAILURE: a job that is supposed to ask must not be able to
//! go green by not asking. A developer without the tree can still skip. Looked for, in order:
//! `$KF_OGKM_580`, `third_party/ogkm-580` (the pinned submodule, when initialised; CI and
//! `scripts/ci/fetch_oracles.sh` check it out shallow at the pinned commit), and the research clone
//! the campaign's citations are against.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Straight to the real stderr, like `kvm_gate`: libtest swallows `eprintln!` of a passing test, and
/// the stripped-tree CI job counts this line (a gate nobody can see is a gate nobody can count).
fn announce(line: &str) {
    let _ = writeln!(std::io::stderr(), "{line}");
}

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

/// `KF_REQUIRE_ORACLES`: unset, empty or `0` lets an absent oracle skip; anything else requires it.
/// Fail closed on a typo (`true`, `yes`): only the two spellings of "off" are off.
fn oracles_required(value: Option<&str>) -> bool {
    !matches!(value, None | Some("" | "0"))
}

#[derive(Debug, PartialEq, Eq)]
enum Gate {
    Run,
    Skip(&'static str),
    Fail(&'static str),
}

const NO_OGKM: &str = "no ogkm-580 tree (set KF_OGKM_580 or run scripts/ci/fetch_oracles.sh)";

/// What an absent input means: a skip for a developer, a failure where oracles are required.
fn gate(ogkm_found: bool, gcc_found: bool, required: bool) -> Gate {
    let missing = if !ogkm_found {
        NO_OGKM
    } else if !gcc_found {
        "no gcc"
    } else {
        return Gate::Run;
    };
    if required {
        Gate::Fail(missing)
    } else {
        Gate::Skip(missing)
    }
}

#[test]
fn present_inputs_run_whether_or_not_oracles_are_required() {
    assert_eq!(gate(true, true, false), Gate::Run);
    assert_eq!(gate(true, true, true), Gate::Run);
}

#[test]
fn an_absent_ogkm_tree_fails_when_oracles_are_required() {
    assert_eq!(gate(false, true, true), Gate::Fail(NO_OGKM));
    assert_eq!(gate(false, false, true), Gate::Fail(NO_OGKM));
}

#[test]
fn an_absent_compiler_fails_when_oracles_are_required() {
    assert_eq!(gate(true, false, true), Gate::Fail("no gcc"));
}

#[test]
fn an_absent_ogkm_tree_skips_for_a_developer() {
    assert_eq!(gate(false, true, false), Gate::Skip(NO_OGKM));
    assert_eq!(gate(false, false, false), Gate::Skip(NO_OGKM));
}

#[test]
fn an_absent_compiler_skips_for_a_developer() {
    assert_eq!(gate(true, false, false), Gate::Skip("no gcc"));
}

#[test]
fn unset_leaves_the_oracles_optional() {
    assert!(!oracles_required(None));
}

#[test]
fn empty_leaves_the_oracles_optional() {
    assert!(!oracles_required(Some("")));
}

#[test]
fn zero_leaves_the_oracles_optional() {
    assert!(!oracles_required(Some("0")));
}

#[test]
fn one_requires_the_oracles() {
    assert!(oracles_required(Some("1")));
}

#[test]
fn true_requires_the_oracles_rather_than_skipping() {
    assert!(oracles_required(Some("true")));
}

#[test]
fn yes_requires_the_oracles_rather_than_skipping() {
    assert!(oracles_required(Some("yes")));
}

#[test]
fn other_spellings_of_zero_require_the_oracles_rather_than_skipping() {
    for typo in ["00", " 0", "0 ", "off"] {
        assert!(oracles_required(Some(typo)), "{typo:?}");
    }
}

#[test]
fn a_developer_who_sets_zero_skips_without_the_oracle() {
    assert!(matches!(
        gate(false, true, oracles_required(Some("0"))),
        Gate::Skip(_)
    ));
}

#[test]
fn a_developer_who_sets_nothing_skips_without_the_oracle() {
    assert!(matches!(
        gate(false, true, oracles_required(None)),
        Gate::Skip(_)
    ));
    assert!(matches!(
        gate(false, true, oracles_required(Some(""))),
        Gate::Skip(_)
    ));
}

#[test]
fn a_ci_job_that_sets_one_fails_without_the_oracle() {
    assert!(matches!(
        gate(false, true, oracles_required(Some("1"))),
        Gate::Fail(_)
    ));
}

#[test]
fn the_committed_table_is_a_fresh_derivation_from_ogkm_580() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let required = oracles_required(std::env::var("KF_REQUIRE_ORACLES").ok().as_deref());
    let og = ogkm_580();
    let gcc = Command::new("gcc").arg("--version").output().is_ok();
    match gate(og.is_some(), gcc, required) {
        Gate::Run => announce(&format!(
            "ORACLE-GATE: RAN hwref_is_current required={required}"
        )),
        Gate::Skip(why) => {
            announce(&format!(
                "ORACLE-GATE: SKIPPED hwref_is_current: {why} — NOT a pass"
            ));
            return;
        }
        Gate::Fail(why) => panic!("KF_REQUIRE_ORACLES is set and the oracle cannot run: {why}"),
    }
    let og = og.expect("gate said the tree exists");
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
