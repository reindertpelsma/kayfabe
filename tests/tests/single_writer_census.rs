//! ★★★★★ **THE SINGLE-WRITER CENSUS** — the half a type cannot see.
//!
//! `completion_observer.md` §8 measured, on 2026-08-10, that this tree has **zero** writers
//! to the page `cuCtxCreate` polls and exactly **one** function that writes a completion
//! semaphore at all. `w226b_534e1b3_cup2` corroborated it from the other side: the observer
//! read guest VA `0x2_0440fff0` **86 times across the whole `cuCtxCreate` wall** and it was
//! `0x00000000` every time, on all eight `GrCompute` channels. §8.3's argument for why that
//! pairing is strong is worth keeping: *"the grep could miss a writer, and the read could
//! miss a write that was overwritten — but a writer the grep missed would have had to write
//! `0` eighty-six times running to hide."*
//!
//! ⊘ **And §8.4 stated the limit, which is why this file exists:** the property is
//! *"currently **accidental**, asserted nowhere, and one refactor from untrue."*
//!
//! # ★★★ Why "accidental" is not survivable here
//!
//! The C artifact spent two months reading its `MC_SERVICE_INTERRUPTS` spin as a **missing**
//! completion. It was a **corrupted** one: a lagging bridged host CE DMA-wrote stale payloads
//! `1,2` over a live `0x1e`; UVM's 32→64-bit wrap detector (`uvm_gpu_semaphore.c:776`) read
//! the decrease as a `2^32` wrap, reconstructed `completed_value 0x1_00000054 > queued 0x54`,
//! and wedged the channel (`uvm_channel.c:205`). **The entire fix (M5.38, `ceb13f5`) was to
//! DELETE THE SECOND WRITER** — no completion plane was built.
//!
//! `UVM_ASSERT_MSG_RELEASE` is compiled into **release** builds, so a backwards write is
//! fatal on **first occurrence**. ⇒ There is no run in which a second writer shows up as a
//! small regression first, and therefore no run in which a behavioural test catches it before
//! a guest does. The gate has to be static.
//!
//! # The two halves, and why neither alone is the property
//!
//! | half | what it pins | where |
//! |---|---|---|
//! | **type** | a `ResolvedRelease` cannot be minted outside `kayfabe-rt` — the only way to obtain one is `resolve_releases`, which resolves every word through the guest's own page tables | `crates/kayfabe-rt/tests/ui/mint_a_release.rs` |
//! | **census** (this file) | no SECOND writer appears anywhere in the workspace's production source | here |
//!
//! ⊘ **A type cannot see a caller that never names it**, and that is not a theoretical gap —
//! §8.4 names the exact shape: `execute_ours` reaches the very same `write_plane` primitive
//! with a [`kayfabe_fwd::CeSpan`], and a `CeSpan` carrying `CeSource::Constant(1)` at a
//! semaphore's `dst_place` is byte-for-byte a semaphore write **with none of the completion
//! discipline** — no resolve-all-before-write, no timestamp precondition, no
//! write-before-signal, no `CeReleaseNoClock`. `partition_ce` can legally emit it for a fill
//! whose destination happens to be the semaphore page. That caller never types the word
//! `ResolvedRelease`. Only a census sees it.
//!
//! ⇒ ★ This is `measure_at_the_boundary_not_inside` as a pair rather than a slogan: the type
//! guards the value, the census guards the primitive, and the two instruments are independent.
//!
//! # The polarity convention, taken from `ci.yml`'s GPA-accessor gate (`:995`)
//!
//! Two halves, both required: **(a)** a scope in which the primitive may not appear at all,
//! and **(b)** an exact count in the file that owns it. A gate that only checks (a) passes
//! when the owning file grows a second call.
//!
//! ⚠ `[the trap this scanner inherits, measured 2026-08-09 in `unranked_locks.rs`]` a
//! line-by-line scan is blind to `rustfmt`'s wrapping. This one strips comments and then scans
//! the **whole file text**, and `the_census_bites_a_writer_that_rustfmt_has_wrapped` is the
//! positive control for exactly that spelling.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// ★★★ **THE PINNED WRITE SURFACE.** Every `(file, primitive, count)` in the workspace's
/// production source that can put bytes where the guest can read them, or can signal that
/// bytes have landed.
///
/// ⊘ **A row is a RULING, not an inventory line.** Adding one is how a second writer is
/// admitted, so each carries the argument for why it is not the M5.38 defect. Changing a
/// count is the same act as adding a row.
///
/// ⚠ Counts, not line numbers: a line number pins nothing and rots on the next edit, while a
/// count goes red exactly when a call appears or disappears.
const WRITE_SURFACE: &[(&str, &str, usize, &str)] = &[
    // ---- the guest-RAM plane -----------------------------------------------------------
    (
        "crates/kayfabe-rt/src/cpu_ce.rs",
        ".gpa_write(",
        1,
        "★ THE one. Inside `write_plane`'s `CpuPlane::GuestRam` arm — the whole CE and \
         completion write surface, funnelled through a single private primitive that takes a \
         `PlaneAddr` newtype and never a raw u64.",
    ),
    (
        "crates/kayfabe-qemu-raw/src/shim.rs",
        ".gpa_write(",
        1,
        "⊘ A DIFFERENT PLANE, and it is not a semaphore. `MachineRam::write` is the \
         emulated-GSP guest-RAM port; its only consumers are `kayfabe_gsp`'s `GspRegion::write` \
         and `rmrpc/fault.rs`'s notifier — bounded to declared GSP regions and the RM event \
         notifier `notifier_gpa`. It cannot reach a completion page. (`completion_observer.md` \
         §8.1, writer W2.)",
    ),
    // ---- the framebuffer plane ---------------------------------------------------------
    (
        "crates/kayfabe-rt/src/cpu_ce.rs",
        ".write_tagged(",
        1,
        "The `CpuPlane::Fb` arm of the same single primitive, tagged `FbWriter::Executor`. \
         ⚠ KNOWN RESIDUAL: that ONE tag covers both `execute_ours` (bulk copy) and \
         `write_resolved_completion` (the release), so the framebuffer's FIRST-WRITER census \
         cannot today tell a copy from a completion. A census-based gate would need a fifth \
         `FbWriter` kind first; this file is the gate that does not need one.",
    ),
    (
        "crates/kayfabe-device/src/fbwin.rs",
        ".write_tagged(",
        1,
        "⊘ Not a writer — the trait's own untagged `write` forwarding to `write_tagged` with \
         `FbWriter::Unattributed`. Infrastructure of the tagging scheme, not a user of it.",
    ),
    (
        "crates/kayfabe-device/src/plane.rs",
        ".write_tagged(",
        1,
        "⊘ The BAR window store, tagged `FbWriter::Window(w)`. A guest MMIO write landing in \
         the framebuffer through its own aperture — a different plane from a GPU-engine \
         release, and attributed as such.",
    ),
    // ---- the funnel itself -------------------------------------------------------------
    (
        "crates/kayfabe-rt/src/cpu_ce.rs",
        "write_plane(",
        4,
        "★ ONE definition and THREE calls: `execute_ours`'s fill arm, `execute_ours`'s copy \
         arm, and `write_resolved_completion`'s word loop. `write_plane` is PRIVATE — making \
         it `pub(crate)` is a one-line edit that opens it to `ceutils.rs`, `device.rs` and \
         every future module in this crate, and that edit alone cannot move this count. \
         `the_write_primitive_is_private` is the assertion that catches it.",
    ),
    // ---- the completion tail -----------------------------------------------------------
    (
        "crates/kayfabe-rt/src/cpu_ce.rs",
        "write_resolved_completion(",
        2,
        "The definition, plus the one call from `write_completion` (which see below).",
    ),
    (
        "crates/kayfabe-rt/src/ceutils.rs",
        "write_resolved_completion(",
        2,
        "★ THE ONLY TWO PRODUCTION RELEASE SITES IN THE TREE: `run_submission`'s \
         transfer-NONE arm and the copy launch's carried finishPayload. Both take a \
         `&[ResolvedRelease]` the resolver minted, both pass `Some(now_ns)`, both run after \
         every byte of the copy has landed.",
    ),
    (
        "crates/kayfabe-rt/src/cpu_ce.rs",
        "write_completion(",
        1,
        "⚠ THE SECOND DOOR, DEFINITION ONLY — zero production callers, asserted separately by \
         `the_raw_address_completion_door_has_no_production_caller`. It is the raw-address \
         entry point (`&[(GpuVa, u64)]`) and therefore the shape a future caller reaches for. \
         ⊘ Kept rather than deleted because `cpu_ce_executor.rs:847` uses it to PIN that \
         resolve-then-write and write-in-one-pass agree, and a deleted configuration cannot \
         be a control.",
    ),
    // ---- the signal --------------------------------------------------------------------
    (
        "crates/kayfabe-rt/src/cpu_ce.rs",
        "COMPLETION_VECTOR",
        2,
        "The `use` and the one `raise_irq` — raised ONLY after every word of every release \
         has landed, and never at all on a refusal.",
    ),
    (
        "crates/kayfabe-fwd/src/lib.rs",
        "COMPLETION_VECTOR",
        3,
        "The const's definition plus `deliver_completions` and `poll_completions` — both of \
         which have ZERO production callers (their consumer `Executor` is never constructed) \
         and both of which deliver NOTIFIERS, not semaphores. `completion_observer.md` §8.2.",
    ),
];

