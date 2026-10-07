//! # The host-class **role wiring** gate (`#166`)
//!
//! ## What was uncovered, measured rather than suspected
//!
//! `#156` gave the host forwarding path a `kayfabe_arch::HostClasses` profile: three
//! NVIDIA class ids that differ by host generation — the GPFIFO channel, the usermode
//! doorbell window, and the copy-engine object. The three *values* were pinned nine ways
//! against NVIDIA's own per-chip table (`crates/kayfabe-chips/tests/host_classes.rs`).
//!
//! **Which role each call site asked for was pinned zero ways.** `scripts/bite_host_classes.py`
//! measured it at `36f746a`: swap `classes.usermode()` for `classes.gpfifo_channel()` at
//! the doorbell-window allocation, `gpfifo_channel()` for `ce_object()` at the channel
//! allocation, `ce_object()` for `gpfifo_channel()` in the pushbuffer's `SET_OBJECT` —
//! **`PROFILE: 9/9 caught. WIRING: 0/3 caught.`** The nine value bites all fired; not one
//! role bite did.
//!
//! ★★★ **And the wrong pick is SERVED, not refused.** GH100's class list still contains
//! `AMPERE_CHANNEL_GPFIFO_A` (`ogkm-580: src/nvidia/generated/g_gpu_class_list.c:1996`)
//! and `AMPERE_USERMODE_A` (`:1997`), and RM has a live `CliGetChannelClassInfo` arm for
//! the former (`src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:1588-1594`). Two of the
//! three wrong roles would be accepted by a real Hopper board with no error, no Xid and
//! no diagnostic. Only `AMPERE_DMA_COPY_B` is absent and fails loudly. A defect class
//! that hardware will not report is one a test has to.
//!
//! ## ★★★ The instrument is the TYPE SYSTEM, and this file guards its edges
//!
//! The fix is not a test. `HostClasses`' three methods now return three **distinct
//! types** — `ChannelClass`, `UsermodeClass`, `CeObjectClass` — and every consumer on the
//! host path names the role it wants in a parameter or a field type. A call site that
//! asks for the wrong role **does not compile**.
//!
//! That is strictly stronger than any test could be, for the reason
//! `gates_quantified_over_a_list` gives: *derive the universe instead of listing it.*
//! **rustc quantifies over every call site in the workspace**, including ones written
//! tomorrow, so there is no list here to fall out of date and no way for a new call site
//! to be uncovered by default.
//!
//! ⊘ **But a type-level refusal has exactly three ways to be dismantled**, and each is a
//! quiet one-line edit that leaves the whole suite green. This file exists for those
//! three and nothing else:
//!
//! 1. **Collapse the types.** `pub use ChannelClass as UsermodeClass;` and every swap
//!    compiles again → [`the_three_roles_are_three_distinct_rust_types`].
//! 2. **Add a uniform escape.** `impl From<ChannelClass> for ClassId`, a `Deref`, or a
//!    `pub` tuple field, and every site can untag without naming a role →
//!    [`no_uniform_escape_off_a_role_type_exists`].
//! 3. **Untag at the call site.** `classes.usermode().usermode_id()` reconstructs the
//!    bare-`u32` hole exactly where it was, since the *next* editor sees a `ClassId` and
//!    can swap the role freely → [`no_role_is_untagged_at_the_point_it_is_asked_for`].
//!
//! …plus the containment that keeps (2) and (3) from creeping: the untag sites are
//! **derived by walking the tree** and pinned per file with a reason
//! ([`the_role_untag_sites_are_exactly_the_pinned_derived_set`]). A fourth untag appearing
//! anywhere is a red test rather than a silent widening.
//!
//! ## ★ Every checker here is run against a synthetic violation in the same test
//!
//! *A gate never SEEN to fail is not evidence* (`suspect_the_instrument_first`), and on
//! this project a list-driven gate must be seen to fail **per entry**. So each checker
//! below is a pure function over `(path, text)` pairs, and each test feeds it a hand-built
//! violating input and requires it to fire before believing the tree-walk it did over the
//! real files.
//!
//! ## ⊘ What this file does NOT establish
//!
//! Whether the class ids are right (that is `kayfabe-chips`' oracle test), and whether a
//! real board accepts any of it. Nothing here has been near Hopper silicon. **Compiling
//! for a generation is not booting on one** — this gate is drift prevention
//! (`only_live_boots_are_proof` (c)), not proof.

