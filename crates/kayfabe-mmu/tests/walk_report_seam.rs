//! ★★★★★ **THE C↔RUST SEAM, ACROSS A REAL C COMPILER.**
//!
//! `[w725]` Before this test, `grep -rn "ReportHeader\|kf_report" crates/ --include=*.rs`
//! returned **nothing**. The walk kernel's report — the struct increment 6 consumes — was
//! declared in `cuda/walk/kf_walk.h` and nowhere else; the differential oracle
//! (`walk_kernel_differential.rs`) compares through a *corpus file* written by a third
//! program, so it never touches the report ABI at all.
//!
//! # ⊘⊘⊘ WHAT A ROUND TRIP AGAINST ITSELF CANNOT CATCH
//!
//! `NVOS34` (`kayfabe-abi/src/submit.rs`): a field at **+12 instead of +16** still encodes,
//! still decodes, still round-trips, and hands the driver a cookie it never minted. Pinning
//! offsets in Rust catches it **only if the pins are right**, and the only authority on that is
//! the C compiler that lays out the struct the kernel writes.
//!
//! ⇒ This test compiles `cuda/walk/kf_report_emit.c` against the **real** `kf_walk.h`, runs it,
//! and:
//!
//! 1. compares the C compiler's `offsetof`/`sizeof`/`_Alignof` against
//!    [`kayfabe_mmu::walkreport`]'s pinned byte ranges — **field by field**;
//! 2. compares every `KFWR_*` constant against the Rust copy;
//! 3. parses the bytes the emitter wrote and checks **every field of every record** against the
//!    emitter's own independently-printed account of what it put there; and
//! 4. feeds the parsed runs into [`kayfabe_mmu::walkdiff`], so the whole path is exercised.
//!
//! # ★ The known-positive, and it is in this file
//!
//! [`a_shifted_field_is_caught_by_the_offset_comparison`] rebuilds the emitter with
//! `-DKF_SEAM_BREAK_LAYOUT`, which inserts a 4-byte hole before `MapRun::flags` —
//! **the NVOS34 defect transplanted** — and requires this test's comparison to catch it, at the
//! offset assertion and not somewhere downstream. Without that, every assertion below is a
//! check nobody has ever seen fire.
//!
//! ⚠ **This test never skips.** A missing C compiler is a hard failure, because a skip is
//! indistinguishable from a pass in every report anyone reads.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use kayfabe_mmu::walkdiff::{PageClass, apply, diff};
use kayfabe_mmu::walkreport::{
    self as wr, MapRun, PdbEntry, Report, ReportHeader, RunAperture, RunOp,
};

/// Where `cuda/walk` lives, relative to this crate.
fn walk_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cuda/walk-v2-frozen")
}

/// The C compiler to use. ⊘ Not optional: `cargo test` already needed a linker to get here, so
/// "no compiler" means the environment is broken, not that this test is inapplicable.
fn cc() -> String {
    if let Some(v) = std::env::var_os("CC") {
        return v.to_string_lossy().into_owned();
    }
    for c in ["cc", "gcc", "clang"] {
        if Command::new(c)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return c.to_string();
        }
    }
    panic!(
        "no C compiler (tried $CC, cc, gcc, clang). This test does NOT skip: the report ABI is \
         unchecked without it, and a skip reads as a pass."
    );
}

/// Everything the emitter printed, split by its first word.
struct Manifest {
    /// `struct <name> -> (sizeof, alignof)`.
    structs: BTreeMap<String, (usize, usize)>,
    /// `(struct, field) -> (offset, size)`.
    fields: BTreeMap<(String, String), (usize, usize)>,
    /// `const <name> -> value`.
    consts: BTreeMap<String, u64>,
    /// `hdr <field> <words...>`.
    hdr: BTreeMap<String, Vec<u64>>,
    /// `pdb <i> ...` and `run <i> ...`, in index order.
    pdbs: Vec<Vec<u64>>,
    runs: Vec<Vec<u64>>,
    ok: bool,
}