/// Where production source lives. ⊘ `crates/*/src` only: a test may construct whatever it
/// needs, and a gate that could not tell a test from a shipped path would be tuned until it
/// said nothing.
fn production_files(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root.join("crates")) else {
        panic!("no crates/ under {}", root.display());
    };
    for e in rd.flatten() {
        walk(&e.path().join("src"), root, &mut out);
    }
    out.sort();
    out
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, root, out);
            continue;
        }
        if p.extension().is_none_or(|x| x != "rs") {
            continue;
        }
        let rel = p
            .strip_prefix(root)
            .expect("under the repo root")
            .to_string_lossy()
            .replace('\\', "/");
        if let Ok(text) = std::fs::read_to_string(&p) {
            out.push((rel, strip_comments(&text)));
        }
    }
}

/// ★★ Comment-stripped, because this very file — and `cpu_ce.rs`, and
/// `completion_observer.md`'s quotations of it — name every one of these primitives in prose
/// dozens of times. A scanner that counted those would be tuned down until it counted nothing.
///
/// ⊘ Block comments first, then line comments, and no string-literal awareness: a `//` inside
/// a string would truncate that line. No file in scope has one, and the positive controls
/// below would go red if the stripping stopped working at all.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let b = text.as_bytes();
    let (mut i, mut block) = (0usize, 0usize);
    while i < b.len() {
        if block > 0 {
            if b[i..].starts_with(b"*/") {
                block -= 1;
                i += 2;
            } else if b[i..].starts_with(b"/*") {
                block += 1;
                i += 2;
            } else {
                if b[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            continue;
        }
        if b[i..].starts_with(b"/*") {
            block = 1;
            i += 2;
        } else if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

/// The measured surface, as `(file, primitive, count)`.
fn measured(files: &[(String, String)]) -> BTreeSet<(String, String, usize)> {
    let primitives: BTreeSet<&str> = WRITE_SURFACE.iter().map(|r| r.1).collect();
    let mut out = BTreeSet::new();
    for (rel, text) in files {
        for p in &primitives {
            let n = text.matches(p).count();
            if n > 0 {
                out.insert(((*rel).to_string(), (*p).to_string(), n));
            }
        }
    }
    out
}

fn pinned() -> BTreeSet<(String, String, usize)> {
    WRITE_SURFACE
        .iter()
        .map(|(f, p, n, _)| ((*f).to_string(), (*p).to_string(), *n))
        .collect()
}

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p
}

fn why(file: &str, prim: &str) -> &'static str {
    WRITE_SURFACE
        .iter()
        .find(|r| r.0 == file && r.1 == prim)
        .map_or("⊘ NOT PINNED — this is a NEW writer.", |r| r.3)
}

// =========================================================================================
// The gate
// =========================================================================================

#[test]
fn the_write_surface_is_exactly_the_pinned_set() {
    let root = repo_root();
    let files = production_files(&root);
    assert!(
        files.len() > 50,
        "★ the scanner found only {} production files — it is not looking at the tree, and a \
         census that scans nothing agrees with any pinned table. Root: {}",
        files.len(),
        root.display()
    );
    let (m, p) = (measured(&files), pinned());

    let extra: Vec<_> = m.difference(&p).collect();
    assert!(
        extra.is_empty(),
        "★★★★★ A SECOND WRITER HAS APPEARED, or an existing one grew a call.\n\n\
         Unpinned: {extra:#?}\n\n\
         ⊘ This is the M5.38 defect class. The C artifact read its own two-writer corruption \
         as a MISSING completion for two months; the fix (`ceb13f5`) was to DELETE THE SECOND \
         WRITER, and a backwards semaphore write is fatal on FIRST occurrence because \
         `UVM_ASSERT_MSG_RELEASE` is in release builds.\n\n\
         There are exactly two honest fixes:\n\
         (1) route the write through `cpu_ce::write_resolved_completion` so it takes a \
         `ResolvedRelease` the resolver minted and inherits resolve-all → write-all → signal; \
         or\n\
         (2) add a row to `WRITE_SURFACE` stating, in its `why`, why this writer cannot \
         reach a completion address.\n\
         ⊘ Neither `#[allow]` nor widening a count without a `why` is one of them. See \
         `docs/design/completion_observer.md` §8 and `docs/design/gr_execution_boundary.md`."
    );

    let missing: Vec<_> = p.difference(&m).collect();
    assert!(
        missing.is_empty(),
        "★★ A PINNED WRITER IS GONE OR MOVED: {missing:#?}\n\n\
         ⊘ Do not simply delete the row. A writer that vanished is either a real \
         simplification — in which case delete the row AND say so in the commit — or the \
         scanner has stopped seeing it, which is the failure mode `unranked_locks.rs` \
         recorded on 2026-08-09 (a `rustfmt`-wrapped field the line scanner could not see). \
         Check the second possibility first: `suspect_the_instrument_first`."
    );
}

#[test]
fn the_raw_address_completion_door_has_no_production_caller() {
    let root = repo_root();
    let files = production_files(&root);
    // The definition lives in `cpu_ce.rs`; every OTHER occurrence in production source is a
    // caller, and there must be none.
    let callers: Vec<&String> = files
        .iter()
        .filter(|(rel, text)| {
            rel != "crates/kayfabe-rt/src/cpu_ce.rs" && text.contains("write_completion(")
        })
        .map(|(rel, _)| rel)
        .collect();
    assert!(
        callers.is_empty(),
        "★★★ `cpu_ce::write_completion` has acquired a production caller: {callers:?}\n\n\
         ⊘ It is the RAW-ADDRESS door — `&[(GpuVa, u64)]`, no `ResolvedRelease`, no \
         timestamp, `None` for the clock. `completion_observer.md` §8.1 recorded it as \"the \
         second door, ZERO production callers\", and §8.4 said the property was one refactor \
         from untrue. This is that refactor.\n\
         ⇒ Use `resolve_releases` + `write_resolved_completion` instead: the point is not the \
         convenience, it is that the address came from the guest's OWN page tables and every \
         word of the record resolved before any byte was written. {}",
        why("crates/kayfabe-rt/src/cpu_ce.rs", "write_completion(")
    );
}

#[test]
fn the_write_primitive_is_private() {
    let root = repo_root();
    let text = strip_comments(
        &std::fs::read_to_string(root.join("crates/kayfabe-rt/src/cpu_ce.rs"))
            .expect("cpu_ce.rs is readable"),
    );
    assert!(
        text.contains("fn write_plane("),
        "★ `write_plane` is gone or renamed — see the missing-row message in \
         `the_write_surface_is_exactly_the_pinned_set`."
    );
    assert!(
        !text.contains("pub fn write_plane(") && !text.contains("pub(crate) fn write_plane("),
        "★★★★★ `write_plane` HAS BEEN PUBLISHED.\n\n\
         It is the single primitive every byte the guest can read passes through, and its \
         privacy is the reason the pinned call set in `WRITE_SURFACE` is the whole story. \
         `pub(crate)` opens it to `ceutils.rs`, `device.rs` and every future module in \
         `kayfabe-rt` WITHOUT changing any count in this file, which is precisely why this is \
         a separate assertion and not a row in the table.\n\
         ⇒ If a new caller genuinely needs to write guest-visible bytes, give it a named \
         function beside `execute_ours` / `write_resolved_completion` that states its \
         discipline, and pin THAT."
    );
}

#[test]
fn the_completion_signal_never_precedes_the_completion_write() {
    let root = repo_root();
    let text = strip_comments(
        &std::fs::read_to_string(root.join("crates/kayfabe-rt/src/cpu_ce.rs"))
            .expect("cpu_ce.rs is readable"),
    );
    let (w, s) = (
        text.rfind("write_plane(").expect("the write loop is there"),
        text.rfind("raise_irq(").expect("the signal is there"),
    );
    assert!(
        w < s,
        "★★★ THE ORDER HAS INVERTED — `raise_irq` now precedes the last `write_plane` in \
         `cpu_ce.rs`.\n\n\
         ⊘ *resolve → write → signal* is the whole ordering argument of the completion tail \
         (`ResolvedRelease`'s own doc). A guest woken before its payload has landed reads the \
         OLD value, and on a monotonic tracking semaphore that is indistinguishable from a \
         backwards write — the M5.38 shape, arrived at from the other end.\n\
         ⚠ This assertion is TEXTUAL and therefore weak: it pins the order of two tokens in \
         one file, not the order of two effects. It is here because it costs nothing and \
         because the strong version needs a fifth `FbWriter` kind (see the `.write_tagged(` \
         row's residual). ⊘ Do not cite it as proof of ordering; cite it as a tripwire."
    );
}

// =========================================================================================
// ★★★ THE POSITIVE CONTROLS — a gate nobody has bitten is a gate nobody has tested
// =========================================================================================

#[test]
fn the_census_bites_a_new_writer() {
    let synthetic = vec![(
        "crates/kayfabe-rt/src/a_new_module.rs".to_string(),
        strip_comments("fn forge(v: &mut dyn Vmm) { v.gpa_write(0x2_0440_fff0, &[1,0,0,0]); }"),
    )];
    let m = measured(&synthetic);
    assert!(
        m.contains(&(
            "crates/kayfabe-rt/src/a_new_module.rs".to_string(),
            ".gpa_write(".to_string(),
            1
        )),
        "★ THE SCANNER IS BLIND. It did not see a `gpa_write` in a file it was handed, so \
         every green run of the gate above is green by construction. Saw: {m:#?}"
    );
    assert!(
        m.difference(&pinned()).count() == 1,
        "the synthetic writer must be exactly one unpinned row"
    );
}

#[test]
fn the_census_bites_a_writer_that_rustfmt_has_wrapped() {
    // ⚠ THE SPELLING THAT BEAT `unranked_locks.rs` on 2026-08-09: `rustfmt` breaks a long
    // expression so the name and the call land on different lines. A line-by-line scanner
    // reports the file as clean; this one strips comments and scans the whole text.
    let synthetic = vec![(
        "crates/kayfabe-rt/src/wrapped.rs".to_string(),
        strip_comments(
            "fn f(v: &mut dyn Vmm, a: u64) {\n    v\n        .gpa_write(\n            a,\n \
             &[1, 0, 0, 0],\n        )\n        .ok();\n}",
        ),
    )];
    assert!(
        measured(&synthetic).contains(&(
            "crates/kayfabe-rt/src/wrapped.rs".to_string(),
            ".gpa_write(".to_string(),
            1
        )),
        "★★ The scanner is blind to `rustfmt`'s wrapping — the exact blindness \
         `unranked_locks.rs` was measured to have, reproduced one gate over."
    );
}

#[test]
fn the_census_does_not_count_a_writer_that_is_only_mentioned() {
    // ⊘ The mirror control. This very file, `cpu_ce.rs` and `completion_observer.md` name
    // these primitives in prose dozens of times. A gate that counted prose would be tuned
    // down until it counted nothing — which is how a gate dies without going red.
    let synthetic = vec![(
        "crates/kayfabe-rt/src/prose.rs".to_string(),
        strip_comments(
            "/// The one writer is `write_plane`, which calls `.gpa_write(` for guest RAM.\n\
             // A second `.gpa_write(` here would be the M5.38 defect.\n/* .gpa_write( */\n\
             fn nothing() {}",
        ),
    )];
    assert!(
        measured(&synthetic).is_empty(),
        "★ The scanner counts PROSE. Every doc comment naming a primitive would read as a \
         writer, so the pinned table would have to absorb the documentation — and a table \
         nobody can read is a table nobody checks. Saw: {:#?}",
        measured(&synthetic)
    );
}
