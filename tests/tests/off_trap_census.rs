//! ★★★★★ **w323 — THE CENSUS THAT GUARDS THE MINT, and it is the half a type cannot do.**
//!
//! `crates/kayfabe-isolate/tests/ui/*.rs` pins that a host RM verb cannot be issued without
//! a [`kayfabe_util::trapwitness::OffTrap`], that one cannot be *named* into existence, and
//! that one cannot be *carried* to another thread. That is the boundary.
//!
//! The mint is the other half, and this file is it. Rust's privacy unit is the crate, and
//! more to the point [`OffTrap::at_a_host_verb`] is deliberately public: it takes the honest
//! branch (`claim` off-trap, a **counted** `inline_under_bql` on a trap thread) so that the
//! gate could be landed without breaking every boot on day one. ⇒ **while it exists, the
//! number of places it is called IS the surface area of the exemption**, and a surface area
//! nobody counts grows.
//!
//! # ★★★ THE NUMBER THIS CAMPAIGN IS DRIVING TO ZERO
//!
//! Two numbers, and they answer different questions:
//!
//! | number | question | where |
//! |---|---|---|
//! | **mint sites** (this file) | how many places *may* declare an inline host verb | source |
//! | `trapwitness::inline_exceptions()` | how many *actually did*, this boot | runtime |
//!
//! ⊘ Neither implies the other, which is why both exist. A boot with three mint sites and
//! `inline_exceptions=0` has proved the whole host-verb path ran off the trap **on that
//! workload**; the source count says how much room there is for it not to.
//!
//! # ⊘ WHAT THIS CANNOT SEE — read before trusting a green run
//!
//! It is a text scan. It sees the spelling `at_a_host_verb` and `inline_under_bql`. A site
//! that obtains a witness some other way — a helper wrapping the helper, a re-export under a
//! new name — is invisible to it, and that is the exact blind spot `unranked_locks.rs` had
//! on `Arc<Mutex<..>>` (**nine** vCPU-path locks unclassified while the gate reported zero).
//! ⇒ The known-positive fixtures below feed the scanner synthetic violations *in the shapes
//! it must catch*, so a future blind spot fails **naming the shape** rather than shortening
//! the list silently.

use std::path::{Path, PathBuf};

/// The workspace root, from cargo rather than the CWD (a test's CWD is the package root
/// today and that is not a promise).
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tests/ has a parent")
        .to_path_buf()
}

/// Strip `//` line comments and `/* */` blocks, so a doc comment that *mentions* the helper
/// is not counted as a call of it. ⊘ Deliberately naive about string literals; none of the
/// patterns below appears inside one, and a cleverer stripper is more code to be wrong in.
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let b = src.as_bytes();
    let (mut i, mut depth) = (0usize, 0usize);
    while i < b.len() {
        if depth == 0 && b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i..].starts_with(b"/*") {
            depth += 1;
            i += 2;
        } else if depth > 0 && b[i..].starts_with(b"*/") {
            depth -= 1;
            i += 2;
        } else {
            if depth == 0 {
                out.push(b[i] as char);
            }
            i += 1;
        }
    }
    out
}