use std::any::TypeId;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kayfabe_arch::{CeObjectClass, ChannelClass, UsermodeClass};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("the workspace root is one level above the tests crate")
}

/// The role names, in role order — channel, usermode, CE object.
const ROLES: [&str; 3] = ["channel", "usermode", "ce_object"];

/// The three untag methods, in role order. There is deliberately no uniform `.id()`, so
/// this list *is* the complete vocabulary for taking a role tag off.
///
/// ★★★ **Assembled from fragments, and that is not decoration.** The first run of this
/// file failed **on itself**: it walks every `.rs` in the tree, its own synthetic
/// violation fixtures are `.rs`, and so the gate reported two violations that were its
/// own test data. This project has hit that exact shape before — *"a gate matched its own
/// scanner's string literal, so the scanner now assembles the token from fragments"* —
/// and this is the same remedy. Building the needles at runtime means the file contains
/// no occurrence of any pattern it hunts for, so the universe stays honest and this file
/// needs **no self-exemption** (an exemption for the scanner is a permanent blind spot in
/// exactly the file most likely to be edited when the gate is inconvenient).
fn untag_needles() -> [String; 3] {
    ROLES.map(|r| format!(".{r}_{}()", "id"))
}

/// The three role *questions*, in the same order — what a call site asks a profile for.
/// Assembled for the same reason as [`untag_needles`].
fn ask_needles() -> [String; 3] {
    ["gpfifo_channel", "usermode", "ce_object"].map(|r| format!(".{r}{}", "()"))
}

// ---------------------------------------------------------------------------------
// The universe: every Rust source in the workspace, derived by walking
// ---------------------------------------------------------------------------------

/// Collect `(repo-relative path, source text)` for every `.rs` file in the tree.
///
/// ★ **Derived, not listed.** A crate added tomorrow is in this universe tomorrow. The
/// only exclusions are build output and VCS metadata, which contain no authored source —
/// and `target` is excluded **by name at any depth**, because the `-maxdepth 1` variant of
/// this idea is precisely how the `unsafe_code` ratchet went blind on 2026-07-29: it
/// counted one directory level while its sibling gate pruned a whole subtree, and a
/// relaxation one directory deeper was invisible to both.
///
/// ⊘ ★ And the reason that sentence spells the LINT's name rather than the keyword: the
/// `*_unsafe.rs` surface gate greps for the bare keyword in **any** `.rs` file, prose
/// included, and this file tripped it on the first `ci_gates.sh --all` run. That gate is
/// right to be that blunt — the moment it needs a judgement call, `ls` stops being the
/// audit — so the prose moves, not the gate.
fn all_rust_sources() -> Vec<(String, String)> {
    let root = repo_root();
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                // Build output, VCS metadata and vendored reference trees hold no
                // authored Rust of ours.
                if name == ".git" || name == "target" || name.starts_with("target-") {
                    continue;
                }
                stack.push(p);
            } else if name.ends_with(".rs")
                && let Ok(text) = std::fs::read_to_string(&p)
            {
                let rel = p
                    .strip_prefix(&root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, text));
            }
        }
    }
    out.sort();
    out
}

