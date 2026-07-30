//! ★★★ The full-suite runner's own floors and its ledger rule, asserted from outside it.
//!
//! # What this guards
//!
//! `scripts/run_full_suite.sh` is the one command that runs everything on real hardware, and
//! its output — `RAN n / FAILED n / SKIPPED n` — is what a human reads instead of re-deriving
//! the state of the tree. That makes it the same *class* of instrument as `scripts/ci_gates.sh`
//! and it inherits the same two failure modes, which `gate_runner_floor.rs` documents at
//! length:
//!
//!   1. **the runner reports success having run nothing** — the shape that once printed
//!      `ALL GATES CLEAN (0 steps)` and exited 0;
//!   2. **the guard against (1) is deleted by an edit to the thing it guards**, and nothing
//!      says so, because the script simply goes back to being cheerful.
//!
//! The script defends (1) with pinned literals. This file defends (2), from a different file
//! and a different mechanism, exactly as `gate_runner_floor.rs` does for `ci_gates.sh`.
//!
//! # ★★ The rule that is easiest to lose, and the one worth most
//!
//! A ledger built on capability probes alone is **circular**: *"the resource was absent, so
//! skipping was fine"* is true on every machine and rules nothing out. What makes a skip *red*
//! is the box's **profile** — a claim the operator makes and the ledger checks. Delete that
//! rule and every run goes green with an empty ledger, which is the most reassuring possible
//! form of "nothing ran".
//!
//! # ★ What this test deliberately does NOT do
//!
//! It does not re-derive the phase count, the target universe or the gate families. That would
//! be a second implementation of the runner living next to the first — the shape that let the
//! tier rule keep two evaluation sites and the ratchet keep a blind spot. Those are facts to be
//! *measured* by running the script; this test only insists the pins exist and bite.

use std::path::PathBuf;

fn runner() -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the tests crate lives one level below the root")
        .join("scripts/run_full_suite.sh");
    std::fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!(
            "★ the full-suite runner {} is not readable: {e}. It is the ONE documented way to \
             run everything on real hardware (docs/reference/full_suite_on_real_hardware.md); \
             if it moved, move this test with it rather than deleting the assertion.",
            p.display()
        )
    })
}

/// The literal assigned to `name`, e.g. `PHASE_FLOOR=17` -> `17`.
fn pinned(src: &str, name: &str) -> u32 {
    let line = src
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with(&format!("{name}=")))
        .unwrap_or_else(|| {
            panic!(
                "★ `{name}` is gone from scripts/run_full_suite.sh. It is a pinned floor, and a \
                 floor DERIVED from the thing it checks moves silently when that thing moves — \
                 which is precisely what a floor must not do. If phases were genuinely removed, \
                 LOWER the literal in the same commit; do not delete it."
            )
        });
    line.split('=')
        .nth(1)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_else(|| panic!("★ `{name}` is no longer a plain integer literal: {line}"))
}

#[test]
fn the_runner_pins_a_floor_on_its_own_phase_list() {
    let src = runner();
    let floor = pinned(&src, "PHASE_FLOOR");

    // ★ Not `> 0`. A floor of one is a floor that a truncation to one surviving phase walks
    // straight through. Loose on purpose: this test's job is that a REAL number is pinned, not
    // to become a second copy of it.
    assert!(
        floor >= 10,
        "★ PHASE_FLOOR is pinned at {floor}, below anything this runner has ever had. Either \
         the phase table collapsed or the literal was lowered to make a red run go away."
    );

    // The floor must not have drifted ABOVE the table, which would make every run exit 3
    // ("phase list truncated") and be indistinguishable from real truncation.
    let declared = src
        .lines()
        .filter(|l| l.starts_with("phase "))
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    assert!(
        declared >= floor,
        "★ PHASE_FLOOR is {floor} but only {declared} phases are declared. Every run would \
         abort with 'PHASE LIST TRUNCATED' having run nothing — the exact failure the floor \
         exists to report, produced by the floor itself."
    );
}

