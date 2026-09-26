//! ★★★ The gate RUNNER's own floor, asserted from outside the runner.
//!
//! # The failure this exists for, and why it is the worst one
//!
//! On 2026-07-30 `scripts/ci_gates.sh` printed **`ALL GATES CLEAN (0 steps)`** and exited
//! **0**. A heredoc terminator at column 0 inside `ci.yml` had ended the YAML block scalar it
//! sat in, the file stopped parsing, the extractor returned nothing, and the runner reported
//! success **having run no gates at all**.
//!
//! Every other gate in this tree protects some property of the code. This one protects the
//! thing that runs those gates — so when it fails open, *all of them* silently do. That makes
//! it the highest-leverage instrument in the repository and the one whose own non-vacuity
//! matters most. It is the fourth time this project has been bitten by a gate that could not
//! fire (the `unsafe_code` ratchet blind to `extern "C"` entry points, the tier rule with a
//! second evaluation site, gate B passing by crashing, and this).
//!
//! # Why the check is in TWO places, and what each one cannot do
//!
//! `ci_gates.sh` carries a pinned literal floor and refuses below it. That catches the runtime
//! failure — an emptied or shortened step list — which is the actual threat. But a guard that
//! lives only inside the thing it guards can be *removed* by an edit to that thing, and
//! nothing would say so: the script would simply go back to being cheerful.
//!
//! So this test asserts, from a different file and a different mechanism, that the floor is
//! **still there and still load-bearing**. Neither half can silence the other:
//!
//! | removal | caught by |
//! |---|---|
//! | the step list is emptied or shortened at runtime | the script's floor |
//! | the floor literals are deleted from the script   | this test |
//! | the floor is defined but never compared          | this test |
//! | the floor is moved to *after* the gates have run | this test |
//!
//! ★ **What this test deliberately does NOT do** is re-derive the expected step count by
//! parsing `ci.yml` itself. That would be a second implementation of the extractor living
//! next to the first — the exact shape that let the tier rule keep two evaluation sites and
//! the ratchet keep a blind spot. The count is a fact to be *measured* (`ci_gates.sh --all |
//! tail -1`) and pinned by a human; this test only insists the pin exists and bites.

use std::path::PathBuf;

fn runner() -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the tests crate lives one level below the root")
        .join("scripts/ci_gates.sh");
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("★ the gate runner {} is not readable: {e}", p.display()))
}

/// The literal assigned to `name`, e.g. `GATE_STEPS_ALL_MIN=17` -> `17`.
fn pinned(src: &str, name: &str) -> u32 {
    let line = src
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with(&format!("{name}=")))
        .unwrap_or_else(|| {
            panic!(
                "★ `{name}` is gone from scripts/ci_gates.sh. That is the pinned floor on how \
                 many gate steps must be extracted before a run means anything. Without it the \
                 runner reports ALL GATES CLEAN over however many steps it happened to find, \
                 including zero — which it did, on 2026-07-30, and no test went red. If gates \
                 were genuinely removed, LOWER the literal; do not delete it."
            )
        });
    line.split('=')
        .nth(1)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_else(|| panic!("★ `{name}` is no longer a plain integer literal: {line}"))
}

#[test]
fn the_gate_runner_pins_a_floor_for_both_of_its_modes() {
    let src = runner();
    let all = pinned(&src, "GATE_STEPS_ALL_MIN");
    let fast = pinned(&src, "GATE_STEPS_FAST_MIN");

    // ★ Not `> 0`. A floor of 1 is a floor that a truncation to one surviving step walks
    // straight through, and this repository's gate list is an order of magnitude bigger than
    // that. The bound is loose on purpose — the test's job is that a REAL number is pinned,
    // not to become a second copy of it.
    assert!(
        all >= 10,
        "★ the --all floor is pinned at {all}, which is below anything this job has ever \
         had. Either the gate list collapsed or the literal was zeroed to make a red run go \
         away"
    );
    assert!(
        fast >= 5,
        "★ the default-mode floor is pinned at {fast}, which is implausibly low"
    );
    assert!(
        all > fast,
        "★ --all runs a superset of the default mode ({all} vs {fast}), so its floor cannot \
         be the lower of the two. One of the literals was edited without the other"
    );
}

