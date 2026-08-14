//! ★★★★★ **THE BENCH HARNESS, GATED FROM OUTSIDE IT.**
//!
//! `scripts/bench/` is ~90 shell scripts that boot a guest, grade the boot, and print the
//! sentence a rung is reported on. `[census, w295]` **nothing tests any of it.**
//! `tests/tests/gate_runner_floor.rs` and `tests/tests/full_suite_ledger.rs` do exactly this
//! kind of check — read a script as text, assert a property of it — but their subjects are
//! `scripts/ci_gates.sh` and `scripts/run_full_suite.sh`. The defects below all happened in
//! `scripts/bench/`, where no such gate exists.
//!
//! ⚠ **Every rule here is generalised from a measured incident that produced a FALSE RESULT**,
//! which is worse than a red one: a wrong number is reported with a citation and nobody
//! re-checks it.
//!
//! | § | the incident | the rule |
//! |---|---|---|
//! | 1 | an unanchored `CUP2_RC` grep matched the guest **compiler's** `GCC_CUP2_RC=0` | a verdict read of `CUP2_RC` must be anchored, or labelled as contrast |
//! | 2 | `KAYFABE_TAG=w294${ARM}` set unconditionally ⇒ boot 3 overwrote boot 1's logs **while printing a perfect result under boot 1's filename** | a boot tag must be caller-influenceable |
//! | 3 | a grading block committed **after** the inherited `exit` line — twice, hours apart | no statement may follow a script's terminating `exit` |
//!
//! # ★ Why the classifiers are pure functions with their own fixtures
//!
//! Each rule below is a `fn` over one line (or one file) of text, tested against **literal
//! strings quoted from the tree** before it is ever pointed at the tree. That ordering is the
//! point: a scanner run only over a clean tree returns clean, and *clean is exactly what a
//! broken scanner returns*. The fixture tests are the known-positives —
//! `every_audit_ships_with_a_known_positive`, applied to the audit itself.
//!
//! ⊘ **What these gates deliberately do NOT do** is grade a boot's numbers. A red boot is
//! evidence too (`scripts/bench/assert_boot_evidence.sh` says so about its own scope). These
//! check that the *instrument* can report what it saw.

use std::path::{Path, PathBuf};

fn bench_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the tests crate lives one level below the root")
        .join("scripts/bench")
}

/// Every `*.sh` under `scripts/bench`, as `(file name, source)`.
fn bench_scripts() -> Vec<(String, String)> {
    let dir = bench_dir();
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("★ {} is not readable: {e}", dir.display()));
    for e in entries {
        let p = e.expect("a directory entry").path();
        if p.extension().is_some_and(|x| x == "sh") {
            let name = p
                .file_name()
                .expect("a file name")
                .to_string_lossy()
                .into_owned();
            let src = std::fs::read_to_string(&p)
                .unwrap_or_else(|e| panic!("★ {} is not readable: {e}", p.display()));
            out.push((name, src));
        }
    }
    out.sort();
    out
}

/// The bench directory's own non-vacuity, asserted once and reused. A scan over an empty set
/// is the shape every gate in this tree has been bitten by at least once.
fn scripts_or_panic() -> Vec<(String, String)> {
    let s = bench_scripts();
    assert!(
        s.len() >= 40,
        "★ NON-VACUITY: only {} scripts found under {}. Every rule in this file quantifies over \
         that set, so a set this small means the scan lost its subject and all three gates below \
         are passing over nothing",
        s.len(),
        bench_dir().display()
    );
    s
}

// =====================================================================================
// § 1 — THE ANCHOR
// =====================================================================================

