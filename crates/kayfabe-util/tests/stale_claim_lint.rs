//! ★★★★★ **w755k — THE STALE-CLAIM LINT: a reachability claim must carry a gate or a date.**
//!
//! > Owner, 2026-09-17: *"the thing I am most worried about is false old claims in those 120
//! > thousand line comments"*
//!
//! The worry is right, and 2026-09-16/17 produced three instances in one day, each **true when
//! written**:
//!
//! | claim | true at | false at | what it cost |
//! |---|---|---|---|
//! | *"a child backend cannot produce `PlacementRefused`"* | #102 | HEAD | the wall was unreadable for two boots |
//! | *"an identity window is an 11.8 GiB allocation"* | 02:23 | 08:17 **the same day** | the walk differential is foreclosed |
//! | *"a silently failed unmap here…"* | always | — | described a leak as hypothetical while nothing called the function |
//!
//! ⊘ **What they share is not being wrong — it is being UNMAINTAINED.** A claim about what
//! *cannot happen* is exactly the kind nothing re-checks, because no test fails when the world
//! changes underneath it.
//!
//! # The rule
//!
//! > A comment claiming something **cannot happen** must carry either a **citation**
//! > (`[measured …]`, `[ogkm …]`, `[source: …]`, `[C: …]`) or a **named gate** that enforces
//! > it. A bare *"X cannot happen"* is an assertion with no owner.
//!
//! # ⊘ Why this starts as a CENSUS and not a hard failure
//!
//! `[measured w755k]` the tree has thousands of such lines. A gate that fails the build on all
//! of them is a gate someone switches off on day one, which is worse than none. ⇒ it prints the
//! count and the ranking, and **fails only for crates listed in `PINNED`** — tightened one
//! crate at a time, exactly like `ci.yml`'s relaxation ratchet.
//!
//! ⚠ And that ratchet's own trap, learned today: a hand-maintained constant that stops matching
//! the tree is a comment. This one prints the real number on every run, so drift is visible
//! before it is a failure.

/// Phrasings that assert something about what CANNOT occur. ⊘ Deliberately narrow: it must find
/// the class that rots, not every confident sentence.
const CLAIM: &[&str] = &[
    "cannot ",
    "can never",
    "never happens",
    "unreachable",
    "impossible",
    "no caller",
    "is not reachable",
    "by construction",
];

/// What discharges a claim: a citation to something checkable, or a named gate.
const DISCHARGE: &[&str] = &[
    "[measured",
    "[source:",
    "[ogkm",
    "[c:",
    "[corroborated",
    "[campaign record]",
    "gate",
    "assert",
    "test ",
];

/// ⊘ Crates whose undischarged count is PINNED. Tighten one at a time; a crate absent here is
/// censused, not gated.
///
/// ★ `kayfabe-abi` is deliberately NOT pinned despite having the most claims: its claims are
/// transcription facts pinned by `const _: () = assert!(offset_of!(..) == N)`, so a false one
/// **breaks the build**. That is the safe kind — and the pattern the logic crates lack.
const PINNED: &[(&str, usize)] = &[];

fn crates_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/ is this crate's parent")
        .to_path_buf()
}

/// Every `(crate, file, line, text)` making an undischarged reachability claim.
fn undischarged() -> Vec<(String, String, usize, String)> {
    let root = crates_dir();
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(p);
                continue;
            }
            if !p.extension().is_some_and(|x| x == "rs") {
                continue;
            }
            // ⊘ `src` only. A test's comments describe the test, and the gate IS the discharge.
            if !p.components().any(|c| c.as_os_str() == "src") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let krate = p
                .strip_prefix(&root)
                .ok()
                .and_then(|r| {
                    r.components()
                        .next()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                })
                .unwrap_or_default();
            let lines: Vec<&str> = text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let t = line.trim_start();
                if !(t.starts_with("///") || t.starts_with("//!") || t.starts_with("//")) {
                    continue;
                }
                let low = line.to_lowercase();
                if !CLAIM.iter().any(|c| low.contains(c)) {
                    continue;
                }
                // ★ The discharge may sit a few lines either side — a citation usually
                // introduces the paragraph rather than the sentence.
                let lo = i.saturating_sub(4);
                let hi = (i + 5).min(lines.len());
                let window = lines[lo..hi].join("\n").to_lowercase();
                if DISCHARGE.iter().any(|d| window.contains(d)) {
                    continue;
                }
                out.push((
                    krate.clone(),
                    p.display().to_string(),
                    i + 1,
                    t.chars().take(150).collect(),
                ));
            }
        }
    }
    out
}

/// ★★★ The census. Always prints; fails only for crates in [`PINNED`].
#[test]
fn reachability_claims_carry_a_citation_or_a_gate() {
    let rows = undischarged();
    // ★ NON-VACUITY: a scan finding nothing has broken, not cleaned the tree.
    assert!(
        rows.len() > 50,
        "the stale-claim scan found only {} undischarged claims — the comment spellings this \
         lint matches have changed and it is now vacuous",
        rows.len()
    );

    let mut by_crate: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for (k, ..) in &rows {
        *by_crate.entry(k.clone()).or_default() += 1;
    }
    println!("STALE-CLAIM CENSUS — undischarged reachability claims, by crate");
    let mut ranked: Vec<_> = by_crate.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1));
    for (k, n) in &ranked {
        println!("    {n:5}  {k}");
    }
    println!("    {:5}  TOTAL", rows.len());
    println!(
        "⊘ DISCHARGED = carries a citation ([measured …]/[ogkm …]/[C: …]) or names the gate that \
         enforces it. Undischarged means nothing re-checks it when the world moves."
    );

    for (krate, expected) in PINNED {
        let actual = by_crate.get(*krate).copied().unwrap_or(0);
        assert!(
            actual <= *expected,
            "★★★ {krate} has {actual} undischarged reachability claims, pinned at {expected}. \
             Discharge the new one — cite a measurement, or name the gate — or it is an \
             assertion with no owner."
        );
    }
}

/// ★★ The worst offenders, printed with file and line so a reader can go straight there.
///
/// ⊘ A separate test so the census above stays readable, and capped so the output is a
/// starting point rather than a dump.
#[test]
fn the_worst_offenders_are_named_with_file_and_line() {
    let mut rows = undischarged();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    println!(
        "STALE-CLAIM SAMPLE — 40 of {}, alphabetical by crate",
        rows.len()
    );
    for (krate, file, line, text) in rows.iter().take(40) {
        let short = file.rsplit("crates/").next().unwrap_or(file);
        println!("  [{krate}] {short}:{line}\n      {text}");
    }
    assert!(!rows.is_empty(), "nothing to sample");
}