/// Every `.rs` under `crates/`, excluding the module that DEFINES the helpers (a definition
/// is not a use) and excluding `tests/` trees (a test may legitimately mint one: it is
/// never inside a trap).
fn production_sources(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if matches!(name, "target" | "tests" | "wip" | "benches" | "examples") {
                    continue;
                }
                stack.push(p);
            } else if p.extension().and_then(|s| s.to_str()) == Some("rs")
                && !p.ends_with("kayfabe-util/src/trapwitness.rs")
            {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Count occurrences of each mint spelling in one file's comment-stripped text.
fn mints_in(text: &str) -> usize {
    text.matches("OffTrap::at_a_host_verb").count()
        + text.matches("OffTrap::inline_under_bql").count()
}

/// ★★★★★ **THE ENUMERATED SET.** Every production site that may hand a host verb a witness
/// on a possibly-trapping thread, with the reason it is still there.
///
/// ⚠ Adding a row is a design decision with a number attached. **Removing** one is the
/// campaign's progress: when publication runs on the worker, the first two rows below take
/// an honest `OffTrap::claim` instead and disappear from this list.
const DECLARED_MINT_SITES: &[(&str, usize, &str)] = &[
    (
        "crates/kayfabe-rt/src/device.rs",
        1,
        "`SharedDevice::verb_op`'s execute phase — THE production host-verb issue point. \
         Today it is reached from `SharedDoorbell::ring` on the vCPU inside the MMIO trap, \
         so it reports `inline_under_bql`; once the publication lane is armed the same line \
         runs on the worker and reports `claim`. This is the row w323 exists to retire.",
    ),
    (
        "crates/kayfabe-fwd/src/lib.rs",
        2,
        "`round_trip`'s execute phase, and `dispose_on`'s. ⊘ The second is the REVOCATION \
         direction and is NOT a candidate for deferral at all — a deferred unmap is a leak \
         window (owner ruling 2026-08-14). It is tier 2 by design, bounded by w317's \
         budget, and `kayfabe_device::pubqueue` refuses it at compile time. Expect this row \
         to survive the campaign.",
    ),
    (
        "crates/kayfabe-core/src/gpu.rs",
        1,
        "`Proc::drop`'s staged-release chain — the REVOCATION direction, valid→invalid. Same \
         ruling as `dispose_on`: a deferred unmap is a leak window, not a latency choice, so \
         this is deliberately synchronous and bounded by w317's budget. Expect this row to \
         survive the campaign.",
    ),
];

/// ★★★★★ **The census.** The set of production mint sites is exactly the declared set, both
/// directions.
#[test]
fn every_production_site_that_may_mint_an_inline_witness_is_declared() {
    let root = workspace_root();
    let mut actual: Vec<(String, usize)> = Vec::new();
    for p in production_sources(&root) {
        let Ok(src) = std::fs::read_to_string(&p) else {
            continue;
        };
        let n = mints_in(&strip_comments(&src));
        if n > 0 {
            let rel = p
                .strip_prefix(&root)
                .expect("under the root")
                .to_string_lossy()
                .replace('\\', "/");
            actual.push((rel, n));
        }
    }
    actual.sort();

    let mut declared: Vec<(String, usize)> = DECLARED_MINT_SITES
        .iter()
        .map(|(f, n, _)| ((*f).to_string(), *n))
        .collect();
    declared.sort();

    assert!(
        !actual.is_empty(),
        "★ THE FLOOR: the scanner found ZERO mint sites, which is a scanner failure and not \
         a clean tree — the production host-verb path demonstrably issues verbs. See this \
         file's 'what this cannot see'."
    );
    assert_eq!(
        actual, declared,
        "\nthe set of production sites that may mint an INLINE trap witness has changed.\n\
         Each one is a place a host RM verb may run with the QEMU BQL held — freezing every \
         vCPU and QEMU's main loop, not just the ringing one.\n\
         ⇒ If you ADDED one: say in `DECLARED_MINT_SITES` why the work beneath it is \
         bounded and guest-independent (`INLINE-SAFE` clauses (a) and (b)).\n\
         ⇒ If you REMOVED one: that is the campaign finishing. Delete its row.\n"
    );
}

/// ★★★ **The known-positive.** A census zero needs one, and a census *equality* needs one
/// just as much: the scanner must be shown catching each spelling it claims to cover.
#[test]
fn the_scanner_catches_every_spelling_a_mint_has() {
    // Both helper names, and both fully-qualified and imported forms.
    for shape in [
        "let off = kayfabe_util::trapwitness::OffTrap::at_a_host_verb(\"x\");",
        "let off = OffTrap::at_a_host_verb(\"x\");",
        "let off = kayfabe_util::trapwitness::OffTrap::inline_under_bql(\"x\");",
        "let off = OffTrap::inline_under_bql(\"x\");",
    ] {
        assert_eq!(
            mints_in(&strip_comments(shape)),
            1,
            "the scanner is blind to this spelling of a mint: {shape}"
        );
    }
    // …and the negative controls, so it is not simply matching everything.
    for benign in [
        "// OffTrap::at_a_host_verb is described here but not called",
        "/* OffTrap::inline_under_bql */",
        "let off = OffTrap::claim(\"honest, off a worker\");",
        "off.still_off_trap(\"the door\");",
    ] {
        assert_eq!(
            mints_in(&strip_comments(benign)),
            0,
            "the scanner counted something that is not a mint: {benign}"
        );
    }
}

/// ★★ **Every declared row carries a REASON, and the reason says something.**
///
/// Copied from `unranked_locks.rs`' fourth test, which exists because a classification
/// column filled with `"ok"` is a list that passes review and answers nothing.
#[test]
fn every_declared_mint_site_states_why_it_is_still_inline() {
    for (file, n, why) in DECLARED_MINT_SITES {
        assert!(*n > 0, "{file}: a declared row with count 0 declares nothing");
        assert!(
            why.len() > 80,
            "{file}: the reason is too short to be a reason ({} chars)",
            why.len()
        );
        assert!(
            why.contains("bounded")
                || why.contains("worker")
                || why.contains("REVOCATION")
                || why.contains("revocation"),
            "{file}: a reason must say either what bounds the work, or that it moves to a \
             worker, or that it is the revocation direction which may not be deferred"
        );
    }
}

/// ⊘ **The runtime counters exist and are readable** — so a boot can print the ratio rather
/// than a reader inferring it from the source count.
#[test]
fn the_runtime_census_names_both_numbers() {
    let line = kayfabe_util::trapwitness::census();
    assert!(line.contains("off_trap_claims="), "{line}");
    assert!(line.contains("inline_exceptions="), "{line}");
    assert!(line.contains("worst_trap="), "{line}");
}