/// Is this line a **verdict read** of `CUP2_RC` that a `GCC_CUP2_RC=0` can satisfy?
///
/// # ⊘⊘ The incident
///
/// `cup2`'s exit status is reported as `CUP2_RC=<n>` in the guest's probe log. The step that
/// *compiles* `cup2` in the guest reports its own status as `GCC_CUP2_RC=<n>` — and it prints
/// **first**. So `grep -m1 -oE 'CUP2_RC=[0-9]+'` returns the **compiler's** status, and a
/// compiler that built fine reports `0`. `[measured]` this would have reported **success on
/// seven consecutive failing rungs**; `rm.rs`'s `met_the_whole_bar` doc names it as the same
/// class, one plane over.
///
/// # What counts as safe, and why each form is
///
/// * `^CUP2_RC=` — the line must *begin* with it. The strongest form; `GCC_` cannot precede.
/// * `(^|[^A-Z_])CUP2_RC=` — a boundary. Weaker than `^` but rules out any `[A-Z_]`-prefixed
///   sibling, which is the whole known population.
/// * `[A-Z_]*CUP2_RC=` — **prefix-preserving**: the match text carries whatever prefix was
///   there, so a census printing `uniq -c` over it shows `GCC_CUP2_RC` as a distinct row rather
///   than folding it into the answer.
/// * `GCC_CUP2_RC` — a different token entirely; reading it is not reading `CUP2_RC`.
/// * a line saying **contrast** — the tree's own convention for *"printed beside the anchored
///   read so the reader can see the ambiguity"*, and those lines are never assigned to a
///   verdict variable. ⊘ This is the one judgement-based exemption here, and it is why the
///   word must appear on the same line as the grep rather than in a comment above it.
/// * a comment line — the trap is discussed in ~15 comment blocks and describing it is not
///   committing it.
#[must_use]
fn is_unanchored_cup2_read(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return false;
    }
    if !line.contains("CUP2_RC") {
        return false;
    }
    // Only greps read it; an `echo "CUP2_RC=..."` is a producer, not a consumer.
    if !line.contains("grep") && !line.contains("first ") {
        return false;
    }
    if line.to_ascii_lowercase().contains("contrast") {
        return false;
    }
    // Every occurrence in the line must be safe — a line that anchors one read and not the
    // other is exactly `w290p_run.sh`'s `strict=`/`loose=` pair, and only the first is a
    // verdict. Occurrences are examined by the two characters in front of them.
    let bytes = line.as_bytes();
    let mut idx = 0;
    let mut any_unsafe = false;
    while let Some(hit) = line[idx..].find("CUP2_RC") {
        let at = idx + hit;
        idx = at + "CUP2_RC".len();
        // `GCC_CUP2_RC` and friends: a different identifier.
        if at > 0 && matches!(bytes[at - 1], b'A'..=b'Z' | b'_' | b'*') {
            continue;
        }
        // `^CUP2_RC` — the regex anchor immediately in front.
        if at > 0 && bytes[at - 1] == b'^' {
            continue;
        }
        // `(^|[^A-Z_])CUP2_RC` — the boundary alternation.
        if line[..at].ends_with("(^|[^A-Z_])") {
            continue;
        }
        any_unsafe = true;
    }
    any_unsafe
}

/// The classifier's own known-positives and known-negatives, **quoted from the tree**, asserted
/// before the scan below is trusted.
#[test]
fn the_anchor_classifier_bites_the_line_that_actually_shipped() {
    // ⊘ KNOWN-POSITIVES — every one of these is a real line from `scripts/bench`, and each was
    // a verdict read at the time it was written.
    for bad in [
        r#"for a in $ARMS; do printf '%-14s' "$(first "$D/run_w264_${a}_probe.log" 'CUP2_RC=[0-9]+')"; done"#,
        r#"  rc=$(grep -o 'CUP2_RC=[A-Z0-9_]*' "$P" | tail -1)"#,
        r#"rc=$(grep -oE 'CUP2_RC=[0-9]+' "$P" | tail -1)"#,
    ] {
        assert!(
            is_unanchored_cup2_read(bad),
            "★ THE CLASSIFIER DID NOT BITE ITS OWN KNOWN-POSITIVE, so the scan below is a \
             constant function returning clean: {bad}"
        );
    }
    // KNOWN-NEGATIVES — the forms the tree uses deliberately. A gate that flags these gets
    // switched off within a rung, which is the same outcome as not having it.
    for good in [
        r#"strict=$(grep -oE '^CUP2_RC=[0-9]+' "$P" 2>/dev/null | tail -1)"#,
        r#"rc=$(grep -oE '(^|[^A-Z_])CUP2_RC=[0-9]+' "$P" 2>/dev/null | grep -oE 'CUP2_RC=[0-9]+' | tail -1)"#,
        r#"grep -h '^CUP2_RC=' "$P" 2>/dev/null | sed 's/^/      /'"#,
        r#"echo "    UNANCHORED, for contrast = [$(grep -oh 'CUP2_RC=[0-9]*' "$P" 2>/dev/null)]""#,
        r#"echo "        GCC_CUP2_RC    = [$(grep -oE 'GCC_CUP2_RC=[0-9]+' "$P" 2>/dev/null)]""#,
        r#"grep -oE '[A-Z_]*CUP2_RC=[A-Z0-9_]+' "$P" 2>/dev/null | sort | uniq -c"#,
        r#"# ⊘⊘ `grep -o 'CUP2_RC=[0-9]*'` ALSO matches `GCC_CUP2_RC=0`, the compiler's status"#,
        r#"echo "CUP2_RC=$rc""#,
    ] {
        assert!(
            !is_unanchored_cup2_read(good),
            "★ THE CLASSIFIER FLAGGED A FORM THE TREE USES ON PURPOSE. A gate with false \
             positives is a gate that gets deleted: {good}"
        );
    }
}

