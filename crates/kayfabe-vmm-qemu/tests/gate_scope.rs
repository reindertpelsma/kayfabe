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
        // ★ CHANGED AT Q2. This used to be `memory_region_init_ram_ptr`, the constructor
        // §5.4 had us hand a pointer to. `host_execution_plane.md` §1 replaced that whole
        // mechanism: the hypervisor RESERVES the window with `memory_region_init_io` and
        // backs nothing. The witness follows the mechanism — its job is to prove the
        // adapter crates really do speak the vocabulary the gates forbid elsewhere, and a
        // witness naming a constructor the design no longer calls would prove it about a
        // sentence nobody reads.
        (
            "crates/kayfabe-qemu-raw/src/lib.rs",
            "memory_region_init_io",
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

/// Which crates each vocabulary gate deliberately does **not** scope, and why.
///
/// ★★ This table encodes the tree as it is at 2026-07-29. It changes NOTHING about the
/// gates; its whole job is to make the next crate's classification a decision somebody
/// has to write down, instead of a silence.
///
/// One line per (gate, crate) exemption. If a reason reads wrong to you, that is the
/// point — it is now in one place and arguable, which it was not when it was the
/// difference between two hand-written lists 250 lines apart in a YAML file.
type Exemption = (&'static str, &'static str);
const GATE_EXEMPTIONS: &[Exemption] = &[
    // ── Hexagonal boundary gate (`pure`): may this crate name an OS readiness primitive?
    ("Hexagonal boundary gate", "kayfabe-rt"), // adapter; its own manifest says "deliberately outside the hexagonal-boundary grep"
    ("Hexagonal boundary gate", "kayfabe-shell"), // IS the L1 OS shell — naming epoll is its job
    ("Hexagonal boundary gate", "kayfabe-linux-raw"), // the one audited raw-OS crate
    ("Hexagonal boundary gate", "kayfabe-isolate-host"), // host adapter: spawns a child, opens /dev/nvidia*
    ("Hexagonal boundary gate", "kayfabe-vmm-kvm"),      // real KVM adapter
    ("Hexagonal boundary gate", "kayfabe-vmm-qemu"),     // ADAPTER_CRATES
    ("Hexagonal boundary gate", "kayfabe-qemu-raw"),     // ADAPTER_CRATES
    ("Hexagonal boundary gate", "kayfabe-crec"), // Axis-B arch adapter + trace replay; test-facing
    ("Hexagonal boundary gate", "kayfabe-mocks"), // test-only doubles
    // ── VMM-vocabulary gate (`portable`): may this crate name one hypervisor's API?
    ("VMM-vocabulary gate", "kayfabe-linux-raw"),
    ("VMM-vocabulary gate", "kayfabe-isolate-host"),
    ("VMM-vocabulary gate", "kayfabe-vmm-kvm"), // ★ ARGUABLE: a non-QEMU adapter; a QEMU-ism here would be a real defect and nothing catches it
    ("VMM-vocabulary gate", "kayfabe-vmm-qemu"), // ADAPTER_CRATES — must say MemoryRegion
    ("VMM-vocabulary gate", "kayfabe-qemu-raw"), // ADAPTER_CRATES
    ("VMM-vocabulary gate", "kayfabe-crec"),
    ("VMM-vocabulary gate", "kayfabe-mocks"), // ★ ARGUABLE: holds MockVmm, the port's reference impl
    // ── Generation-name gate (`pure`): may this crate name a concrete chip/driver version?
    ("Generation-name gate", "kayfabe-abi"), // Axis-A's OWN quarantine — the gate's prose says so
    ("Generation-name gate", "kayfabe-crec"), // Axis-B's arch adapter: still names GA10x in prose
    // ★★ Axis-B's PRODUCTION home as of 2026-07-31. The GA10x register map moved here from
    // `kayfabe-crec` so a shipped archive could reach it, and `ga10x.rs` is exactly where a
    // chip's offsets BELONG — the gate's own failure text says "an arch-impl crate". This is
    // the crate that sentence names. Note it is SCOPED by the other two vocabulary gates:
    // being allowed to say GA106 is not being allowed to say `epoll` or `MemoryRegion`.
    ("Generation-name gate", "kayfabe-device"),
    ("Generation-name gate", "kayfabe-mocks"), // MockArch classifies AMPERE_* class ids (mocks/src/lib.rs:298-306)
    ("Generation-name gate", "kayfabe-rt"), // ★ ARGUABLE: clean today, and should never name a chip
    ("Generation-name gate", "kayfabe-shell"), // ★ ARGUABLE: same
    ("Generation-name gate", "kayfabe-linux-raw"),
    // ⊘ **NOT clean** — this annotation said "★ ARGUABLE: clean today" and that was false.
    // Measured 2026-08-01 at `419afe8` with the gate's own regex: **10 matching lines**, all in
    // `rm.rs` — `AMPERE_CHANNEL_GPFIFO_A`, `AMPERE_DMA_COPY_B`, `AMPERE_USERMODE_A`,
    // `KEPLER_CHANNEL_GROUP_A`, `FERMI_VASPACE_A`. (The gate's first alternation has a leading
    // `\b` and **no trailing one**, so it matches `AMPERE_DMA_COPY_B`; a `\bampere\b` spot-check
    // reports 0 and is the wrong instrument.)
    //
    // ★ Why the distinction is load-bearing rather than pedantic: "clean today" says the
    // exemption is FREE and could be dropped at no cost. It cannot — dropping it turns the gate
    // red. This exemption is **carrying** something.
    //
    // What it carries: these are NVIDIA's own class names for objects we allocate on the **host**
    // GPU. Kepler/Fermi ones are permanent names for still-current classes and are not a
    // generation claim at all. But `AMPERE_DMA_COPY_B` genuinely differs on a Hopper host, and it
    // sits on the forwarding path behind no `Arch` trait — so this is a **real residue**, tracked
    // as task #156, not a settled exemption. Kept as a record of a known gap.
    ("Generation-name gate", "kayfabe-isolate-host"),
    ("Generation-name gate", "kayfabe-vmm-kvm"),
    ("Generation-name gate", "kayfabe-vmm-qemu"),
    ("Generation-name gate", "kayfabe-qemu-raw"),
    // ★ `kayfabe-chips` is the arch-impl crate the Generation-name gate's OWN failure
    // message points at: *"the concrete number itself -> an arch-impl crate
    // (`impl Arch for <Gen>`), never a logic crate"*. Naming AD10x and GH100 is the
    // crate's entire job, so it is exempt HERE and SCOPED by the other two gates
    // (`crates/kayfabe-chips` is in both `pure=` and `portable=` in ci.yml) — a chip
    // model must still never name an OS readiness primitive or one hypervisor's API.
    // Exempting it from all three would have been the quiet way to let a new
    // architecture escape the seam checks.
    ("Generation-name gate", "kayfabe-chips"),
];

/// ★★★ Every crate in the tree is either **scoped by** a vocabulary gate or **named in
/// the exemption table above**. A new crate is covered on the day it is added.
///
/// ## The hole this closes, measured rather than reasoned about
///
/// The three vocabulary gates scope a **hand-written list of crate paths**. Nothing
/// re-derived those lists against the tree, and the sibling test above only asserts (a)
/// that the adapter crates are *absent* and (b) that `crates/kayfabe-core` is *present*
/// as a non-vacuity anchor. Measured on 2026-07-29 (task #100, the five-axis seam audit):
/// deleting `crates/kayfabe-rt` from the VMM-vocabulary gate's `portable=` list made the
/// gate stop covering a crate **and every test in the workspace still passed**. The same
/// silence covers the other direction, which is the one that actually happens: two crates
/// added on 2026-07-29 — `kayfabe-crec` (the first concrete GPU generation in the tree)
/// and `kayfabe-isolate-host` (the first real driver ioctls) — entered a workspace whose
/// seam gates could not see them, and nothing said so.
///
/// That is the failure mode the Unsafe-surface gate's own comment already names:
/// *"a gate that enumerates today's crates stops covering the code the moment someone
/// adds one"* (`ci.yml`). That gate walks the whole repo and so does not have it. These
/// three cannot — a per-crate scope is the whole point of them — so the enumeration is
/// checked against the filesystem here instead.
///
/// ## What this test does NOT do
///
/// It does not decide anything. Every exemption above is the status quo, transcribed. The
/// ones marked ★ ARGUABLE are flagged for the owner and left exactly as they were.
#[test]
fn every_crate_is_either_gated_or_explicitly_exempted() {
    let yml = ci_yml();
    let root = repo_root();

    let mut crates: Vec<String> = std::fs::read_dir(root.join("crates"))
        .expect("crates/ exists")
        .filter_map(|e| {
            let e = e.expect("readable dir entry");
            let name = e.file_name().to_string_lossy().into_owned();
            e.path().join("Cargo.toml").is_file().then_some(name)
        })
        .collect();
    crates.sort();
    assert!(
        crates.len() >= 20,
        "★ NON-VACUITY: only {} crates discovered — the scan is broken, and every \
         assertion below would be about an empty set ({crates:?})",
        crates.len()
    );

    let mut unclassified = Vec::new();
    for (step, var) in [
        ("Hexagonal boundary gate", "pure"),
        ("VMM-vocabulary gate", "portable"),
        ("Generation-name gate", "pure"),
    ] {
        let list = gate_list(&step_body(&yml, step), var);
        for c in &crates {
            // Substring is not enough: `crates/kayfabe-vmm` is a prefix of
            // `crates/kayfabe-vmm-kvm`, so a naive `contains` would report the KVM
            // adapter as gated by the VMM gate. Match the whitespace-delimited token.
            let gated = list
                .split_whitespace()
                .any(|tok| tok == format!("crates/{c}"));
            let exempt = GATE_EXEMPTIONS.iter().any(|(s, x)| *s == step && x == c);
            match (gated, exempt) {
                (true, true) => unclassified.push(format!(
                    "  {step}: {c} is BOTH scoped and exempted — the table contradicts the gate"
                )),
                (false, false) => {
                    unclassified.push(format!("  {step}: {c} is neither scoped nor exempted"))
                }
                _ => {}
            }
        }
    }

    assert!(
        unclassified.is_empty(),
        "★ A CRATE IS INVISIBLE TO A SEAM GATE.\n{}\n\n\
         Every crate must be one of two things for each vocabulary gate, and saying which \
         is a design decision, not a list edit:\n\
         - SCOPED   -> add `crates/<name>` to that gate's list in .github/workflows/ci.yml\n\
         - EXEMPT   -> add a row to GATE_EXEMPTIONS in this file, WITH the reason on the \
         line. \"It is an adapter\" is a reason; silence is not.\n\
         This test exists because on 2026-07-29 the answer for two brand-new crates was \
         neither, and the seam gates that are supposed to protect Axes A/B/D could not \
         see the first real GPU generation or the first real driver ioctls in the tree.",
        unclassified.join("\n")
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
            // ★★★ THE FIFTH FORM, added 2026-07-30 at stage Q2, because the first FFI crate
            // proved the other four are not a partition. `{kw} extern "C" fn` is the
            // dominant shape in a foreign-function crate — it is what an entry point IS —
            // and none of the four above can match it, so the ratchet counted 23 of this
            // crate's 31 relaxations and reported a complete audit. A DEFINITION is matched
            // (a name follows `fn `) and a FIELD TYPE is not (`(` follows), because
            // `Option<{kw} extern "C" fn(..)>` declares a signature and relaxes nothing.
            // `kayfabe-linux-raw` contains no occurrence of the form, so its reviewed bar is
            // unchanged by this — measured before the change, not assumed after it.
            let extern_defs = text
                .match_indices(&format!("{kw} extern \"C\" fn "))
                .filter(|(i, m)| {
                    text[i + m.len()..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                })
                .count();
            actual += u32::try_from(
                text.match_indices(&format!("{kw} {{")).count()
                    + text.match_indices(&format!("{kw} fn ")).count()
                    + text.match_indices(&format!("{kw} impl ")).count()
                    + text.match_indices(&format!("{kw} trait ")).count()
                    + extern_defs,
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
