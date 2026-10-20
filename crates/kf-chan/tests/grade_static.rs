//! 2026-10-20 -- **Grader groups G9 (structural / static) and G11 (the failure catalogue)**
//! (`docs/design/V3_RELAY_GRADER_20261020.md`, `docs/design/V3_FAILURE_CATALOGUE.md`). Source audits, std only.
//!
//! * G9a: the wake-source registry (`tests/wake_registry.rs`) plus: every wrapper of `RelayLoop` forwards EVERY method.
//! * G9b: banned heuristics in the worker / relay modules (allowlist `scripts/relay_grade/heuristics_allowlist.txt`).
//! * G9c: blocking calls reachable from the worker (allowlist `scripts/relay_grade/blocking_allowlist.txt`, folding the findings of
//!   `docs/design/V3_BLOCKING_AUDIT_20261020.md`, branch `claude/blocking-audit-20261020`).
//! * G9d: `unsafe` only in `*_unsafe.rs` of `kf-linux-raw`, `kf-qemu`, `kf-cuda` (allowlist `scripts/relay_grade/unsafe_allowlist.txt`).
//! * G9e: the adapter's pins (`grade_adapter.rs` `PINS`) still match the glue they transliterate.
//! * G11: every catalogue entry (`scripts/relay_grade/catalogue.toml`) names a pinning test that exists, or a hardware gate.
#![allow(dead_code, missing_docs)]

#[path = "grade_adapter.rs"]
mod adapter;

use adapter::{read, root};

/// The production part of a Rust source: everything before the first `#[cfg(test)]`, comment lines and trailing comments removed.
fn production_lines(src: &str) -> Vec<(usize, String)> {
    let prod = src.split("#[cfg(test)]").next().unwrap_or("");
    prod.lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let t = l.trim();
            if t.starts_with("//") || t.is_empty() {
                return None;
            }
            let code = match t.find(" //") {
                Some(k) => &t[..k],
                None => t,
            };
            Some((i + 1, code.to_string()))
        })
        .collect()
}

/// An allowlist: lines `file | substring | status | why`; `#` comments.
fn allowlist(rel: &str) -> Vec<(String, String, String)> {
    read(rel)
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split('|').map(str::trim).collect();
            (f.len() >= 3).then(|| (f[0].to_string(), f[1].to_string(), f[2].to_string()))
        })
        .collect()
}

fn audit(files: &[&str], patterns: &[(&str, &str)], allow_rel: &str) -> (Vec<String>, Vec<String>) {
    let allow = allowlist(allow_rel);
    let mut hits = Vec::new();
    let mut allowed = Vec::new();
    for f in files {
        let src = read(f);
        assert!(!src.is_empty() || !f.contains("kf-chan"), "{f} unreadable");
        let mut cur_fn = String::new();
        for (n, line) in production_lines(&src) {
            if let Some(i) = line.find("fn ") {
                let head = &line[..i];
                let h = head.trim();
                if !h.contains('=') && (h.is_empty() || h.starts_with("pub") || matches!(h, "const" | "async" | "unsafe")) {
                    cur_fn = line[i + 3..].split(|c: char| c == '(' || c == '<').next().unwrap_or("").to_string();
                }
            }
            for (pat, what) in patterns {
                if line.contains(pat) {
                    // an allowlist substring `fn:<name> <text>` matches only inside that function
                    let matches = |sub: &str| match sub.strip_prefix("fn:") {
                        Some(rest) => {
                            let (name, text) = rest.split_once(' ').unwrap_or((rest, ""));
                            name == cur_fn && line.contains(text)
                        }
                        None => line.contains(sub),
                    };
                    let ok = allow.iter().find(|(af, sub, _)| f.ends_with(af.as_str()) && matches(sub));
                    match ok {
                        Some((_, _, status)) => allowed.push(format!("{f}:{n} (fn {cur_fn}) [{status}] {what}: {line}")),
                        None => hits.push(format!("{f}:{n} (fn {cur_fn}) {what}: {line}")),
                    }
                }
            }
        }
    }
    (hits, allowed)
}

// ---------------------------------------------------------------------------------------------------------------- G9a