#[test]
fn no_bench_script_reads_cup2_rc_with_a_pattern_the_compilers_status_satisfies() {
    let scripts = scripts_or_panic();
    let mut seen = 0usize;
    let mut offenders = Vec::new();
    for (name, src) in &scripts {
        for (n, line) in src.lines().enumerate() {
            if line.contains("CUP2_RC") {
                seen += 1;
            }
            if is_unanchored_cup2_read(line) {
                offenders.push(format!("{name}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    // ★ The scan's own non-vacuity: `CUP2_RC` is the campaign's headline metric and appears
    // dozens of times. A zero here would mean the files were read and the subject was not in
    // them — a clean result from a scan that measured nothing.
    assert!(
        seen >= 20,
        "★ NON-VACUITY: only {seen} lines mentioning CUP2_RC were found across {} scripts. \
         Either the metric was renamed (in which case this gate must be renamed with it) or the \
         scan is not reading what it thinks it is",
        scripts.len()
    );
    assert!(
        offenders.is_empty(),
        "★★★ {} unanchored CUP2_RC read(s). `GCC_CUP2_RC=0` — the guest COMPILER's status — \
         satisfies these patterns and is printed BEFORE cup2 runs, so `grep -m1` returns it. \
         This would have reported SUCCESS on seven consecutive failing rungs.\n\
         ⇒ Use `^CUP2_RC=`, or `(^|[^A-Z_])CUP2_RC=`, or put the word `contrast` on the line if \
         it really is printed beside an anchored read.\n    {}",
        offenders.len(),
        offenders.join("\n    ")
    );
}

// =====================================================================================
// § 2 — THE TAG
// =====================================================================================

/// A top-level assignment to a boot tag whose value **no caller can influence**.
///
/// `[measured 2026-08-14]` `w294_run.sh` read `export KAYFABE_TAG=w294${ARM}` unconditionally,
/// so `KAYFABE_TAG=w294cup2b ./w294_run.sh cup2` had its override silently discarded and the
/// **third boot overwrote the first boot's logs**. Nothing said so: the grading block printed a
/// perfect result, under the first boot's filename, from the third boot's data. What separated
/// them afterwards was that `MEMALLOC OK` happens to carry an ASLR-moved address — a
/// discriminator that exists by luck.
///
/// ⇒ The rule is the weakest one that would have caught it: the value must contain **at least
/// one parameter expansion**, so a caller has some way to reach it. `TAG=${KAYFABE_TAG:-w294x}`
/// and `TAG=${PFX}_pin` pass; `TAG=w289cup2` does not.
///
/// ⊘ It is NOT a collision guard. Two boots under one tag remain possible and are sometimes
/// intended; what is refused is a tag *nobody can change* when they are not.
#[must_use]
fn is_uninfluenceable_tag_assignment(line: &str) -> Option<String> {
    let t = line.trim_start();
    if t.starts_with('#') {
        return None;
    }
    // Top level only: an assignment inside a function or a loop is not the script's identity.
    if line.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = t
        .strip_prefix("export ")
        .map_or(t, str::trim_start)
        .strip_prefix("TAG=")
        .or_else(|| {
            t.strip_prefix("export ")
                .map_or(t, str::trim_start)
                .strip_prefix("KAYFABE_TAG=")
        })?;
    let value = rest.split_whitespace().next().unwrap_or(rest);
    // ★ Any expansion at all — `${KAYFABE_TAG:-…}`, `${PFX}_pin`, `$(date …)` or a bare `$1`.
    // The rule is *"a caller can reach this"*, not *"it uses one particular idiom"*: pinning the
    // idiom would flag `host_xid_watch.sh`'s `TAG=$1`, which is the most influenceable form
    // there is, and a gate with false positives is a gate that gets switched off.
    if value.contains('$') {
        return None;
    }
    Some(value.to_string())
}

#[test]
fn the_tag_classifier_bites_the_assignment_that_actually_shipped() {
    for (line, want) in [
        ("TAG=w289cup2", Some("w289cup2")),
        ("export KAYFABE_TAG=w294cup2", Some("w294cup2")),
    ] {
        assert_eq!(
            is_uninfluenceable_tag_assignment(line).as_deref(),
            want,
            "★ the classifier missed its own known-positive: {line}"
        );
    }
    for good in [
        "export KAYFABE_TAG=${KAYFABE_TAG:-w294${ARM}}",
        "TAG=${PFX}_pin",
        "TAG=${1:-r33}",
        "TAG=w291r33${ARM}",
        "TAG=$1",
        "  TAG=inside_a_function",
        "# TAG=w289cup2",
    ] {
        assert_eq!(
            is_uninfluenceable_tag_assignment(good),
            None,
            "★ false positive — a gate that flags the correct form gets deleted: {good}"
        );
    }
}

#[test]
fn every_bench_scripts_boot_tag_can_be_changed_by_its_caller() {
    let scripts = scripts_or_panic();
    let mut seen = 0usize;
    let mut offenders = Vec::new();
    for (name, src) in &scripts {
        for (n, line) in src.lines().enumerate() {
            let t = line.trim_start();
            if !line.starts_with(char::is_whitespace)
                && (t.starts_with("TAG=")
                    || t.starts_with("KAYFABE_TAG=")
                    || t.starts_with("export TAG=")
                    || t.starts_with("export KAYFABE_TAG="))
            {
                seen += 1;
            }
            if let Some(v) = is_uninfluenceable_tag_assignment(line) {
                offenders.push(format!("{name}:{}: TAG={v}", n + 1));
            }
        }
    }
    assert!(
        seen >= 10,
        "★ NON-VACUITY: only {seen} top-level tag assignments found — the scan is not reading \
         the runners"
    );
    assert!(
        offenders.is_empty(),
        "★★★ {} boot tag(s) no caller can change. A second run of the same script overwrites \
         the first run's logs in `/workspace/bench` AND in `traces/guest_boots`, and the grading \
         block prints a perfect result under the first boot's filename.\n\
         ⇒ Write `TAG=${{KAYFABE_TAG:-<default>}}`.\n    {}",
        offenders.len(),
        offenders.join("\n    ")
    );
}

// =====================================================================================
// § 3 — THE REACHABLE TAIL
// =====================================================================================

/// The 1-based line numbers of top-level statements that follow the script's last top-level
/// `exit`/`finish`, i.e. **code that cannot run**.
///
/// `[measured]` a grading block was committed **after** an inherited `exit` line — twice, hours
/// apart. It printed nothing, and *printing nothing* is what a grading block does when the run
/// had nothing to say, so the two states were indistinguishable. ⚠ The harness self-check that
/// would have reported it (`w294_run.sh`'s `MUST be >= 1`) lives **inside** such a block, so it
/// cannot detect its own case.
///
/// ⊘ Heredoc bodies are skipped: a `<<EOF` payload containing a column-0 `exit 0` is data, not
/// control flow, and treating it as a terminator would make this gate fire on the scripts that
/// *write* other scripts.
#[must_use]
fn statements_after_the_terminator(src: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = src.lines().collect();
    // Pass 1 — mark heredoc bodies.
    let mut in_heredoc: Vec<bool> = vec![false; lines.len()];
    let mut delimiter: Option<String> = None;
    for (i, line) in lines.iter().enumerate() {
        if let Some(d) = delimiter.clone() {
            in_heredoc[i] = true;
            if line.trim() == d {
                delimiter = None;
            }
            continue;
        }
        if let Some(pos) = line.find("<<") {
            let tail = &line[pos + 2..];
            let tail = tail.strip_prefix('-').unwrap_or(tail);
            let word: String = tail
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '"' || *c == '\'')
                .filter(|c| *c != '"' && *c != '\'')
                .collect();
            if !word.is_empty() {
                delimiter = Some(word);
            }
        }
    }
    // Pass 2 — the last top-level terminator.
    let is_terminator = |l: &str| {
        let w = l.split_whitespace().next().unwrap_or("");
        w == "exit" || w == "finish"
    };
    let Some(last) = (0..lines.len()).rev().find(|&i| {
        !in_heredoc[i] && !lines[i].starts_with(char::is_whitespace) && is_terminator(lines[i])
    }) else {
        return Vec::new();
    };
    // Pass 3 — anything executable after it.
    let mut after = Vec::new();
    for (i, line) in lines.iter().enumerate().skip(last + 1) {
        if in_heredoc[i] {
            continue;
        }
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        // Block closers belong to constructs that opened before the terminator.
        if matches!(t, "}" | "fi" | "esac" | "done" | ")" | ";;" | "*)" | "EOF") {
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        after.push((i + 1, t.to_string()));
    }
    after
}

#[test]
fn the_reachability_classifier_bites_a_planted_dead_tail() {
    // ⊘ KNOWN-POSITIVE — the shape that shipped: a grading block below the inherited exit.
    let planted = "#!/usr/bin/env bash\nset -e\nrun_the_boot\nexit \"$rc\"\n\
                   echo '=== GRADING ==='\ngrep -c Xid \"$LOG\"\n";
    let found = statements_after_the_terminator(planted);
    assert_eq!(
        found.len(),
        2,
        "★ THE CLASSIFIER DID NOT SEE ITS OWN PLANTED DEAD TAIL — the scan below would be a \
         constant function returning clean. found: {found:?}"
    );
    assert_eq!(found[0].0, 5, "the first dead line is reported by number");

    // KNOWN-NEGATIVES.
    let ok = "#!/usr/bin/env bash\necho '=== GRADING ==='\nexit \"$rc\"\n# a trailing comment\n";
    assert!(
        statements_after_the_terminator(ok).is_empty(),
        "★ a comment after the terminator is not code"
    );
    let heredoc = "#!/usr/bin/env bash\ncat > /tmp/x <<\"EOF\"\nexit 0\nEOF\nreal_work\nexit 0\n";
    assert!(
        statements_after_the_terminator(heredoc).is_empty(),
        "★ a heredoc BODY containing `exit 0` is data — flagging it makes this gate fire on \
         every script that writes another script"
    );
    let none = "#!/usr/bin/env bash\nmain \"$@\"\n";
    assert!(
        statements_after_the_terminator(none).is_empty(),
        "★ a script with no top-level exit has no dead tail to find"
    );
}

#[test]
fn no_bench_scripts_grading_block_sits_below_its_own_exit() {
    let scripts = scripts_or_panic();
    let mut terminated = 0usize;
    let mut offenders = Vec::new();
    for (name, src) in &scripts {
        let lines: Vec<&str> = src.lines().collect();
        if lines
            .iter()
            .any(|l| !l.starts_with(char::is_whitespace) && l.starts_with("exit"))
        {
            terminated += 1;
        }
        for (n, text) in statements_after_the_terminator(src) {
            offenders.push(format!("{name}:{n}: {text}"));
        }
    }
    // ★ Non-vacuity: most runners end in an explicit `exit`, so a near-zero count means the
    // terminator search is failing rather than the tree being clean.
    assert!(
        terminated >= 15,
        "★ NON-VACUITY: only {terminated} of {} scripts were seen to have a top-level `exit`. \
         The classifier is not finding terminators, so it cannot find code after them",
        scripts.len()
    );
    assert!(
        offenders.is_empty(),
        "★★★ {} statement(s) below a script's terminating `exit` — code that cannot run. \
         ⚠ A grading block there prints nothing, and printing nothing is indistinguishable from \
         a run with nothing to report. This has been committed twice.\n    {}",
        offenders.len(),
        offenders.join("\n    ")
    );
}

// =====================================================================================
// § 4 — the evidence gate itself must stay executable
// =====================================================================================

/// `scripts/bench/assert_boot_evidence.sh` is the only exit-status gate over a committed boot.
/// `[census, w295]` it is named by **no** CI job and by **no** `run_full_suite.sh` phase — it
/// runs when a human remembers. That is a real gap and it is recorded in this rung's report;
/// what this test guards is narrower and is the half a reader cannot check by eye: that the
/// script still exists, is executable, and still asserts the three things a `BOOTED` claim
/// rests on.
#[test]
fn the_boot_evidence_gate_still_checks_trackedness_emptiness_and_the_rev_stamp() {
    let p = bench_dir().join("assert_boot_evidence.sh");
    let src = std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("★ {} is gone: {e}", p.display()));
    for (needle, why) in [
        (
            "git ls-files --others",
            "evidence copied into the tree and never committed — `git commit -- <path>` omits \
             untracked files and EXITS 0",
        ),
        (
            "is EMPTY",
            "a zero-byte log's existence reads as a capture and is not one",
        ),
        (
            "kayfabe-rev:",
            "a boot with no revision stamp cannot be cited against a commit",
        ),
    ] {
        assert!(
            src.contains(needle),
            "★ `assert_boot_evidence.sh` no longer checks for `{needle}` — {why}"
        );
    }
    assert!(
        is_executable(&p),
        "★ {} is not executable, so the one gate over committed boot evidence cannot be run",
        p.display()
    );
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}