/// Drop whole-line comments before matching.
///
/// ★ Deliberate, and it is a trade rather than an oversight: a doc comment that *names*
/// `.ce_object_id()` — this file's own module docs do — is prose, not a call, and a gate
/// that reddens when someone documents it would be an instrument nobody keeps. Code on a
/// line with a trailing `//` comment is still scanned; only lines that are *entirely*
/// comment are dropped.
fn code_lines(text: &str) -> Vec<(usize, &str)> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| {
            let t = l.trim_start();
            !(t.starts_with("//") || t.starts_with("* ") || t == "*")
        })
        .map(|(i, l)| (i + 1, l))
        .collect()
}

// ---------------------------------------------------------------------------------
// Checker 1 — the untag census
// ---------------------------------------------------------------------------------

/// `path -> [count of .channel_id(), .usermode_id(), .ce_object_id()]`, code lines only.
fn untag_census(sources: &[(String, String)]) -> BTreeMap<String, [usize; 3]> {
    let mut out = BTreeMap::new();
    let untag = untag_needles();
    for (path, text) in sources {
        let mut counts = [0usize; 3];
        for (_, line) in code_lines(text) {
            for (i, needle) in untag.iter().enumerate() {
                counts[i] += line.matches(needle.as_str()).count();
            }
        }
        if counts.iter().any(|c| *c > 0) {
            out.insert(path.clone(), counts);
        }
    }
    out
}

/// ★★★ **The pinned set of places a role tag is allowed to come off**, with the reason on
/// the line — derived counts on the left, a human's justification on the right.
///
/// `gates_quantified_over_a_list` again: a list *of things to check* is weakened by
/// shortening it, so this is not that. This is a list of **exemptions**, and the universe
/// it is checked against is the whole tree. Deleting an entry here does not shrink
/// coverage — it turns the site it described into an unexplained violation.
const PINNED_UNTAG_SITES: &[(&str, [usize; 3], &str)] = &[
    (
        "crates/kayfabe-isolate-host/src/rm.rs",
        [1, 1, 2],
        "the host adapter — the four places a class id must become a u32 to reach a real \
         NV_ESC_RM_ALLOC: alloc_gpfifo_channel, open_usermode, ce_pushbuffer's SET_OBJECT \
         and alloc_ce_engine_object. Each sits inside a function whose PARAMETER (or \
         struct field) already names the role, so the untag cannot be the wrong one",
    ),
    (
        "crates/kayfabe-chips/tests/host_classes.rs",
        [1, 1, 1],
        "the value oracle's `roles()` helper — its expectation is a table of bare u32s \
         transcribed from g_gpu_class_list.c, so the tag has to come off exactly once, \
         in role order, to compare against it",
    ),
];

/// Violations = a file untagging that the pin does not describe, or describing it with
/// the wrong count.
fn untag_violations(sources: &[(String, String)]) -> Vec<String> {
    let census = untag_census(sources);
    let pinned: BTreeMap<&str, [usize; 3]> = PINNED_UNTAG_SITES
        .iter()
        .map(|(p, c, _)| (*p, *c))
        .collect();
    let mut bad = Vec::new();
    for (path, counts) in &census {
        match pinned.get(path.as_str()) {
            None => bad.push(format!(
                "{path}: untags a role class {counts:?} (channel/usermode/ce) and is NOT \
                 in PINNED_UNTAG_SITES. Route the value through a role-typed parameter or \
                 field instead — or add it here WITH the reason"
            )),
            Some(want) if want != counts => bad.push(format!(
                "{path}: untag count moved {want:?} -> {counts:?} (channel/usermode/ce)"
            )),
            Some(_) => {}
        }
    }
    for (path, _, _) in PINNED_UNTAG_SITES {
        if !census.contains_key(*path) {
            bad.push(format!(
                "{path}: pinned as an untag site but untags nothing — the exemption is \
                 stale, and a stale exemption is a hole nobody is watching"
            ));
        }
    }
    bad
}

// ---------------------------------------------------------------------------------
// Checker 2 — no untag at the point the role is asked for
// ---------------------------------------------------------------------------------