/// G9a: every type that implements `RelayLoop` by wrapping another (`Watched`, the progress diagnostic) forwards EVERY method of
/// the trait: a wrapper that falls back to a default (`unsourced() -> false`) silently changes the block decision whenever the
/// diagnostic is on (`KF3_DIAG_PROGRESS`, which the hardware plan runs).
#[test]
fn g9_watched_forwards_every_relayloop_method() {
    let src = read("crates/kf-chan/src/worker.rs");
    let prod = src.split("#[cfg(test)]").next().unwrap();
    let methods = |block: &str| -> Vec<String> {
        block
            .lines()
            .filter_map(|l| {
                let t = l.trim();
                t.strip_prefix("fn ")
                    .and_then(|r| r.split('(').next())
                    .map(str::to_string)
            })
            .collect()
    };
    let body = |start: &str| -> &str {
        let a = prod.find(start).unwrap_or_else(|| panic!("{start} not found"));
        let rest = &prod[a..];
        let mut depth = 0i32;
        for (i, c) in rest.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &rest[..i];
                    }
                }
                _ => {}
            }
        }
        rest
    };
    let trait_methods = methods(body("pub trait RelayLoop"));
    assert!(trait_methods.len() >= 2, "{trait_methods:?}");
    let mut missing = Vec::new();
    for imp in ["impl RelayLoop for Watched"] {
        if !prod.contains(imp) {
            continue;
        }
        let m = methods(body(imp));
        for t in &trait_methods {
            if !m.contains(t) {
                missing.push(format!("{imp} does not forward `{t}` (the trait default answers instead of the wrapped relay)"));
            }
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

// ---------------------------------------------------------------------------------------------------------------- G9b

/// The banned heuristics (owner, 2026-10-20: no retry budgets, no every-Nth-pass looks, no timers as logic, no modulus counters in
/// look decisions). Each pattern is conservative: it may hit a legitimate line, which then goes to the allowlist WITH a reason.
const HEURISTICS: &[(&str, &str)] = &[
    ("RETRIES", "a retry budget constant"),
    ("may_retry", "a retry budget decision"),
    ("contended_run()", "a contended-run count read by a decision"),
    ("LOOK_EVERY", "an every-Nth-pass look"),
    ("busy_passes", "a pass counter gating looks"),
    ("Peek::Backstop", "a timer-driven look (the net as logic)"),
    ("0..user_loop::SPIN", "a bounded pre-park spin (a budget)"),
    ("thread::sleep", "a sleep"),
    ("park_timeout", "a timed park"),
    ("PollTimeout::Millis", "a timed wait"),
    (" % ", "a modulus (every-Nth gating)"),
    ("Instant::now", "a clock read on the loop"),
];

/// G9b: no banned heuristic in the worker loop, the decision vocabulary or the glue, except the allowlisted lines.
#[test]
fn g9_no_banned_heuristics() {
    let mut files: Vec<&str> = adapter::LOOP_FILES.to_vec();
    files.extend_from_slice(adapter::GLUE_FILES);
    let (hits, allowed) = audit(&files, HEURISTICS, "scripts/relay_grade/heuristics_allowlist.txt");
    for a in &allowed {
        println!("ALLOWED {a}");
    }
    assert!(hits.is_empty(), "{} banned heuristic(s):\n{}", hits.len(), hits.join("\n"));
}

// ---------------------------------------------------------------------------------------------------------------- G9c

const BLOCKING: &[(&str, &str)] = &[
    (".lock()", "a blocking Mutex::lock"),
    (".read()", "a blocking RwLock::read"),
    (".write()", "a blocking RwLock::write"),
    ("thread::sleep", "a sleep"),
    (".wait(", "a condvar / blocking wait"),
    ("ioctl", "an ioctl"),
    ("recv()", "a blocking channel receive"),
];

/// G9c: every blocking call in the worker-reachable modules is allowlisted with its audit status.
#[test]
fn g9_blocking_calls_are_audited() {
    let mut files: Vec<&str> = adapter::LOOP_FILES.to_vec();
    files.extend_from_slice(adapter::GLUE_FILES);
    let (hits, allowed) = audit(&files, BLOCKING, "scripts/relay_grade/blocking_allowlist.txt");
    let flagged: Vec<&String> = allowed.iter().filter(|a| a.contains("[VIOLATION") || a.contains("[SUSPECT")).collect();
    for a in &flagged {
        println!("AUDIT {a}");
    }
    assert!(hits.is_empty(), "{} un-audited blocking call(s):\n{}", hits.len(), hits.join("\n"));
}

// ---------------------------------------------------------------------------------------------------------------- G9d

fn rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// G9d: `unsafe` code only in files named `*_unsafe.rs` of `kf-linux-raw`, `kf-qemu` and `kf-cuda` (CLAUDE.md); other hits must be
/// allowlisted with a reason.
#[test]
fn g9_unsafe_only_in_allowed_files() {
    let allow = allowlist("scripts/relay_grade/unsafe_allowlist.txt");
    let crates = root().join("crates");
    let mut bad = Vec::new();
    let Ok(rd) = std::fs::read_dir(&crates) else { panic!("crates/") };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with("kf-") {
            continue;
        }
        let mut files = Vec::new();
        rs_files(&e.path().join("src"), &mut files);
        for f in files {
            let src = std::fs::read_to_string(&f).unwrap_or_default();
            let uses = src.lines().any(|l| {
                let t = l.trim_start();
                !t.starts_with("//")
                    && (t.contains("unsafe {") || t.contains("unsafe fn ") || t.contains("unsafe impl ") || t.starts_with("unsafe extern"))
            });
            if !uses {
                continue;
            }
            let rel = f.strip_prefix(root()).unwrap_or(&f).to_string_lossy().to_string();
            let allowed_crate = matches!(name.as_str(), "kf-linux-raw" | "kf-qemu" | "kf-cuda");
            if allowed_crate && rel.ends_with("_unsafe.rs") {
                continue;
            }
            if allow.iter().any(|(af, _, _)| rel.ends_with(af.as_str())) {
                continue;
            }
            bad.push(rel);
        }
    }
    assert!(bad.is_empty(), "unsafe outside the allowed files:\n{}", bad.join("\n"));
}

// ---------------------------------------------------------------------------------------------------------------- G9e

/// G9e: the lines of the glue the adapter transliterates are still there (a drift makes the grader grade a stale mirror).
#[test]
fn g9_adapter_pins_match_the_glue() {
    let mut missing = Vec::new();
    for (f, pin) in adapter::PINS {
        if !read(f).contains(pin) {
            missing.push(format!("{f}: {pin}"));
        }
    }
    assert!(missing.is_empty(), "{}: the adapter's mirror drifted:\n{}", adapter::LINEAGE, missing.join("\n"));
}

// ---------------------------------------------------------------------------------------------------------------- G11

#[derive(Debug, Default)]
struct Entry {
    id: String,
    tests: Vec<String>,
    gate: String,
    status: String,
}

/// The catalogue's `[[entry]]` tables (a minimal reader of the subset the catalogue uses: strings and string arrays).
fn catalogue() -> Vec<Entry> {
    let src = read("scripts/relay_grade/catalogue.toml");
    assert!(!src.is_empty(), "scripts/relay_grade/catalogue.toml missing");
    let mut out = Vec::new();
    for chunk in src.split("[[entry]]").skip(1) {
        let mut e = Entry::default();
        let mut key = String::new();
        let mut in_array = false;
        for line in chunk.lines() {
            let t = line.trim();
            if t.starts_with('#') {
                continue;
            }
            if in_array {
                if t.starts_with(']') {
                    in_array = false;
                    continue;
                }
                let v = t.trim_end_matches(',').trim_matches('"').to_string();
                if key == "tests" && !v.is_empty() {
                    e.tests.push(v);
                }
                continue;
            }
            let Some((k, v)) = t.split_once('=') else { continue };
            key = k.trim().to_string();
            let v = v.trim();
            if v == "[" {
                in_array = true;
                continue;
            }
            if v.starts_with('[') {
                let inner = v.trim_start_matches('[').trim_end_matches(']');
                if key == "tests" {
                    e.tests.extend(
                        inner
                            .split("\",")
                            .map(|x| x.trim().trim_matches(',').trim().trim_matches('"').to_string())
                            .filter(|x| !x.is_empty()),
                    );
                }
                continue;
            }
            let s = v.trim_matches('"').to_string();
            match key.as_str() {
                "id" => e.id = s,
                "hardware_gate" => e.gate = s,
                "status" => e.status = s,
                _ => {}
            }
        }
        out.push(e);
    }
    out
}

/// Whether `crate:bin:fn` names a function that exists in the tree.
fn test_exists(spec: &str) -> bool {
    let parts: Vec<&str> = spec.split(':').collect();
    if parts.len() != 3 {
        return false;
    }
    let (krate, bin, f) = (parts[0], parts[1], parts[2]);
    let needle = format!("fn {f}(");
    let dir = root().join("crates").join(krate);
    let mut files = Vec::new();
    if bin == "lib" {
        rs_files(&dir.join("src"), &mut files);
    } else {
        files.push(dir.join("tests").join(format!("{bin}.rs")));
        rs_files(&dir.join("tests").join(bin), &mut files);
    }
    files
        .iter()
        .any(|p| std::fs::read_to_string(p).is_ok_and(|s| s.contains(&needle)))
}

/// G11: every catalogued failure mode names a GPU-free test that exists in this tree, or a hardware / host / process gate.
#[test]
fn g11_catalogue_every_entry_is_pinned_or_gated() {
    let entries = catalogue();
    assert!(entries.len() >= 50, "the catalogue lost entries: {}", entries.len());
    let mut bad = Vec::new();
    let (mut pinned, mut gated, mut open) = (0, 0, 0);
    for e in &entries {
        let existing: Vec<&String> = e.tests.iter().filter(|t| test_exists(t)).collect();
        let missing: Vec<&String> = e.tests.iter().filter(|t| !test_exists(t)).collect();
        if !missing.is_empty() {
            bad.push(format!("{}: cited test(s) not found: {missing:?}", e.id));
        }
        if existing.is_empty() && e.gate.is_empty() {
            bad.push(format!("{}: no pinning test and no hardware gate", e.id));
        }
        match e.status.as_str() {
            "open" => open += 1,
            _ if !existing.is_empty() => pinned += 1,
            _ => gated += 1,
        }
    }
    println!(
        "CATALOGUE entries={} pinned={pinned} hw/process-gated={gated} open={open}",
        entries.len()
    );
    assert!(bad.is_empty(), "{} catalogue problem(s):\n{}", bad.len(), bad.join("\n"));
}

/// G11: ids are unique and every entry states an expected behaviour.
#[test]
fn g11_catalogue_ids_unique_and_expected_stated() {
    let src = read("scripts/relay_grade/catalogue.toml");
    let entries = catalogue();
    let mut ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    ids.sort_unstable();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate catalogue ids");
    assert_eq!(src.matches("\nexpected = \"").count(), n, "every entry states one expected behaviour");
}