#[test]
fn the_phase_floor_is_compared_before_any_phase_runs() {
    let src = runner();
    // A pinned constant nothing reads is the same as no constant at all, and it is a shape that
    // survives review because the definition looks purposeful sitting there.
    let floor_at = src
        .find(r#"[ "${#PHASE_NAMES[@]}" -lt "$PHASE_FLOOR" ]"#)
        .expect(
            "★ nothing compares the declared phase count against PHASE_FLOOR. The floor is \
             decorative.",
        );
    let loop_at = src
        .find(r#"for i in "${!PHASE_NAMES[@]}"; do"#)
        .expect("the phase loop");
    // ★★ Ordering is the whole point. A floor evaluated after the loop still exits non-zero,
    // but by then a screenful of `=== <phase>` / `ok` has been printed — the reassuring output
    // a truncated run must never produce.
    assert!(
        floor_at < loop_at,
        "★ the phase floor is checked AFTER the phases run. A truncated table would print a \
         page of passing phases first, which is the reassurance the floor exists to withhold."
    );
    assert!(
        src.contains("PHASE LIST TRUNCATED"),
        "★ the truncation refusal must say the phase LIST is suspect — a different fact from \
         'a phase failed', and it sends a reader somewhere else."
    );
}

#[test]
fn a_skip_the_box_should_not_have_had_turns_the_run_red() {
    let src = runner();

    // The profile is what makes a skip red rather than informational; see the module docs.
    assert!(
        src.contains("profile_reqs()"),
        "★ the profile table is gone. Without it the ledger is circular: 'the resource was \
         absent so skipping was fine' is true on every machine."
    );
    assert!(
        src.contains("gpu-box)") && src.contains("ci)"),
        "★ a profile disappeared. `gpu-box` is the authoritative configuration and `ci` is what \
         a GitHub runner can honestly satisfy; losing either makes the other unfalsifiable."
    );
    assert!(
        src.contains("SKIPPED FOR A REASON THIS BOX WAS SUPPOSED TO SATISFY"),
        "★ the rule that turns an unexpected skip red is gone. A clean exit must mean \
         'everything this box can run, ran' — with this rule removed, a run in which every \
         phase skipped would exit 0 with an empty ledger, which is the most reassuring \
         possible form of nothing having happened."
    );

    // ★ And the acknowledgement must stay EXPLICIT and per-phase. A blanket
    // `--ignore-skips` would restore the same silence through a friendlier door.
    assert!(
        src.contains("--allow-skip") && src.contains("ACKNOWLEDGED"),
        "★ the named-acknowledgement path is gone; a skip can now only be red or invisible."
    );
    assert!(
        !src.contains("--allow-all-skips") && !src.contains("--ignore-skips"),
        "★ a blanket skip-suppression flag appeared. `--allow-skip <phase>` is deliberately \
         per-phase and deliberately printed in the ledger: acknowledging a skip must cost a \
         named argument, or it becomes the default."
    );
}

#[test]
fn the_three_censuses_are_wired_and_derive_rather_than_hand_list() {
    let src = runner();

    for (phase, func) in [
        ("target-census", "census_targets"),
        ("gate-census", "census_gates"),
        ("workspace-census", "census_workspaces"),
    ] {
        assert!(
            src.contains(&format!("phase {phase} ")),
            "★ the `{phase}` phase is gone from the table, so `{func}` can no longer run."
        );
        assert!(
            src.contains(&format!("{func}() {{")),
            "★ `{func}` is declared as a phase but no longer defined — the phase would fail \
             with 'command not found', which reads as a broken script rather than as the \
             census it replaced."
        );
    }

    // ★★ The gate families must be DERIVED from the run's own markers. A hand-list is a
    // smaller true statement, and this repository has been bitten three times by a list that
    // quietly stopped describing the tree (a `-maxdepth 1` ratchet, a token list, three crate
    // lists). The derivation is the grep below; if it goes, the census silently narrows to
    // whatever someone remembered to type.
    assert!(
        src.contains(r"grep -oE '\b[A-Z0-9][A-Z0-9-]*-GATE: (RAN|SKIPPED)'"),
        "★ the gate-family derivation is gone. Families must come out of the markers the run \
         emits, so a family added tomorrow is counted tonight."
    );
    // The target universe must come from cargo, not from a list in the script.
    assert!(
        src.contains("cargo") && src.contains("metadata"),
        "★ the target universe is no longer derived from `cargo metadata`; a hand-list of test \
         targets weakens the moment someone adds one."
    );

    let universe_floor = pinned(&src, "TARGET_UNIVERSE_FLOOR");
    assert!(
        universe_floor >= 50,
        "★ TARGET_UNIVERSE_FLOOR is {universe_floor}. An empty or collapsed universe trivially \
         satisfies 'every target ran', so the floor is what stops the census reporting success \
         over nothing."
    );

    // ★ The `#[ignore]` allowance is a literal and not an env knob, for the same reason as
    // every other floor here: a second ignored test must be a diff someone can see.
    let ignored = pinned(&src, "IGNORED_ALLOWANCE");
    assert!(
        ignored <= 2,
        "★ IGNORED_ALLOWANCE is {ignored}. Nothing in this tree carries an `#[ignore]` \
         attribute — the one entry a run reports is a ```ignore fenced block in a module doc. \
         Raising this hides a real one, and an ignored test is invisible in a summary and \
         impossible to count, which is why this repo uses loud runtime skips instead."
    );
}

#[test]
fn a_red_caused_by_the_box_cannot_be_reported_as_a_red_caused_by_the_code() {
    // ★★★ Three of these landed in one evening, each costing a wasted verification cycle: a
    // reached-count step reporting `total=0` because a sibling process wrote the same `/tmp`
    // path; a background run reported "failed, exit 1" that was purely a full disk; a suite
    // run in a shared tree failing a seam gate on another agent's untracked crate. Same
    // shape every time — an environment failure wearing the costume of a code failure — and
    // the ledger is the only place that can tell them apart, because it is the only thing
    // that sees both.
    let src = runner();

    assert!(
        src.contains("DISK_REFUSE_MB") && src.contains("REFUSING TO RUN"),
        "★ the disk floor is gone. A build that dies of ENOSPC reports a compiler error, and \
         a compiler error reads as a code defect. Refusing to start is the only answer that \
         cannot be misattributed."
    );
    assert!(
        src.contains("exit 4"),
        "★ the unfit-box refusal no longer has its own exit status. Sharing `1` with a real \
         failure is exactly the conflation this test exists to prevent."
    );
    assert!(
        src.contains("git status --porcelain"),
        "★ the runner no longer reports working-tree state. The boundary, vocabulary, \
         unsafe-surface and ABI-quarantine gates grep the tree AS IT IS, so a file belonging \
         to another writer is a real input to a real gate — a red run needs that fact beside \
         it, not discovered an hour later."
    );
}

#[test]
fn a_reached_count_of_zero_names_which_kind_of_zero_it_is() {
    // ★★★ MEASURED 2026-07-30. `ran=0 skipped=0 total=0` was reported on a box that has
    // /dev/kvm and both vendored ogkm trees, and whose own test run had just printed 51
    // `KVM-GATE: RAN` and 10 `VBIOS-ORACLE-GATE: RAN`. The gates were reading a concurrent
    // run's file. That output reads as *the gated tests vanished* — a confident wrong
    // answer, and strictly worse than no check at all, because someone acts on it.
    //
    // "The producer ran and genuinely found zero" and "I could not read my input" must not
    // be representable the same way. The producer writes a completion sentinel; every
    // consumer refuses, with its own exit status, over an input that lacks one.
    let ci = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the tests crate lives one level below the root")
        .join(".github/workflows/ci.yml");
    let src = std::fs::read_to_string(&ci)
        .unwrap_or_else(|e| panic!("★ {} is not readable: {e}", ci.display()));

    assert!(
        src.contains("KAYFABE-TEST-LOG-COMPLETE rc="),
        "★ the test step no longer writes a completion sentinel, so the reached-count steps \
         below it cannot tell a real zero from an unreadable input."
    );
    let guards = src.matches("INFRASTRUCTURE FAILURE, NOT A TEST RESULT").count();
    assert!(
        guards >= 3,
        "★ only {guards} reached-count step(s) refuse over an unreadable log; there are three \
         (KVM, SANDBOX, VBIOS-ORACLE). A step without the guard reports a count of zero and \
         calls it a regression."
    );
    assert!(
        src.contains("${KAYFABE_TEST_LOG:-/tmp/kayfabe-test.log}"),
        "★ the test log path is hardcoded again. A fixed name in /tmp is SHARED: two runs on \
         one box clobber each other, which is invisible on GitHub (one runner per job) and \
         wrong exactly where the authoritative run happens."
    );

    // And the runner that drives those steps must actually set the variable — a default that
    // nothing overrides is the old hardcoded path with extra syntax.
    assert!(
        runner_gates().contains("export KAYFABE_TEST_LOG"),
        "★ scripts/ci_gates.sh no longer exports a private KAYFABE_TEST_LOG, so concurrent \
         local runs share `/tmp/kayfabe-test.log` again. The steps are extracted from ci.yml \
         and run in child shells, so the environment is the only channel that reaches them."
    );
}

fn runner_gates() -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the tests crate lives one level below the root")
        .join("scripts/ci_gates.sh");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("★ {} is not readable: {e}", p.display()))
}