/// `classes.usermode().usermode_id()` puts the bare `ClassId` back exactly where the
/// swap used to live: the next editor sees an untyped number at the call site and can
/// change which role produced it freely. Refuse the shape.
///
/// The **one** allowed file is the value oracle, whose whole job is to compare the three
/// roles against a table of raw ids.
const INLINE_UNTAG_ALLOWED: &[(&str, usize, &str)] = &[(
    "crates/kayfabe-chips/tests/host_classes.rs",
    3,
    "`roles()` — the oracle's single untag point, three roles, once each",
)];

fn inline_untag_violations(sources: &[(String, String)]) -> Vec<String> {
    let (ask_all, untag) = (ask_needles(), untag_needles());
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut sites = Vec::new();
    for (path, text) in sources {
        for (n, line) in code_lines(text) {
            for ask in &ask_all {
                let mut from = 0usize;
                while let Some(at) = line[from..].find(ask.as_str()) {
                    let after = from + at + ask.len();
                    let rest = &line[after..];
                    if untag.iter().any(|u| rest.starts_with(u.as_str())) || rest.starts_with(".0")
                    {
                        *counts.entry(path.clone()).or_default() += 1;
                        sites.push(format!("{path}:{n}: {}", line.trim()));
                    }
                    from = after;
                }
            }
        }
    }
    let allow: BTreeMap<&str, usize> = INLINE_UNTAG_ALLOWED
        .iter()
        .map(|(p, c, _)| (*p, *c))
        .collect();
    let mut bad = Vec::new();
    for (path, n) in &counts {
        match allow.get(path.as_str()) {
            Some(want) if want == n => {}
            Some(want) => bad.push(format!(
                "{path}: {n} inline untags at a role call site, pinned at {want}"
            )),
            None => bad.push(format!(
                "{path}: {n} inline untag(s) at a role call site — a role asked for and \
                 immediately stripped is the pre-#166 hole rebuilt in one line. Sites: {}",
                sites
                    .iter()
                    .filter(|s| s.starts_with(path.as_str()))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" | ")
            )),
        }
    }
    bad
}

// ---------------------------------------------------------------------------------
// Checker 3 — no uniform escape off a role type
// ---------------------------------------------------------------------------------

const ROLE_TYPES: [&str; 3] = ["ChannelClass", "UsermodeClass", "CeObjectClass"];

/// Trait impls that would hand back the inner `ClassId` without naming a role, and a
/// public tuple field, which does the same with no impl at all.
fn escape_hatch_violations(sources: &[(String, String)]) -> Vec<String> {
    let mut bad = Vec::new();
    for (path, text) in sources {
        for (n, line) in code_lines(text) {
            let t = line.trim();
            for role in ROLE_TYPES {
                if t.starts_with(&format!("pub struct {role}(pub "))
                    || t.starts_with(&format!("pub struct {role} {{ pub "))
                {
                    bad.push(format!(
                        "{path}:{n}: `{role}` has a PUBLIC inner field — every site can \
                         untag without naming a role: {t}"
                    ));
                }
                for form in [
                    format!("impl From<{role}> for"),
                    format!("impl Deref for {role}"),
                    format!("impl DerefMut for {role}"),
                    format!("impl AsRef<ClassId> for {role}"),
                    format!("impl Borrow<ClassId> for {role}"),
                ] {
                    if t.starts_with(&form) {
                        bad.push(format!(
                            "{path}:{n}: `{form}` is a UNIFORM escape off a role type — \
                             the whole refusal is `into()` away after this: {t}"
                        ));
                    }
                }
            }
        }
    }
    bad
}

// =================================================================================
// The tests
// =================================================================================