fn parse_manifest(text: &str) -> Manifest {
    let mut m = Manifest {
        structs: BTreeMap::new(),
        fields: BTreeMap::new(),
        consts: BTreeMap::new(),
        hdr: BTreeMap::new(),
        pdbs: Vec::new(),
        runs: Vec::new(),
        ok: false,
    };
    for line in text.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.as_slice() {
            ["struct", name, sz, al] => {
                m.structs.insert(
                    (*name).to_string(),
                    (sz.parse().unwrap(), al.parse().unwrap()),
                );
            }
            ["field", st, fld, off, sz] => {
                m.fields.insert(
                    ((*st).to_string(), (*fld).to_string()),
                    (off.parse().unwrap(), sz.parse().unwrap()),
                );
            }
            ["const", name, v] => {
                m.consts.insert((*name).to_string(), v.parse().unwrap());
            }
            ["hdr", fld, rest @ ..] => {
                m.hdr.insert(
                    (*fld).to_string(),
                    rest.iter().map(|s| s.parse().unwrap()).collect(),
                );
            }
            ["pdb", rest @ ..] => m
                .pdbs
                .push(rest.iter().map(|s| s.parse().unwrap()).collect()),
            ["run", rest @ ..] => m
                .runs
                .push(rest.iter().map(|s| s.parse().unwrap()).collect()),
            ["EMIT_OK"] => m.ok = true,
            _ => {}
        }
    }
    m
}