#[test]
fn the_floor_is_actually_compared_and_not_merely_declared() {
    let src = runner();
    // A pinned constant nothing reads is the same as no constant at all — and it is a shape
    // that survives review, because the definition is right there looking purposeful.
    assert!(
        src.contains("floor=$GATE_STEPS_ALL_MIN") && src.contains("floor=$GATE_STEPS_FAST_MIN"),
        "★ the pinned literals are declared but never selected into `floor`, so nothing \
         compares against them"
    );
    assert!(
        src.contains(r#"[ "${#steps[@]}" -lt "$floor" ]"#),
        "★ nothing in the gate runner compares the extracted step count against the floor. \
         The floor is decorative"
    );
    assert!(
        src.contains("GATE LIST TRUNCATED"),
        "★ the truncation refusal must say that the gate LIST is suspect, which is a \
         different fact from `a gate failed` and sends a reader somewhere else"
    );
}

#[test]
fn the_floor_is_checked_before_any_gate_runs() {
    let src = runner();
    // ★★ Ordering is the whole point. A floor evaluated after the loop still exits non-zero,
    // but by then the run has printed a screenful of `=== <gate>` / `ok` — which is exactly
    // the reassuring output a truncated run must never produce. It must abort having said
    // nothing encouraging.
    let floor_at = src
        .find(r#"[ "${#steps[@]}" -lt "$floor" ]"#)
        .expect("the floor comparison");
    let loop_at = src
        .find(r#"for s in "${steps[@]}"; do"#)
        .expect("the step loop");
    assert!(
        floor_at < loop_at,
        "★ the floor is checked AFTER the gates run. A truncated list would print a page of \
         passing gates first, which is the reassurance the floor exists to withhold"
    );

    // And it must be lexically outside the construct that can be cut short — the extractor's
    // own heredoc. A guard inside the thing that gets truncated is removed by the truncation.
    let heredoc_at = src.find("<<'PY'").expect("the extractor heredoc");
    let decl_at = src.find("GATE_STEPS_ALL_MIN=").expect("the pinned literal");
    assert!(
        decl_at < heredoc_at,
        "★ the pinned floor is declared inside or after the extractor's heredoc. The heredoc \
         is the construct a truncation cuts; a floor living in it dies with the thing it was \
         supposed to detect"
    );
}

/// ★★★ The oracle census, and why a *second* guard on the same script is not duplication.
///
/// `[measured]` 2026-08-03, vast 46494693: `ci_gates.sh --all` printed
/// `ALL GATES CLEAN (22 steps, floor 22)` on a GPU bench where **every compiled-oracle test
/// was SKIPPED and asserted nothing** — the box had no open-kernel-modules tree at the
/// absolute path `tests/build.rs` looks for. The floor above was satisfied (22 steps really
/// did run); the oracle steps were satisfied too, because their floors count `ran + skipped`
/// **deliberately** — GitHub's runners can never carry those trees.
///
/// So this is a different failure from the one the floor catches. The floor asks *"did the
/// gates run?"*. This asks *"did the gates that ran have anything to compare against?"* —
/// and the honest answer was no, invisibly, for the entire life of the bench.
///
/// ⊘ **The cost, measured the same hour on the same box:** one mutation to
/// `decode_userd_index_chid` — dropping its refusal of `_USERD_INDEX_FIXED`, which RM answers
/// with `NV_ERR_INVALID_STATE` — was a **non-biter** with the trees absent and failed **two**
/// tests with them present. A skipped oracle does not merely add no confidence; it converts a
/// live guard into a dead one and the green it leaves behind is indistinguishable from a real
/// one.
///
/// This test exists for the same reason the floor test does: a guard living only inside the
/// script it guards can be deleted by an edit to that script, and nothing would say so.
#[test]
fn the_gate_runner_censuses_which_oracle_families_actually_ran() {
    let src = runner();
    assert!(
        src.contains("oracle_census()"),
        "★ `oracle_census` is gone from scripts/ci_gates.sh. Without it the runner's verdict \
         cannot distinguish a green run on a box that COULD compile the ogkm oracles from a \
         green run on a box that could not — and the second kind asserts nothing about them"
    );
    // Declared-but-never-called is the same shape the floor test guards against, and it
    // survives review because the definition sits there looking purposeful. Both call sites
    // matter: the red path is exactly when somebody asks which checks were real.
    let calls = src.matches("\n  oracle_census\n").count();
    assert!(
        calls >= 2,
        "★ `oracle_census` is called {calls} time(s); it must run on BOTH the clean and the \
         failed path. A census only on green tells you nothing on the day a gate goes red, \
         which is the day the question gets asked"
    );
    // ★ The family list must be DERIVED from what the tests emit. A hand-kept list is a
    // smaller universe than the one it claims to cover (`gates_quantified_over_a_list`), and
    // the first draft of this census took its names from this script's own prose comments
    // rather than from the tests and reported `RAN=0 SKIPPED=0` for two live families.
    assert!(
        src.contains("ORACLE-GATE") && src.contains("tests/tests/*.rs"),
        "★ the census no longer derives its family list from the `<FAMILY>-ORACLE-GATE` \
         markers under tests/tests/. A hand-kept list silently stops covering a family the \
         day one is added"
    );
    // And an empty derivation must SHOUT rather than print nothing — a silent census is the
    // exact shape the whole function exists to remove.
    assert!(
        src.contains("ORACLE CENSUS FAILED"),
        "★ the census returns quietly when it derives no families. That is indistinguishable \
         from `there are no oracle families`, which is the failure mode being fixed"
    );
}