/// ★★★ **The cheapest way to dismantle everything**: make two role names the same type.
///
/// `pub use ChannelClass as UsermodeClass;` in `kayfabe-arch` compiles, every call site
/// keeps compiling, every existing test stays green — and every role swap is legal again.
/// Nothing else in this tree observes type *identity*, so nothing else can see it happen.
#[test]
fn the_three_roles_are_three_distinct_rust_types() {
    let ids = [
        ("ChannelClass", TypeId::of::<ChannelClass>()),
        ("UsermodeClass", TypeId::of::<UsermodeClass>()),
        ("CeObjectClass", TypeId::of::<CeObjectClass>()),
    ];
    let mut pairs = 0usize;
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            assert_ne!(
                ids[i].1, ids[j].1,
                "★★★ `{}` and `{}` are the SAME Rust type. The host-class role refusal is \
                 entirely a type-identity property: if two roles alias, `classes.usermode()` \
                 and `classes.gpfifo_channel()` become interchangeable at every call site \
                 and a Hopper host serves the wrong one silently",
                ids[i].0, ids[j].0
            );
            pairs += 1;
        }
    }
    assert_eq!(
        pairs, 3,
        "★ NON-VACUITY: three roles means three unordered pairs must have been compared"
    );

    // ⊘ Deliberately NOT also checking that each role round-trips its `ClassId` here.
    // That would be the only real untag in this file, and it would force the scanner to
    // exempt its own source — see [`untag_needles`]. The value flow through all three
    // untags is `crates/kayfabe-chips/tests/host_classes.rs`'s `roles()`, judged against
    // NVIDIA's own per-chip table, which is a stronger check than a round-trip anyway.
}

/// ★★ Every place in the tree where a role tag comes off is pinned, **with a reason**.
///
/// The universe is a walk of the whole workspace, so a new crate is covered by default;
/// the pin is a list of *exemptions*, so deleting one creates a violation rather than
/// removing coverage.
#[test]
fn the_role_untag_sites_are_exactly_the_pinned_derived_set() {
    let sources = all_rust_sources();
    assert!(
        sources.len() > 100,
        "★ NON-VACUITY: the tree walk found only {} Rust files, so any 'no violations' \
         verdict below is a statement about almost nothing",
        sources.len()
    );

    // ★ SEEN TO FAIL, per entry, before the real verdict is believed.
    for (i, untag) in untag_needles().iter().enumerate() {
        let synthetic = vec![(
            "crates/kayfabe-somewhere/src/new.rs".to_string(),
            format!("fn f(c: Role) -> u32 {{ c{untag}.0 }}\n"),
        )];
        let fired = untag_violations(&synthetic);
        assert!(
            !fired.is_empty(),
            "★★★ THE INSTRUMENT IS THE DEFECT: checker did not fire on a synthetic \
             `{untag}` (role {i}) in an unpinned file"
        );
    }
    // …and a pinned file whose count MOVED must fire too, or the counts are decoration.
    let moved = vec![(
        PINNED_UNTAG_SITES[0].0.to_string(),
        format!(
            "let a = x{};\nlet b = y{};\n",
            untag_needles()[0],
            untag_needles()[0]
        ),
    )];
    assert!(
        !untag_violations(&moved).is_empty(),
        "★★★ THE INSTRUMENT IS THE DEFECT: a pinned file's untag count changed and the \
         checker stayed quiet"
    );

    let bad = untag_violations(&sources);
    assert!(
        bad.is_empty(),
        "★★★ The role tag is coming off somewhere nobody accounted for:\n  {}\n\n\
         Why this matters: a `ClassId` carries no role, so once the tag is off, swapping \
         WHICH role produced it compiles and a real Hopper host SERVES two of the three \
         wrong answers (g_gpu_class_list.c:1996/:1997).",
        bad.join("\n  ")
    );
}