#[test]
fn every_cargo_workspace_in_the_tree_is_named_by_the_runner() {
    // ★★★ `cargo test --workspace` is a statement about ONE workspace. A `Cargo.toml` with its
    // own `[workspace]` table is a ROOT, and nothing run at the repository root reaches its
    // members — which is how `crates/kayfabe-abi/gen`'s 22 unit tests came to have never run
    // anywhere, in any job, ever.
    //
    // This is the *static* half of `census_workspaces`: the script discovers the roots at run
    // time and fails on an uncovered one; this asserts the pinned list still exists and still
    // names the roots that are in the tree right now, so deleting the list turns the suite red
    // rather than turning the census quiet.
    let src = runner();
    let handled = src
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("HANDLED_WORKSPACES="))
        .expect(
            "★ HANDLED_WORKSPACES is gone. Discovery without a pinned list means being found is \
             the same as being covered, and a new detached sub-project rejoins the dark.",
        );

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the tests crate lives one level below the root")
        .to_path_buf();
    let mut roots = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name();
            if p.is_dir() {
                if matches!(
                    name.to_str(),
                    Some("target" | ".git" | "mutants.out" | "corpus" | "artifacts")
                ) {
                    continue;
                }
                stack.push(p);
            } else if name == "Cargo.toml"
                && std::fs::read_to_string(&p).is_ok_and(|s| {
                    s.lines().any(|l| l.trim_start().starts_with("[workspace]"))
                })
            {
                let rel = p
                    .parent()
                    .and_then(|d| d.strip_prefix(&root).ok())
                    .map(|d| d.to_string_lossy().into_owned())
                    .unwrap_or_default();
                roots.push(if rel.is_empty() { ".".to_owned() } else { rel });
            }
        }
    }

    // Non-vacuity: the walk must have found the roots we know exist, or it is measuring
    // nothing and the loop below is a constant function.
    assert!(
        roots.len() >= 2,
        "★ the walk found {} cargo workspace root(s). This repository has at least the root and \
         `fuzz/`; the instrument is broken, not the tree.",
        roots.len()
    );

    for r in &roots {
        assert!(
            handled.contains(r),
            "★ `{r}` is a cargo WORKSPACE ROOT that scripts/run_full_suite.sh does not name in \
             HANDLED_WORKSPACES. Nothing run at the repository root reaches its tests — not \
             `cargo test --workspace`, not `--all-targets`, not any CI job. Add a phase that \
             runs them and add the path to HANDLED_WORKSPACES in the same commit. (Roots found: \
             {roots:?})"
        );
    }
}
