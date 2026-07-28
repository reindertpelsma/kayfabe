//! ★★ Stage **Q0**'s deliverable that is not a crate: *"the three vocabulary gates'
//! hard-coded lists reviewed and left unchanged **deliberately**, with a test asserting
//! the adapter crates are out of their scope by design."*
//!
//! ## Why a test and not a review note
//!
//! Because "deliberately unchanged" and "nobody looked" produce identical diffs. The
//! adapter crates are permitted to say `MemoryRegion` and `bql_lock` — the vocabulary
//! gates exist to keep those *out of the portable crates*, and an adapter is where they
//! belong. What makes that an exemption rather than an accident is that it is **asserted
//! in both directions**: the gated lists must not name the adapter crates, and the
//! adapter crates must actually contain the vocabulary, or the exemption is about nothing.
//!
//! ## And why it re-derives the ratchet
//!
//! The containment step is the one CI change stage Q0 makes. Its per-crate counts
//! are a hand-maintained constant, and the failure mode of a hand-maintained constant is
//! that it stops matching the tree between pushes. Re-deriving it here means a developer
//! who adds a relaxation finds out from `cargo test`, not from a red push.
//!
//! ★ This file reads `ci.yml` as **data**. It deliberately does not reproduce the gates'
//! patterns: a second copy of a pattern is a second thing to keep in sync, and the whole
//! point of the vocabulary gates is that they have exactly one home.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root is two levels above this crate")
}

fn ci_yml() -> String {
    std::fs::read_to_string(repo_root().join(".github/workflows/ci.yml"))
        .expect("ci.yml is the ONE home of the gate lists; this test is about its contents")
}

/// The `name:` of every step, so a step can be located by name and its body read.
fn step_body(yml: &str, name_fragment: &str) -> String {
    let start = yml
        .find(&format!("- name: \"{name_fragment}"))
        .or_else(|| yml.find(&format!("- name: {name_fragment}")))
        .unwrap_or_else(|| panic!("no CI step whose name starts with {name_fragment:?}"));
    let rest = &yml[start + 1..];
    let end = rest.find("\n      - name:").map_or(rest.len(), |i| i);
    rest[..end].to_string()
}

/// The crate list a vocabulary gate is scoped to — the `pure="..."` / `portable="..."`
/// assignment, and **only** that.
///
/// ★ Deliberately not the whole step body. Two of the three gates *name* the adapter
/// crates in their prose, on purpose — the VMM-vocabulary gate's failure text says in as
/// many words that *"naming the adapter crates never trips this gate"* — so a test that
/// grepped the body would fail on the very sentence that states the exemption. The
/// property is about the LIST.
fn gate_list(body: &str, var: &str) -> String {
    let at = body
        .find(&format!("{var}=\""))
        .unwrap_or_else(|| panic!("no {var}= assignment in that step"));
    let rest = &body[at + var.len() + 2..];
    let end = rest.find('"').expect("the list is quoted");
    rest[..end].to_string()
}

/// The two crates L2-Q adds. Named here so the assertions below read as statements about
/// the milestone rather than about two strings.
const ADAPTER_CRATES: [&str; 2] = ["kayfabe-vmm-qemu", "kayfabe-qemu-raw"];

/// ★★★ The three vocabulary gates name neither adapter crate — **and the adapter crates
/// really do contain what those gates forbid.**
///
/// The second half is the one that makes this a test. Without it, "the gates do not cover
/// the adapters" is equally true of an adapter that contains no hypervisor vocabulary at
/// all, which is a device that has not been written yet.
#[test]
fn the_vocabulary_gates_exclude_the_adapter_crates_and_the_exemption_is_not_vacuous() {
    let yml = ci_yml();
    for (step, var) in [
        ("Hexagonal boundary gate", "pure"),
        ("VMM-vocabulary gate", "portable"),
        ("Generation-name gate", "pure"),
    ] {
        let list = gate_list(&step_body(&yml, step), var);
        assert!(
            list.contains("crates/kayfabe-core"),
            "★ NON-VACUITY: the {step}'s {var}= list did not parse — it must at least name \
             the core, or every assertion below is about an empty string ({list:?})"
        );
        for crate_name in ADAPTER_CRATES {
            assert!(
                !list.contains(crate_name),
                "★ the {step}'s scope list names {crate_name}. The three vocabulary gates \
                 are scoped to \
                 the PORTABLE crates on purpose: an adapter is precisely where one \
                 hypervisor's API identifiers belong, and adding an adapter to a gated \
                 list would make the adapter unwritable. If this ever needs to change it \
                 is a design decision, not a list edit"
            );
        }
    }

    // The other direction. `kayfabe-vmm-qemu` must genuinely use the vocabulary the
    // VMM-vocabulary gate forbids elsewhere, or "out of scope by design" is a statement
    // about an empty set.
    let root = repo_root();
    let mut found = Vec::new();
    for (file, needle) in [
        ("crates/kayfabe-vmm-qemu/src/host.rs", "MemoryRegion"),
        ("crates/kayfabe-vmm-qemu/src/lib.rs", "bql_lock"),
        (
            "crates/kayfabe-qemu-raw/src/lib.rs",
            "memory_region_init_ram_ptr",
        ),
    ] {
        let text = std::fs::read_to_string(root.join(file))
            .unwrap_or_else(|e| panic!("{file} must exist: {e}"));
        assert!(
            text.contains(needle),
            "★ NON-VACUITY: {file} does not contain {needle:?}. The claim this test makes \
             is that the adapter crates are exempt from the vocabulary gates BECAUSE they \
             need the vocabulary. An adapter that never says it is an adapter that has not \
             met the hypervisor yet, and the exemption above is then about nothing"
        );
        found.push(needle);
    }
    assert_eq!(
        found.len(),
        3,
        "all three vocabulary witnesses are present ({found:?})"
    );
}