/// Build the emitter and run it. Returns `(manifest, report bytes)`.
///
/// ⚠ `tag` must be UNIQUE PER TEST: `cargo test` runs these in parallel threads, and two tests
/// sharing an output path race as one rewrites the executable the other is exec()ing
/// (`ExecutableFileBusy`). Measured, w725.
///
/// ⚠ Every failure here is loud and names its own cause: a build that fails, a run that fails,
/// and a run that produces no `EMIT_OK` terminator are three different states, and *"the file is
/// empty"* must never be read as *"nothing was wrong"*.
fn build_and_run(defines: &[&str], tag: &str) -> (Manifest, Vec<u8>) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("walk_report_seam_{tag}"));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let exe = dir.join("kf_report_emit");
    let bin = dir.join("kf_report.bin");
    let src = walk_dir().join("kf_report_emit.c");
    assert!(src.exists(), "{} is missing", src.display());

    let mut c = Command::new(cc());
    c.arg("-O1")
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror");
    for d in defines {
        c.arg(d);
    }
    c.arg("-I").arg(walk_dir()).arg("-o").arg(&exe).arg(&src);
    let out = c.output().expect("spawn the C compiler");
    assert!(
        out.status.success(),
        "compiling kf_report_emit.c [{tag}] failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = Command::new(&exe)
        .arg(&bin)
        .output()
        .expect("run the emitter");
    assert!(
        out.status.success(),
        "kf_report_emit [{tag}] exited {:?}:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let m = parse_manifest(&text);
    assert!(
        m.ok,
        "kf_report_emit [{tag}] produced no EMIT_OK terminator -- it did not finish"
    );
    let bytes = std::fs::read(&bin).expect("the emitted report");
    assert!(
        !bytes.is_empty(),
        "the emitted report is ZERO BYTES -- that is a state, not 'not yet'"
    );
    (m, bytes)
}

/// ★★★ Compare the C compiler's layout against this crate's pins. Returns the list of
/// disagreements rather than asserting, so the known-positive can require it to be non-empty.
fn layout_disagreements(m: &Manifest) -> Vec<String> {
    let mut bad = Vec::new();
    let mut want_struct = |name: &str, size: usize| match m.structs.get(name) {
        None => bad.push(format!("the emitter printed no layout for {name}")),
        Some(&(sz, al)) => {
            if sz != size {
                bad.push(format!("sizeof({name}) = {sz} in C, {size} pinned in Rust"));
            }
            if al != 8 {
                bad.push(format!(
                    "_Alignof({name}) = {al}, want 8 -- a padding change"
                ));
            }
        }
    };
    want_struct("KfReportHeader", ReportHeader::SIZE);
    want_struct("KfPdbEntry", PdbEntry::SIZE);
    want_struct("KfMapRun", MapRun::SIZE);

    // The pins. ⊘ Written out longhand on purpose: a table generated from the same source as
    // the parser would agree with it by construction and prove nothing.
    let pins: &[(&str, &str, usize, usize)] = &[
        ("KfReportHeader", "magic", 0, 4),
        ("KfReportHeader", "version", 4, 2),
        ("KfReportHeader", "flags", 6, 2),
        ("KfReportHeader", "generation", 8, 8),
        ("KfReportHeader", "acked_generation", 16, 8),
        ("KfReportHeader", "pdb_count", 24, 4),
        ("KfReportHeader", "pdb_capacity", 28, 4),
        ("KfReportHeader", "run_count", 32, 4),
        ("KfReportHeader", "run_capacity", 36, 4),
        ("KfReportHeader", "entries_visited", 40, 8),
        ("KfReportHeader", "refusals", 48, 4),
        ("KfReportHeader", "refuse_mask", 52, 4),
        ("KfReportHeader", "sparse_slots", 56, 4),
        ("KfReportHeader", "ps_log2", 60, 4),
        ("KfPdbEntry", "pdb", 0, 8),
        ("KfPdbEntry", "first_run", 8, 4),
        ("KfPdbEntry", "run_count", 12, 4),
        ("KfPdbEntry", "vas_flags", 16, 4),
        ("KfPdbEntry", "reserved", 20, 4),
        ("KfPdbEntry", "reserved2", 24, 8),
        ("KfMapRun", "va", 0, 8),
        ("KfMapRun", "gpga", 8, 8),
        ("KfMapRun", "len", 16, 8),
        ("KfMapRun", "flags", 24, 4),
        ("KfMapRun", "op", 28, 2),
        ("KfMapRun", "pdb_index", 30, 2),
    ];
    for &(st, fld, off, sz) in pins {
        match m.fields.get(&(st.to_string(), fld.to_string())) {
            None => bad.push(format!("{st}::{fld} is absent from the C layout manifest")),
            Some(&(c_off, c_sz)) => {
                if c_off != off {
                    bad.push(format!(
                        "★ {st}::{fld} is at +{c_off} in C, pinned at +{off} in Rust"
                    ));
                }
                if c_sz != sz {
                    bad.push(format!(
                        "{st}::{fld} is {c_sz} bytes in C, {sz} pinned in Rust"
                    ));
                }
            }
        }
    }
    bad
}

#[test]
fn the_c_struct_layout_is_the_layout_rust_pins() {
    let (m, _) = build_and_run(&[], "layout");
    let bad = layout_disagreements(&m);
    assert!(
        bad.is_empty(),
        "the C compiler and the Rust parser disagree:\n  {}",
        bad.join("\n  ")
    );

    // Sizes sum to the whole struct with nothing left over — i.e. there is no padding anywhere,
    // which is the property that makes the pins a complete description rather than a sample.
    let sum = |st: &str| -> usize {
        m.fields
            .iter()
            .filter(|((s, _), _)| s == st)
            .map(|(_, &(_, sz))| sz)
            .sum()
    };
    assert_eq!(
        sum("KfReportHeader"),
        64,
        "KfReportHeader has hidden padding"
    );
    assert_eq!(sum("KfPdbEntry"), 32, "KfPdbEntry has hidden padding");
    assert_eq!(sum("KfMapRun"), 32, "KfMapRun has hidden padding");
}

#[test]
fn every_kfwr_constant_matches_the_header() {
    let (m, _) = build_and_run(&[], "consts");
    let want: &[(&str, u64)] = &[
        ("MAGIC", u64::from(wr::MAGIC)),
        ("VERSION", u64::from(wr::VERSION)),
        ("HF_TRUNCATED", u64::from(wr::HF_TRUNCATED)),
        ("HF_RESYNC", u64::from(wr::HF_RESYNC)),
        ("HF_REFUSED", u64::from(wr::HF_REFUSED)),
        ("HF_BUDGET", u64::from(wr::HF_BUDGET)),
        ("HF_SCOPED", u64::from(wr::HF_SCOPED)),
        ("HF_PDB_TRUNCATED", u64::from(wr::HF_PDB_TRUNCATED)),
        ("HF_SCOPE_DEGRADED", u64::from(wr::HF_SCOPE_DEGRADED)),
        ("R_OOB", u64::from(wr::R_OOB)),
        ("R_UNALIGNED", u64::from(wr::R_UNALIGNED)),
        ("R_FOREIGN_AP", u64::from(wr::R_FOREIGN_AP)),
        ("R_TOO_DEEP", u64::from(wr::R_TOO_DEEP)),
        ("R_RUN_CAP", u64::from(wr::R_RUN_CAP)),
        ("R_BUDGET", u64::from(wr::R_BUDGET)),
        ("R_PDB_CAP", u64::from(wr::R_PDB_CAP)),
        ("R_BAD_SCOPE", u64::from(wr::R_BAD_SCOPE)),
        ("R_PDB_UNSORTED", u64::from(wr::R_PDB_UNSORTED)),
        ("R_DELTA_CAP", u64::from(wr::R_DELTA_CAP)),
        ("R_MISALIGNED_LEAF", u64::from(wr::R_MISALIGNED_LEAF)),
        ("R_BAD_FORMAT", u64::from(wr::R_BAD_FORMAT)),
        ("V_NEW", u64::from(wr::V_NEW)),
        ("V_GONE", u64::from(wr::V_GONE)),
        ("V_RESYNC", u64::from(wr::V_RESYNC)),
        ("OP_MAP", u64::from(RunOp::Map.code())),
        ("OP_UNMAP", u64::from(RunOp::Unmap.code())),
        ("OP_REMAP", u64::from(RunOp::Remap.code())),
        ("RF_AP_SHIFT", u64::from(wr::RF_AP_SHIFT)),
        ("RF_AP_MASK", u64::from(wr::RF_AP_MASK)),
        ("RF_READ_ONLY", u64::from(wr::RF_READ_ONLY)),
        ("RF_ATOMIC_DISABLE", u64::from(wr::RF_ATOMIC_DISABLE)),
        ("RF_VOLATILE", u64::from(wr::RF_VOLATILE)),
        ("RF_PRIVILEGE", u64::from(wr::RF_PRIVILEGE)),
        ("RF_PS_SHIFT", u64::from(wr::RF_PS_SHIFT)),
        ("RF_PS_MASK", u64::from(wr::RF_PS_MASK)),
    ];
    for &(name, v) in want {
        let c = *m
            .consts
            .get(name)
            .unwrap_or_else(|| panic!("{name} absent from the manifest"));
        assert_eq!(c, v, "KFWR_{name}: {c} in C, {v} in Rust");
    }
    // The four aperture codes and four page-size codes are positional, so they are checked as
    // an ordered set rather than one at a time.
    assert_eq!(
        [
            m.consts["AP_VIDMEM"],
            m.consts["AP_PEER"],
            m.consts["AP_SYSCOH"],
            m.consts["AP_SYSNONCOH"]
        ],
        [0, 1, 2, 3],
        "the aperture codes"
    );
    assert_eq!(
        [
            m.consts["PS_4K"],
            m.consts["PS_64K"],
            m.consts["PS_2M"],
            m.consts["PS_512M"]
        ],
        [0, 1, 2, 3],
        "the page-size codes"
    );
}

#[test]
fn the_rust_parser_reads_back_exactly_what_the_c_side_wrote() {
    let (m, bytes) = build_and_run(&[], "values");
    let rep = Report::parse(&bytes).expect("parse the C-emitted report");

    // ── the header, field by field, against the emitter's own account ────────────────────
    let h = &rep.header;
    let g = |k: &str| -> u64 { m.hdr[k][0] };
    assert_eq!(u64::from(h.magic), g("magic"));
    assert_eq!(u64::from(h.version), g("version"));
    assert_eq!(u64::from(h.flags), g("flags"));
    assert_eq!(h.generation, g("generation"));
    assert_eq!(h.acked_generation, g("acked_generation"));
    assert_eq!(u64::from(h.pdb_count), g("pdb_count"));
    assert_eq!(u64::from(h.pdb_capacity), g("pdb_capacity"));
    assert_eq!(u64::from(h.run_count), g("run_count"));
    assert_eq!(u64::from(h.run_capacity), g("run_capacity"));
    assert_eq!(h.entries_visited, g("entries_visited"));
    assert_eq!(u64::from(h.refusals), g("refusals"));
    assert_eq!(u64::from(h.refuse_mask), g("refuse_mask"));
    assert_eq!(u64::from(h.sparse_slots), g("sparse_slots"));
    assert_eq!(
        h.ps_log2.map(u64::from).to_vec(),
        m.hdr["ps_log2"],
        "★ ps_log2 is FOUR BYTES, and reading it as one u32 gives the right length and the \
         wrong four numbers"
    );

    // ⊘ The values themselves are distinctive, so this is also a check that nothing was read
    // from the wrong offset and happened to match.
    assert_eq!(h.generation, 0x0123_4567_89AB_CDEF);
    assert_eq!(h.refuse_mask, wr::R_MISALIGNED_LEAF | wr::R_BUDGET);
    assert!(h.is_resync() && !h.is_truncated());

    // ── the PdbEntry array ───────────────────────────────────────────────────────────────
    assert_eq!(rep.pdbs.len(), m.pdbs.len(), "PdbEntry count");
    for (i, p) in rep.pdbs.iter().enumerate() {
        let c = &m.pdbs[i];
        assert_eq!(c[0], i as u64, "manifest index");
        assert_eq!(p.pdb, c[1], "pdbs[{i}].pdb");
        assert_eq!(u64::from(p.first_run), c[2], "pdbs[{i}].first_run");
        assert_eq!(u64::from(p.run_count), c[3], "pdbs[{i}].run_count");
        assert_eq!(u64::from(p.vas_flags), c[4], "pdbs[{i}].vas_flags");
        assert_eq!(u64::from(p.reserved), c[5], "pdbs[{i}].reserved");
        assert_eq!(p.reserved2, c[6], "pdbs[{i}].reserved2");
        assert_eq!(
            (p.reserved, p.reserved2),
            (0, 0),
            "★ the reserved words must stay zero"
        );
    }
    assert!(
        rep.pdbs[2].is_gone() && rep.pdbs[2].run_count == 0,
        "⊘ GONE carries no runs"
    );

    // ── the MapRun array ─────────────────────────────────────────────────────────────────
    assert_eq!(rep.runs.len(), m.runs.len(), "MapRun count");
    for (i, r) in rep.runs.iter().enumerate() {
        let c = &m.runs[i];
        assert_eq!(c[0], i as u64, "manifest index");
        assert_eq!(r.va, c[1], "runs[{i}].va");
        assert_eq!(r.gpga, c[2], "runs[{i}].gpga");
        assert_eq!(r.len, c[3], "runs[{i}].len");
        assert_eq!(u64::from(r.flags), c[4], "runs[{i}].flags");
        assert_eq!(
            u64::from(r.op),
            c[5],
            "★ runs[{i}].op -- a u16, not the low half of flags"
        );
        assert_eq!(u64::from(r.pdb_index), c[6], "★ runs[{i}].pdb_index");
    }

    // ── the DECODED views, which are what a consumer actually uses ───────────────────────
    assert_eq!(rep.runs[0].page_size(h, 0).expect("ps"), 4096);
    assert_eq!(rep.runs[1].page_size(h, 1).expect("ps"), 65_536);
    assert_eq!(rep.runs[2].page_size(h, 2).expect("ps"), 0x0020_0000);
    assert_eq!(rep.runs[3].page_size(h, 3).expect("ps"), 0x2000_0000);
    assert_eq!(rep.runs[0].aperture(), RunAperture::Vidmem);
    assert_eq!(rep.runs[1].aperture(), RunAperture::SysmemCoherent);
    assert_eq!(rep.runs[2].aperture(), RunAperture::SysmemNonCoherent);
    assert_eq!(rep.runs[3].aperture(), RunAperture::Peer);
    assert!(rep.runs[1].read_only() && !rep.runs[1].is_volatile());
    assert!(rep.runs[2].is_volatile() && !rep.runs[2].read_only());
    assert!(rep.runs[3].atomic_disable());
    let all = &rep.runs[4];
    assert!(all.read_only() && all.atomic_disable() && all.is_volatile() && all.privileged());
    assert_eq!(rep.runs[5].op_decoded(5).expect("op"), RunOp::Map);
    assert_eq!(rep.runs[6].op_decoded(6).expect("op"), RunOp::Unmap);
    assert_eq!(rep.runs[7].op_decoded(7).expect("op"), RunOp::Remap);

    // ★★★ The 512 MiB run whose target is only 4 KiB-aligned parses. VER2 can spell it, so a
    // host that refused it would be asserting an alignment the encoding cannot carry.
    assert_eq!(
        rep.runs[3].gpga, 0x3000,
        "a 4 KiB-aligned 512 MiB target must survive the parse"
    );

    // ── and the same bytes read as three separate arrays, which is what kf_refresh gives ──
    let pdb_at = Report::pdb_array_offset();
    let run_at = Report::run_array_offset(h.pdb_count);
    let parts = Report::parse_parts(&bytes[..pdb_at], &bytes[pdb_at..run_at], &bytes[run_at..])
        .expect("parse_parts");
    assert_eq!(parts, rep, "the packed and three-array readings must agree");
}

#[test]
fn the_parsed_runs_reach_walkdiff_and_the_diff_closes() {
    let (_, bytes) = build_and_run(&[], "diff");
    let rep = Report::parse(&bytes).expect("parse");

    let cur = rep.present_runs().expect("present runs");
    // 9 runs, one of which is an UNMAP and therefore not a present mapping.
    assert_eq!(cur.len(), 8, "MAP and REMAP are present; UNMAP is not");
    let classes: Vec<PageClass> = cur.iter().map(|r| r.class).collect();
    assert!(
        classes.contains(&PageClass::P4K),
        "the corpus must exercise 4 KiB"
    );
    assert!(classes.contains(&PageClass::P64K), "…64 KiB");
    assert!(classes.contains(&PageClass::P2M), "…2 MiB");
    assert!(classes.contains(&PageClass::P512M), "…and 512 MiB");

    // apply(∅, diff(∅, cur)) == cur — the closure property, over runs that came out of C.
    let ops = diff(&[], &cur);
    assert!(
        !ops.is_empty(),
        "⚠ an empty diff would make this assertion vacuous"
    );
    let mut got = apply(&[], &ops);
    let mut want = cur.clone();
    got.sort_by_key(|r| (r.class, r.va));
    want.sort_by_key(|r| (r.class, r.va));
    assert_eq!(got, want, "apply(∅, diff(∅, C-emitted runs)) != those runs");

    // …and order-independence, which is the property the segment diff exists to provide.
    let mut rev = ops.clone();
    rev.reverse();
    let mut back = apply(&[], &rev);
    back.sort_by_key(|r| (r.class, r.va));
    assert_eq!(
        back, want,
        "applying the ops backwards gives a different mapping set"
    );

    // A second, different mapping set, so the diff is exercised on all three ops rather than
    // only on MAP. Drop one run, move another, and change a third's flags.
    let mut next = cur.clone();
    next.remove(0);
    next[0].gpga += 0x10_0000;
    next[1].flags ^= wr::RF_READ_ONLY;
    let ops2 = diff(&cur, &next);
    let kinds = ops2.iter().map(std::mem::discriminant).collect::<Vec<_>>();
    assert!(
        kinds.len() >= 3,
        "expected at least an unmap, a remap and a flag change"
    );
    let mut got2 = apply(&cur, &ops2);
    let mut want2 = next;
    got2.sort_by_key(|r| (r.class, r.va));
    want2.sort_by_key(|r| (r.class, r.va));
    assert_eq!(got2, want2, "apply(prev, diff(prev, cur)) != cur");
}

/// ★★★★★ **THE KNOWN-POSITIVE.** Rebuild the emitter with a 4-byte hole inserted before
/// `MapRun::flags` — the `NVOS34` defect, transplanted into this struct — and require the
/// layout comparison to catch it, **naming that field**.
///
/// ⚠ It must also be caught for the RIGHT REASON. A broken build that failed to compile, or
/// whose *report* happened to parse differently, would satisfy a bare "it failed" check while
/// leaving the offset comparison itself unexercised. So this asserts three things: the broken
/// emitter still builds and runs, the disagreement list is non-empty, and the disagreement
/// **names `KfMapRun::flags` and its wrong offset**.
#[test]
fn a_shifted_field_is_caught_by_the_offset_comparison() {
    let (control, _) = build_and_run(&[], "kp_control");
    assert!(
        layout_disagreements(&control).is_empty(),
        "the CONTROL must be clean, or this known-positive proves nothing"
    );

    let (broken, broken_bytes) = build_and_run(&["-DKF_SEAM_BREAK_LAYOUT"], "kp_broken");
    assert!(
        !broken_bytes.is_empty(),
        "the broken emitter still has to produce a report"
    );

    let bad = layout_disagreements(&broken);
    assert!(
        !bad.is_empty(),
        "⊘⊘⊘ A FIELD MOVED FOUR BYTES AND THE COMPARISON DID NOT NOTICE. Every offset \
         assertion in this file is vacuous."
    );
    let names_the_field = bad
        .iter()
        .any(|s| s.contains("KfMapRun::flags") && s.contains("+28"));
    assert!(
        names_the_field,
        "the break was caught, but NOT by the offset check on the field that moved -- wrong \
         reason:\n  {}",
        bad.join("\n  ")
    );
    // And the struct grew, which is the second, independent signal.
    assert_eq!(
        broken.structs["KfMapRun"].0, 40,
        "the broken struct should be 8 bytes longer (4 of hole + 4 of tail padding)"
    );
}

/// ⊘ A guard on the file this test depends on: if `kf_walk.h` grows a field, the emitter's
/// manifest stops covering the struct and the padding sum above starts failing. This states
/// the dependency explicitly so the failure reads as *"the ABI changed"*.
#[test]
fn the_header_this_test_pins_is_the_one_in_the_tree() {
    let h = walk_dir().join("kf_walk.h");
    let text = std::fs::read_to_string(&h).unwrap_or_else(|e| panic!("{}: {e}", h.display()));
    for field in [
        "uint32_t magic;",
        "uint16_t version;",
        "uint64_t generation;",
        "uint32_t refuse_mask;",
        "uint32_t sparse_slots;",
        "uint8_t  ps_log2[4];",
    ] {
        assert!(
            text.contains(field),
            "kf_walk.h no longer declares `{field}`"
        );
    }
    assert!(
        Path::new(&walk_dir().join("kf_report_emit.c")).exists(),
        "the C half of this seam is missing"
    );
}