/// ★★ A role asked for and stripped on the same expression is the old hole, rebuilt.
#[test]
fn no_role_is_untagged_at_the_point_it_is_asked_for() {
    let sources = all_rust_sources();

    // SEEN TO FAIL — one synthetic per role question.
    for (ask, untag) in ask_needles().iter().zip(untag_needles().iter()) {
        let synthetic = vec![(
            "crates/kayfabe-somewhere/src/new.rs".to_string(),
            format!("let x = self.classes{ask}{untag}.0;\n"),
        )];
        assert!(
            !inline_untag_violations(&synthetic).is_empty(),
            "★★★ THE INSTRUMENT IS THE DEFECT: `{ask}{untag}` did not fire the checker"
        );
    }
    // The `.0` form too — `HostClasses` returning a tuple struct again would be the
    // same hole with different spelling.
    assert!(
        !inline_untag_violations(&[(
            "crates/kayfabe-somewhere/src/new.rs".to_string(),
            format!("let x = c{}.0;\n", ask_needles()[2]),
        )])
        .is_empty(),
        "★★★ THE INSTRUMENT IS THE DEFECT: the `.0` form did not fire"
    );

    let bad = inline_untag_violations(&sources);
    assert!(
        bad.is_empty(),
        "★★ A host class role is being untagged at the site that asks for it:\n  {}",
        bad.join("\n  ")
    );
}

/// ★★ No `From`, no `Deref`, no public inner field — the untag must always name a role.
#[test]
fn no_uniform_escape_off_a_role_type_exists() {
    let sources = all_rust_sources();

    // SEEN TO FAIL, per forbidden form AND per role.
    for role in ROLE_TYPES {
        for line in [
            format!("pub struct {role}(pub ClassId);"),
            format!("impl From<{role}> for ClassId {{}}"),
            format!("impl Deref for {role} {{}}"),
            format!("impl AsRef<ClassId> for {role} {{}}"),
        ] {
            assert!(
                !escape_hatch_violations(&[(
                    "crates/kayfabe-arch/src/lib.rs".to_string(),
                    format!("{line}\n"),
                )])
                .is_empty(),
                "★★★ THE INSTRUMENT IS THE DEFECT: `{line}` did not fire the checker"
            );
        }
    }

    let bad = escape_hatch_violations(&sources);
    assert!(
        bad.is_empty(),
        "★★ A uniform escape off a role type exists. Once `ClassId::from(role)` compiles, \
         every call site can untag without naming a role and `#166` is decoration:\n  {}",
        bad.join("\n  ")
    );
}

/// ★ The host adapter really is where the roles are consumed — otherwise the three tests
/// above are guarding a seam nothing uses.
///
/// This is the non-vacuity of the whole file: it asserts the role *questions* are asked
/// in the tree at all, and that they are asked from the host-forwarding adapter, which is
/// the only place a class id reaches a real `NV_ESC_RM_ALLOC`.
#[test]
fn the_role_questions_are_actually_asked_on_the_host_forwarding_path() {
    let sources = all_rust_sources();
    let ask = ask_needles();
    let mut per_role = [0usize; 3];
    let mut in_host_adapter = 0usize;
    for (path, text) in &sources {
        // Only the adapter counts for the second tally; the profiles DEFINE these
        // methods rather than asking them.
        let is_adapter = path.starts_with("crates/kayfabe-isolate-host/");
        for (n, line) in code_lines(text) {
            let _ = n;
            for (i, ask) in ask.iter().enumerate() {
                // `fn usermode(&self) -> …` is a definition, not a question.
                let c = line.matches(ask.as_str()).count();
                if c > 0 && !line.trim_start().starts_with("fn ") {
                    per_role[i] += c;
                    if is_adapter {
                        in_host_adapter += c;
                    }
                }
            }
        }
    }
    for (i, ask) in ask.iter().enumerate() {
        assert!(
            per_role[i] > 0,
            "★ NON-VACUITY: nothing in the tree asks a profile for `{ask}` — the role \
             refusal guards a seam with no consumers"
        );
    }
    assert!(
        in_host_adapter >= 4,
        "★ NON-VACUITY: only {in_host_adapter} role question(s) in \
         crates/kayfabe-isolate-host/. #166 was raised because FOUR call sites there \
         (doorbell window, channel alloc, SET_OBJECT, CE engine object) each pick a role. \
         If they moved, this gate is watching the wrong crate"
    );
}