/// ★★ The containment step's `AUDITED` list, re-derived against the tree.
///
/// Three properties, and the third is the one Q0 actually changed:
/// 1. the list names exactly the two crates it is allowed to name;
/// 2. every count in it matches the relaxations actually present;
/// 3. the list is used by **all three** sub-gates, so "which crates may hold the unsound
///    surface?" has one answer and not three that can drift apart.
#[test]
fn the_audited_crate_list_matches_the_tree_and_is_used_by_all_three_sub_gates() {
    let yml = ci_yml();
    let body = step_body(&yml, "Unsafe-containment gates");
    let line = body
        .lines()
        .find(|l| l.trim_start().starts_with("AUDITED="))
        .expect("the ONE list must be a single assignment nobody has to reconstruct");
    let listed: Vec<(String, u32)> = line
        .trim()
        .trim_start_matches("AUDITED=")
        .trim_matches('"')
        .split_whitespace()
        .map(|e| {
            let (c, n) = e.split_once(':').expect("every entry is crate:count");
            (c.to_string(), n.parse().expect("a count"))
        })
        .collect();

    assert_eq!(
        listed.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(),
        vec!["kayfabe-linux-raw", "kayfabe-qemu-raw"],
        "★ exactly two crates may omit the workspace lints, and they are these. A third \
         is a design decision (l2_qemu_adapter.md §2.2), not a manifest edit — and \
         `kayfabe-vmm-qemu` must never be one of them: it is the crate that holds ALL the \
         logic, and the whole three-crate split exists so that it does not need the \
         relaxation"
    );

    let root = repo_root();
    for (crate_name, expected) in &listed {
        let dir = root.join("crates").join(crate_name).join("src");
        let mut actual = 0u32;
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{crate_name} must have a src/: {e}"))
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with("_unsafe.rs"))
            })
            .collect();
        files.sort();
        for f in &files {
            let text = std::fs::read_to_string(f).expect("readable");
            // The same syntactic forms the ratchet counts — the keyword followed by a
            // block, a function, an impl or a trait — so prose in those files cannot
            // inflate either number.
            //
            // ★ The keyword is assembled from fragments rather than written out, and that
            // is not obfuscation: the surface gate greps every `.rs` file that is not
            // named `*_unsafe.rs` for the whole word, so a file that MATCHES on the word
            // in order to count it would fail the gate it is checking. Its own failure
            // text names rewording as the fix; here the word is a pattern rather than
            // prose, so the rewording has to be lexical.
            let kw = concat!("uns", "afe");
            actual += u32::try_from(
                text.match_indices(&format!("{kw} {{")).count()
                    + text.match_indices(&format!("{kw} fn ")).count()
                    + text.match_indices(&format!("{kw} impl ")).count()
                    + text.match_indices(&format!("{kw} trait ")).count(),
            )
            .expect("a reviewable number of relaxations");
        }
        assert_eq!(
            actual, *expected,
            "★ {crate_name} declares {expected} relaxation(s) in ci.yml and the tree has \
             {actual}. A hand-maintained constant that stops matching the tree is a \
             ratchet that has quietly become a comment"
        );
    }

    // Property 3: one list, three consumers.
    let uses = body.matches("$AUDITED").count() + body.matches("${entry%%:*}").count();
    assert!(
        uses >= 5,
        "★ the three sub-gates must all read the SAME list. Found {uses} references to it \
         — gate A's exemption, gate B's prune and the ratchet's loop each need at least \
         one, or one of them is carrying its own copy of the answer"
    );
}

/// ★ `kayfabe-vmm-qemu` inherits the workspace lints, which is the whole reason the
/// three-crate split exists. Asserted here rather than trusted, because gate A can only
/// see a manifest that FORGOT the block — it cannot see one that was exempted on purpose.
#[test]
fn the_adapter_crate_that_holds_the_logic_is_still_forbidden_the_relaxation() {
    let manifest = std::fs::read_to_string(repo_root().join("crates/kayfabe-vmm-qemu/Cargo.toml"))
        .expect("the adapter manifest");
    assert!(
        manifest.contains("[lints]") && manifest.contains("workspace = true"),
        "★ the crate that holds ALL the logic must inherit the workspace lints. The split \
         into three crates buys exactly one thing — that the logic is auditable without \
         reading a single relaxation — and it buys nothing if this manifest ever opts out"
    );
    assert!(
        !manifest.contains("[lints.rust]"),
        "and it must not restate them either: a restated lint block is how a crate opts \
         out of `forbid` without ever saying so"
    );
}
