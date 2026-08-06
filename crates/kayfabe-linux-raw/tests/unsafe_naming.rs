//! # ★★★ Files that are soundness-critical WITHOUT the keyword (`l1_os_shell.md` §4.2.1.1)
//!
//! Owner's axiom, 2026-08-06: *"the raw layer must assume safe code is bugged, broken, skips
//! validation, and it may still not violate. It facilitates extra functions to safe code, that
//! are still safe — not that safe code calls raw functions or handles pointers."*
//!
//! The filing consequence: **the keyword is not the boundary.** It marks five *operations* and
//! no *reasoning* at all, and Rust's model puts the soundness boundary at the **privacy
//! boundary** — safe code that can reach an abstraction's private fields is part of its safety
//! argument. So a file can be soundness-critical with none of the keyword in it — and the
//! keyword is what `ls`-based auditing can see.
//!
//! ## ⊘ What this file does NOT do, because something else already does it
//!
//! The keyword half — *every `.rs` file using the keyword is named `*_unsafe.rs`* — is already
//! gated, in `scripts/ci_gates.sh`'s **Unsafe-surface gate**, whose rationale is that an
//! auditor must be able to enumerate that whole surface with `ls`. `[measured]`
//! 2026-08-06: it holds exactly, 13 files each side.
//!
//! ⚠ A first cut of this file re-implemented that scan in Rust and **tripped that very gate on
//! its own detection strings** — a duplicate that also broke the thing it duplicated. It is
//! deleted rather than fixed. The scan below therefore matches no keyword text at all.
//!
//! ## What is genuinely missing, and is here
//!
//! That gate is a perfect score *against the keyword criterion*, which is precisely why it
//! cannot see the class above. It proves only that nobody hid the keyword under an innocent
//! name. The residue — **safe files carrying reasoning the raw layer depends on** — is
//! enumerated below by name.
//!
//! ⊘ **This is not an allowlist, and the distinction matters** because the shell gate says in
//! as many words *"never add an allowlist: the moment this gate needs a judgement call, `ls`
//! stops being the audit."* An allowlist would excuse a file from the naming rule. This list
//! does the opposite — it names files the rule must ALSO cover. It can only ever make the
//! surface larger, never smaller, so `ls` remains the audit.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Crates whose sources this gate walks — the two permitted to hold a raw surface at all.
const CRATES: &[&str] = &["kayfabe-linux-raw", "kayfabe-qemu-raw"];

/// ★★★ **The soundness-critical SAFE files** — files the `ls` audit cannot see, that
/// nevertheless carry reasoning a raw-surface block depends on, and therefore owe the
/// `_unsafe` suffix. §4.2.1.1 names the three qualifying constructs.
///
/// **It is empty, and empty is a CLAIM rather than an absence.** It asserts that no block in
/// this tree's raw layer trusts a value it did not re-derive itself. The worked standard is
/// `Region::word_at` (`src/mapping_unsafe.rs`): it re-checks alignment, then bounds the access
/// with `bounds::checked_span(self.map.len_bytes(), …)` — from the **mapping's own length**,
/// never from what the caller claimed. Safe code passing a wild offset gets a `RawError`.
///
/// ★ Which is exactly why offsets and lengths are *allowed* to be computed in safe files: the
/// rule is not *"never compute an offset in safe code"*, it is *"never let the raw layer trust
/// one"*. Owner, 2026-08-06: *"offset/length free to exist outside, as long as every use in
/// the raw layer revalidates it."*
///
/// ⊘ Adding a name here is a design decision, not a filing one. Prefer the better fix — make
/// the raw side re-validate, which empties the entry instead of decorating it.
const SOUNDNESS_CRITICAL_SAFE_FILES: &[&str] = &[];

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

/// Every `.rs` path under `<crate>/src`, repo-relative. Paths only — this gate never reads a
/// file's text, which is how it stays clear of the shell gate's pattern.
fn sources() -> BTreeSet<String> {
    let root = repo_root();
    let mut out = BTreeSet::new();
    for c in CRATES {
        walk(&root.join("crates").join(c).join("src"), &root, &mut out);
    }
    out
}

fn walk(dir: &Path, root: &Path, out: &mut BTreeSet<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, root, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.insert(
                p.strip_prefix(root)
                    .expect("under the repo root")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
}

// =====================================================================================

/// ★★★ Every declared soundness-critical safe file exists and carries the suffix.
///
/// ⚠ The list is empty today, so this test's *body* is vacuous — and that is stated rather
/// than hidden. What it buys is that **adding a name is a deliberate edit that must also
/// rename the file**: `gates_quantified_over_a_list`, where the failure mode is a list quietly
/// getting shorter. The companion test below is what stops the emptiness itself from rotting.
#[test]
fn every_declared_soundness_critical_safe_file_carries_the_unsafe_suffix() {
    let present = sources();
    for f in SOUNDNESS_CRITICAL_SAFE_FILES {
        assert!(
            present.contains(*f),
            "⊘ `{f}` is declared soundness-critical but does not exist — a phantom row makes \
             this gate quantify over a file nobody can read"
        );
        assert!(
            f.ends_with("_unsafe.rs"),
            "★★★ `{f}` is declared to carry reasoning a raw-surface block depends on, so it \
             owes the `_unsafe` suffix even though rustc compiled it without the keyword. ⊘ \
             Before renaming, try the better fix: make the raw side RE-VALIDATE, which removes \
             the row instead of decorating it"
        );
    }
}

/// ★★ The scan still finds the tree, so an empty list means *"nothing qualifies"* and not
/// *"the walker broke"*.
///
/// ⊘ Without this, deleting `CRATES` or breaking `walk` would leave the test above green while
/// it examined nothing — the same false green as a filter that matches no test name and still
/// reports `ok`.
#[test]
fn the_scan_still_sees_the_raw_crates_so_an_empty_list_means_nothing_qualifies() {
    let present = sources();
    assert!(
        present.len() > 20,
        "the walker found only {} sources under {CRATES:?} — it is broken, and a broken walker \
         makes the declared list vacuously satisfiable",
        present.len()
    );
    let named = present.iter().filter(|p| p.ends_with("_unsafe.rs")).count();
    assert!(
        named >= 10,
        "only {named} `*_unsafe.rs` files found — the raw surface cannot have shrunk that far, \
         so this is the instrument breaking rather than the tree improving"
    );
}
