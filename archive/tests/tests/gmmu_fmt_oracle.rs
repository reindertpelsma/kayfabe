//! # The GMMU page-table format, judged against NVIDIA's OWN encoder
//!
//! `kayfabe_chips::Ga10xGmmu` decodes GA10x page-table entries. Its unit tests
//! (`crates/kayfabe-chips/tests/ga10x_gmmu.rs`) construct the entries it decodes — so
//! decoder and test corpus were written by reading the same C, and *"a transcribed parser
//! cannot detect a shared misreading, by construction"* (`tests/oracle/vbios_oracle.c`).
//!
//! ★★★ **The size of the hole, measured.** `#149` landed the first real GMMU
//! translations: 12 402 writes and 56 reads resolved through this decoder on a live guest
//! boot. Sixteen of those 12 402 were checked by anything at all. Everything else rested
//! on the decoder and its corpus being wrong in the same direction — which is exactly the
//! failure `#13` was: on GA10x, `PD0`'s entry is a SIXTEEN-byte dual entry naming two
//! sub-tables, and `PD1` is itself a 512 MiB leaf level. That misreading cost weeks.
//!
//! These tests remove the hole rather than shrinking it. `tests/oracle/gmmu_fmt_oracle.c`
//! compiles the driver's ACTUAL format encoder — `kgmmuFmtInitLevels_GA10X`,
//! `kgmmuFmtInitPde{,Multi}_GP10X`, `kgmmuFmtInitPte_GP10X`, the aperture tables, and
//! `gmmu_fmt.c`'s own `gmmuFmtEntryIsPte` / `gmmuFmtGetPde` / address-field selectors —
//! unmodified, out of the vendored open kernel modules, with the per-chip HAL binding
//! *derived from the driver's own dispatch table* rather than chosen. Every judgement
//! below is that code's, not a transcription of it.
//!
//! ## The gate, and its honest limit
//!
//! Every test prints `GMMU-ORACLE-GATE: RAN <name>` or `GMMU-ORACLE-GATE: SKIPPED <name>
//! — …` to stderr in **both** arms, and CI counts RAN+SKIPPED against a floor. GitHub's
//! runners do not have the vendored trees and nothing here stands in for them, so on CI
//! this suite is counted and never passes: it is a developer-box and bench gate. That is
//! the KVM gate's failure mode repeated knowingly, and the floor is the only thing that
//! stops the tests vanishing from both places at once.
//!
//! ## ⊘ What this does NOT establish
//!
//! - **Nothing about the walk.** The oracle judges *one entry at a time* plus the level
//!   geometry. `kayfabe_mmu::walker`'s traversal, its fault taxonomy and
//!   `leaf_disposition`'s policy are untouched — and `#13`'s round-4 bug lived in the
//!   policy half, not the decode half.
//! - **Nothing about `GMMU_FMT_VERSION_3`** (Hopper+), and nothing about `VERSION_1`.
//! - **Nothing about the 610.43.02 tree.** Its `gmmuFmtPtePhysAddrFld` takes an
//!   `MMU_FMT_LEVEL *` and a `GMMU_PEER_TYPE` that 580.159.04's does not; `tests/build.rs`
//!   detects the arity and skips that tree by name. `ogkm` is VERSIONED, not the spec.
//! - **Nothing about what the GUEST actually writes.** A decoder that agrees with RM's
//!   encoder on every bit pattern still says nothing about whether the bytes we read are
//!   the bytes the guest wrote.

#![allow(clippy::too_many_lines)]

use kayfabe_arch::{Aperture, GmmuFmt, PteDecode};
use kayfabe_chips::Ga10xGmmu;
use std::collections::BTreeMap;
use std::io::Write as _;
use std::process::{Command, Stdio};

// ===========================================================================================
// The gate
// ===========================================================================================

/// The oracles this build has, as `(tag, path)`. Empty when no vendored tree served one.
///
/// `option_env!` and not `env!`: a machine without the trees still builds and tests
/// everything else.
fn oracles() -> Vec<(&'static str, &'static str)> {
    [
        ("ogkm-580.159.04", option_env!("KAYFABE_GMMU_ORACLE_580")),
        (
            "ogkm-610 (610.43.02)",
            option_env!("KAYFABE_GMMU_ORACLE_610"),
        ),
    ]
    .into_iter()
    .filter_map(|(tag, p)| p.map(|p| (tag, p)))
    .collect()
}

/// Emit this test's gate line. Straight to `stderr` rather than through `eprintln!`'s
/// capture-aware path, so the **passing** arm is visible too — a gate whose "it ran"
/// marker only appears on failure cannot be counted, and counting it is the whole
/// non-vacuity argument.
fn report(test: &str, available: bool) {
    let mut err = std::io::stderr();
    let _ = if available {
        writeln!(err, "GMMU-ORACLE-GATE: RAN {test}")
    } else {
        writeln!(
            err,
            "GMMU-ORACLE-GATE: SKIPPED {test} — no vendored open-kernel-modules tree whose \
             GMMU abstraction this harness is written against (set KAYFABE_OGKM_580). The \
             test asserts NOTHING; this line is the only record that it did not run."
        )
    };
}

/// `require_oracle!("name")` — gate the enclosing test on a built oracle, announcing both
/// arms. Returns the `(tag, path)` list.
macro_rules! require_oracle {
    ($name:expr) => {{
        let __o = oracles();
        report($name, !__o.is_empty());
        if __o.is_empty() {
            return;
        }
        __o
    }};
}

// ===========================================================================================
// Driving the oracle
// ===========================================================================================

/// The big page shift the oracle is driven with. 16 (64 KiB) is what a GA10x guest uses;
/// `kgmmuFmtInitLevels_GP10X` asserts the value is 16 or 17, so it is not a free parameter.
const BIG_PAGE_SHIFT: u32 = 16;

/// `gpuIsUnifiedMemorySpaceEnabled` — false on a discrete GPU. Set, `kgmmuFmtInitPte_GP10X`
/// points the SYSMEM address field at the VIDMEM descriptor (the Tegra comptag case), which
/// is not the regime this port emulates.
const UNIFIED_APERTURE: u32 = 0;

/// One `key=value …` line from the oracle.
#[derive(Debug, Clone)]
struct Record(BTreeMap<String, String>);

impl Record {
    fn get(&self, k: &str) -> Option<&str> {
        self.0.get(k).map(String::as_str)
    }
    fn need(&self, k: &str) -> &str {
        self.get(k)
            .unwrap_or_else(|| panic!("the oracle reported no `{k}`; it said:\n{self:#?}"))
    }
    fn num(&self, k: &str) -> u64 {
        let v = self.need(k);
        v.strip_prefix("0x").map_or_else(
            || {
                v.parse()
                    .unwrap_or_else(|_| panic!("`{k}` = `{v}` is not a number"))
            },
            |h| u64::from_str_radix(h, 16).unwrap_or_else(|_| panic!("`{k}` = `{v}` is not hex")),
        )
    }
    fn flag(&self, k: &str) -> bool {
        self.num(k) != 0
    }
}

/// Split one output line into `key=value` pairs. The echoed input is last and is taken
/// whole, because it contains spaces.
fn parse_line(line: &str) -> Record {
    let (head, echo) = line.split_once(" in=").unwrap_or((line, ""));
    let mut m = BTreeMap::new();
    for tok in head.split_whitespace() {
        if let Some((k, v)) = tok.split_once('=') {
            m.insert(k.to_string(), v.to_string());
        }
    }
    if !echo.is_empty() {
        m.insert("in".to_string(), echo.to_string());
    }
    Record(m)
}

/// Run one oracle binary in `mode`, feeding `stdin_lines`, and return one [`Record`] per
/// non-trailer output line plus the trailer (`asserts.total`, `done`).
fn ask(oracle: &str, mode: &str, stdin_lines: &[String]) -> (Vec<Record>, Record) {
    let mut child = Command::new(oracle)
        .arg(mode)
        .arg(BIG_PAGE_SHIFT.to_string())
        .arg(UNIFIED_APERTURE.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot run the GMMU oracle `{oracle}`: {e}"));
    // ★★ The feed runs on its OWN THREAD, and that is not tidiness. The random sweep
    // pushes ~29 000 lines in and gets ~6 MB back; a pipe holds 64 KiB. Writing stdin
    // inline blocks as soon as the child's stdout buffer fills, while the child blocks
    // writing to it — a clean deadlock that presents as "the test suite hangs", with no
    // failing assertion anywhere. It did, for ten minutes, before this thread existed.
    let mut sin = child.stdin.take().expect("stdin");
    let feed: Vec<String> = stdin_lines.to_vec();
    let writer = std::thread::spawn(move || {
        for l in &feed {
            if writeln!(sin, "{l}").is_err() {
                return; // the child died; `wait_with_output` reports it properly
            }
        }
        drop(sin);
    });
    let out = child.wait_with_output().expect("the oracle to finish");
    writer.join().expect("the feed thread");
    assert!(
        out.status.success(),
        "the GMMU oracle `{oracle}` exited {:?} — a SIGNAL here means the driver's own \
         encoder crashed on our input, which is a verdict, not an infrastructure failure.\n\
         stderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut records = Vec::new();
    let mut trailer = BTreeMap::new();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=')
            && matches!(k, "asserts.total" | "asserts.first" | "done")
        {
            trailer.insert(k.to_string(), v.to_string());
            continue;
        }
        if !line.trim().is_empty() {
            records.push(parse_line(line));
        }
    }
    let trailer = Record(trailer);
    assert_eq!(
        trailer.get("done"),
        Some("1"),
        "the GMMU oracle did not run to completion — its output was truncated, so every \
         comparison below would be against a prefix. It said:\n{text}"
    );
    (records, trailer)
}

/// The `levels` mode's output, flattened into one record (it is one `key=value` per line).
fn ask_levels(oracle: &str) -> Record {
    let (records, trailer) = ask(oracle, "levels", &[]);
    assert_eq!(
        trailer.need("asserts.total"),
        "0",
        "the driver's own NV_ASSERTs fired while BUILDING the format ({}), so everything \
         reported after them is not its considered opinion",
        trailer.get("asserts.first").unwrap_or("?"),
    );
    let mut m = BTreeMap::new();
    for r in records {
        for (k, v) in r.0 {
            m.insert(k, v);
        }
    }
    Record(m)
}

/// The levels the driver actually filled in, in its own numbering.
fn present_levels(lv: &Record) -> Vec<u8> {
    (0..16u8)
        .filter(|i| lv.get(&format!("level.{i}.present")) == Some("1"))
        .collect()
}

// ===========================================================================================
// Translating between the two vocabularies
// ===========================================================================================

/// The oracle's aperture name for one of ours.
fn aperture_name(a: Aperture) -> &'static str {
    match a {
        Aperture::Vidmem => "VIDEO",
        Aperture::Peer => "PEER",
        Aperture::SysmemCoherent => "SYS_COH",
        Aperture::SysmemNonCoherent => "SYS_NONCOH",
    }
}

/// Sixteen bytes little-endian, as the oracle prints and reads them.
fn hex16(raw: u128) -> String {
    raw.to_le_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn from_hex16(s: &str) -> u128 {
    let bytes: Vec<u8> = (0..16)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex byte"))
        .collect();
    u128::from_le_bytes(bytes.try_into().expect("16 bytes"))
}

/// Assert that our decode of `raw` at `level` says exactly what the driver's own reading
/// of the same bytes says. **This is the whole oracle**, in one function.
///
/// ★ Written as one comparison rather than per-test assertions so that every corpus below
/// — the encoder round trip, the random sweep, the named fixtures — is judged by the same
/// rules. A per-test relaxation is how a differential quietly stops covering a field.
fn agree(tag: &str, lv: &Record, level: u8, raw: u128, c: &Record, ctx: &str) {
    let fmt = Ga10xGmmu::new();
    let ours = fmt.decode_entry(level, raw);
    let where_ = || format!("[{tag}] level {level} entry {:#034x} ({ctx})", raw);

    if c.get("level_present") == Some("0") {
        assert_eq!(
            ours,
            PteDecode::Invalid,
            "{}: the driver has no level {level}, so our decode must be Invalid — not a \
             plausible entry",
            where_()
        );
        return;
    }
    assert_eq!(
        u64::from(fmt.entry_size(level)),
        c.num("entry_size"),
        "{}: entry width disagrees with the driver's own level table",
        where_()
    );

    if c.flag("is_pte") {
        // ★★ `gmmuFmtEntryIsPte` answers *"read this slot AS a PTE"*, not *"this slot maps
        // something"* — at a level that is only a page table (`bPageTable && !numSubLevels`)
        // it returns `NV_TRUE` unconditionally, because a pure page table has nothing else
        // its slots could be (`gmmu_fmt.c:47-63`). Validity is then the PTE's own `fldValid`.
        //
        // ⊘ Getting this wrong was the FIRST thing this oracle caught, and what it caught
        // was the TEST: `agree` demanded a `Leaf` for every slot at levels 4 and 5, and
        // both the random sweep and the encoder round trip went red on entries our decoder
        // reads exactly right. Suspect the instrument first.
        if !c.flag("pte.valid") {
            let want = if c.flag("pte.volatile") {
                PteDecode::Sparse
            } else {
                PteDecode::Invalid
            };
            assert_eq!(
                ours,
                want,
                "{}: the driver reads this slot as a PTE with VALID clear and volatile={}, \
                 which maps nothing. Sparse and Invalid are different facts.",
                where_(),
                c.flag("pte.volatile"),
            );
            return;
        }
        // A valid PTE. Ours must be a Leaf of the level's own page size.
        let PteDecode::Leaf {
            phys,
            aperture,
            size,
            read_only,
        } = ours
        else {
            panic!(
                "{}: the driver's own gmmuFmtEntryIsPte says this slot is a PTE mapping a \
                 {:#x}-byte page, and we decoded {ours:?}. This is #13's exact shape.",
                where_(),
                c.num("leaf.page_size"),
            );
        };
        assert_eq!(phys, c.num("pte.address"), "{}: leaf address", where_());
        assert_eq!(
            aperture_name(aperture),
            c.need("pte.aperture"),
            "{}: leaf aperture — the PDE and PTE aperture tables are NOT the same table",
            where_()
        );
        assert_eq!(
            size.0,
            c.num("leaf.page_size"),
            "{}: leaf page size (the driver reads it off the level, from virtAddrBitLo)",
            where_()
        );
        assert_eq!(
            read_only,
            c.flag("pte.read_only"),
            "{}: read-only",
            where_()
        );
        // A leaf we express must be a size we enumerate, or the walker's "un-enumerated
        // size is a loud fault" rule fires on a page the driver considers ordinary.
        assert!(
            fmt.page_sizes().contains(&size),
            "{}: the driver maps a {:#x}-byte page here and page_sizes() does not list it",
            where_(),
            size.0
        );
        return;
    }

    // `gmmuFmtEntryIsPte` said PDE.
    let subs = c.num("pde.sub_levels");
    let sub = |i: u64| -> (Option<(u64, String, u8)>, bool) {
        let ap = c.need(&format!("pde.{i}.aperture")).to_string();
        let vol = c.flag(&format!("pde.{i}.volatile"));
        if ap == "INVALID" {
            (None, vol)
        } else {
            let addr = c.num(&format!("pde.{i}.address"));
            let child = u8::try_from(c.num(&format!("pde.{i}.child_level"))).expect("level");
            (Some((addr, ap, child)), vol)
        }
    };

    // Which halves the driver considers present, and how ours must fold them into
    // `edge`/`also`. For a single-sub-level directory there is only one.
    let (edges, empty_vol) = if subs == 1 {
        let (e, vol) = sub(0);
        (e.into_iter().collect::<Vec<_>>(), vol)
    } else {
        // Sub-level 0 is the BIG half and sub-level 1 the SMALL one — the driver's own
        // ordering, taken from `level.N.sub.i` rather than assumed.
        let (big, big_vol) = sub(0);
        let (small, _) = sub(1);
        // ★★ SMALL first: a point query follows `edge` before `also`, and the small-page
        // table is the one RM populates for ordinary mappings.
        let mut v = Vec::new();
        v.extend(small);
        v.extend(big);
        (v, big_vol)
    };

    if edges.is_empty() {
        let want = if empty_vol {
            PteDecode::Sparse
        } else {
            PteDecode::Invalid
        };
        assert_eq!(
            ours,
            want,
            "{}: the driver reads no sub-level here; volatile={empty_vol}, so this is \
             {want:?}. Sparse and Invalid are different facts — one is a declaration the \
             guest made, the other is nothing at all.",
            where_()
        );
        return;
    }

    let PteDecode::Pde { edge, also } = ours else {
        panic!(
            "{}: the driver reads {} sub-level(s) here and we decoded {ours:?}",
            where_(),
            edges.len()
        );
    };
    let got: Vec<(u64, String, u8)> = std::iter::once(edge)
        .chain(also)
        .map(|e| (e.next, aperture_name(e.aperture).to_string(), e.child_level))
        .collect();
    assert_eq!(
        got,
        edges,
        "{}: sub-levels disagree. A decode that returns one half of a dual PDE drops a \
         whole sub-tree with no diagnostic — that is #13 one level up.",
        where_()
    );
    // …and the child levels the driver names must be levels we have geometry for.
    for (_, _, child) in &got {
        assert!(
            fmt.level_shift(*child).is_some(),
            "{}: the driver points at level {child} and we have no geometry for it",
            where_()
        );
        let _ = lv;
    }
}

/// A deterministic 64-bit generator — no dev-dependency, and the same corpus on every box,
/// so a failure is reproducible from the seed printed in the message.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

// ===========================================================================================
// The tests
// ===========================================================================================

#[test]
fn the_oracle_is_the_ampere_encoder_and_not_turings() {
    let os = require_oracle!("the_oracle_is_the_ampere_encoder_and_not_turings");
    for (tag, o) in os {
        let lv = ask_levels(o);
        assert_eq!(
            lv.need("chip"),
            "GA106",
            "[{tag}] the oracle was built for a different chip"
        );
        // ★★★ The whole GA10x generation delta is `kgmmuFmtInitLevels_GA10X`'s single
        // statement `pLevels[2].bPageTable = NV_TRUE`. Judged against `_GP10X` (Turing)
        // the 512 MiB leaf simply would not exist and #13 would be un-catchable here.
        assert_eq!(
            lv.need("hal.init_levels"),
            "kgmmuFmtInitLevels_GA10X",
            "[{tag}] the driver's dispatch table does not give GA106 the Ampere level \
             builder — everything below would be judged against another generation"
        );
        assert_eq!(lv.num("fmt.version"), 2, "[{tag}] GMMU_FMT_VERSION_2");
        assert_eq!(
            format!("{:?}", Ga10xGmmu::new().version()),
            "Ver2",
            "[{tag}] our format claims a different version than the driver builds"
        );
        assert_eq!(
            lv.num("fmt.max_entry_size"),
            16,
            "[{tag}] GMMU_FMT_MAX_ENTRY_SIZE — the width our u128 must carry"
        );
    }
}

#[test]
fn every_level_geometry_matches_the_drivers_own_table() {
    let os = require_oracle!("every_level_geometry_matches_the_drivers_own_table");
    let fmt = Ga10xGmmu::new();
    for (tag, o) in os {
        let lv = ask_levels(o);
        let present = present_levels(&lv);
        assert_eq!(
            present,
            vec![0, 1, 2, 3, 4, 5],
            "[{tag}] the driver filled in a different set of levels than we model"
        );
        for i in &present {
            let g = fmt
                .level_shift(*i)
                .unwrap_or_else(|| panic!("[{tag}] we have no geometry for level {i}"));
            let lo = lv.num(&format!("level.{i}.virt_lo"));
            let hi = lv.num(&format!("level.{i}.virt_hi"));
            assert_eq!(
                u64::from(g.shift),
                lo,
                "[{tag}] level {i}: our stride is not the driver's virtAddrBitLo"
            );
            assert_eq!(
                u64::from(g.entries),
                lv.num(&format!("level.{i}.entry_count")),
                "[{tag}] level {i}: entry COUNT (the driver reads it off {hi}:{lo}). \
                 `entries` is not `page_bytes / entry_size` — PT_BIG holds 32 in a page \
                 that could hold 512, and dividing over-reads it by 3 840 bytes."
            );
            assert_eq!(
                u64::from(fmt.entry_size(*i)),
                lv.num(&format!("level.{i}.entry_size")),
                "[{tag}] level {i}: entry WIDTH"
            );
        }
        // A level the driver does not have must decode to nothing, not to something.
        assert_eq!(fmt.entry_size(6), 0, "[{tag}] level 6 has no width");
        assert!(
            fmt.level_shift(6).is_none(),
            "[{tag}] level 6 has no geometry"
        );
        assert_eq!(
            fmt.decode_entry(6, u128::MAX),
            PteDecode::Invalid,
            "[{tag}] a level the format does not have must not decode to a plausible entry"
        );
    }
}

#[test]
fn our_page_size_list_is_exactly_the_drivers_leaf_levels() {
    let os = require_oracle!("our_page_size_list_is_exactly_the_drivers_leaf_levels");
    let fmt = Ga10xGmmu::new();
    for (tag, o) in os {
        let lv = ask_levels(o);
        let mut driver: Vec<u64> = present_levels(&lv)
            .into_iter()
            .filter(|i| lv.flag(&format!("level.{i}.b_page_table")))
            .map(|i| lv.num(&format!("level.{i}.page_size")))
            .collect();
        driver.sort_unstable();
        driver.dedup();
        let ours: Vec<u64> = fmt.page_sizes().iter().map(|p| p.0).collect();
        // ★ Exhaustive is the contract: an un-enumerated leaf size is a LOUD fault in the
        // walker, so a list short by one turns a decodable page into a refusal — and a
        // list long by one invents a page the hardware cannot map.
        assert_eq!(
            ours, driver,
            "[{tag}] our page-size enumeration is not the set of the driver's own \
             bPageTable levels ({driver:x?})"
        );
    }
}

#[test]
fn the_walk_depth_is_the_drivers_root_to_small_page_chain() {
    let os = require_oracle!("the_walk_depth_is_the_drivers_root_to_small_page_chain");
    let fmt = Ga10xGmmu::new();
    for (tag, o) in os {
        let lv = ask_levels(o);
        // Follow the driver's own sub-level pointers, taking the SMALLEST page at a fork.
        let mut level = 0u8;
        let mut depth = 1usize;
        loop {
            let subs = lv.num(&format!("level.{level}.num_sub_levels"));
            if subs == 0 {
                break;
            }
            let next = (0..subs)
                .map(|s| u8::try_from(lv.num(&format!("level.{level}.sub.{s}"))).expect("level"))
                .min_by_key(|c| lv.num(&format!("level.{c}.page_size")))
                .expect("a sub-level");
            level = next;
            depth += 1;
            assert!(depth < 16, "[{tag}] the level table has a cycle");
        }
        assert_eq!(
            usize::from(fmt.levels()),
            depth,
            "[{tag}] our walk depth is not the driver's root-to-4KiB chain"
        );
    }
}

#[test]
fn pd1_is_a_512mib_leaf_level_and_pd2_is_not() {
    let os = require_oracle!("pd1_is_a_512mib_leaf_level_and_pd2_is_not");
    for (tag, o) in os {
        let lv = ask_levels(o);
        // ★★★ THE HISTORICAL DEFECT, asked of the driver directly. `#13` cost weeks
        // because `PD1` was treated as a pure directory; the whole GA10x delta is that it
        // is not (`kern_gmmu_fmt_ga10x.c:52`).
        assert!(
            lv.flag("level.2.b_page_table"),
            "[{tag}] the driver says PD1 is NOT a page-table level — either this is not \
             GA10x, or the oracle is judging us against the wrong generation"
        );
        assert!(
            !lv.flag("level.1.b_page_table"),
            "[{tag}] PD2 must NOT be a leaf level; if it were, the control below proves \
             nothing"
        );
        assert_eq!(
            lv.num("level.2.page_size"),
            512 << 20,
            "[{tag}] PD1's leaf is 512 MiB"
        );

        // The same bytes at PD1 and at PD2. The valid bit makes one a page and leaves the
        // other a pointer, and no amount of Rust-side testing establishes which.
        let entry = hex16(0x0000_0000_0001_0007);
        let (recs, trailer) = ask(o, "decode", &[format!("2 {entry}"), format!("1 {entry}")]);
        assert_eq!(trailer.need("asserts.total"), "0");
        assert_eq!(recs.len(), 2, "[{tag}] one result line per input line");
        assert!(
            recs[0].flag("is_pte"),
            "[{tag}] with VALID set, PD1's slot is a PAGE"
        );
        assert!(
            !recs[1].flag("is_pte"),
            "[{tag}] the same bytes at PD2 are a POINTER — this is the control"
        );
        agree(tag, &lv, 2, from_hex16(&entry), &recs[0], "PD1 valid=1");
        agree(tag, &lv, 1, from_hex16(&entry), &recs[1], "PD2 control");

        // …and with VALID clear, PD1 is a pointer again.
        let ptr = hex16(0x0000_0000_0001_0002);
        let (recs, _) = ask(o, "decode", &[format!("2 {ptr}")]);
        assert!(
            !recs[0].flag("is_pte"),
            "[{tag}] with VALID clear, PD1's slot is a pointer"
        );
        agree(tag, &lv, 2, from_hex16(&ptr), &recs[0], "PD1 valid=0");
    }
}

#[test]
fn the_dual_pd0_entry_names_both_sub_tables_with_the_drivers_own_shifts() {
    let os =
        require_oracle!("the_dual_pd0_entry_names_both_sub_tables_with_the_drivers_own_shifts");
    for (tag, o) in os {
        let lv = ask_levels(o);
        assert_eq!(
            lv.num("level.3.entry_size"),
            16,
            "[{tag}] PD0's slot is 16 bytes"
        );
        assert_eq!(
            lv.num("level.3.num_sub_levels"),
            2,
            "[{tag}] …and names two"
        );
        let big = u8::try_from(lv.num("level.3.sub.0")).expect("level");
        let small = u8::try_from(lv.num("level.3.sub.1")).expect("level");
        assert_ne!(big, small, "[{tag}] two sub-levels, two rows");
        // The driver's own ordering: sub-level 0 is the BIG-page table.
        assert!(
            lv.num(&format!("level.{big}.page_size")) > lv.num(&format!("level.{small}.page_size")),
            "[{tag}] sub-level 0 must be the bigger page — every fold below depends on it"
        );

        // ★★ The two halves use DIFFERENT SHIFTS (8 vs 12). One shift for both puts every
        // big-page table at one-sixteenth of its real address, silently.
        let specs = [
            "dual VIDEO 7654321000 0 VIDEO 1234567000 0".to_string(),
            "dual SYS_COH 7654321000 0 SYS_NONCOH 1234567000 0".to_string(),
            "dual INVALID 0 0 VIDEO 1234567000 0".to_string(),
            "dual VIDEO 7654321000 0 INVALID 0 0".to_string(),
            "dual INVALID 0 1 INVALID 0 0".to_string(),
        ];
        let (encoded, trailer) = ask(o, "encode", &specs);
        assert_eq!(trailer.need("asserts.total"), "0");
        assert_eq!(encoded.len(), specs.len());
        let lines: Vec<String> = encoded
            .iter()
            .map(|r| format!("3 {}", r.need("out")))
            .collect();
        let (decoded, _) = ask(o, "decode", &lines);
        for (i, (e, d)) in encoded.iter().zip(&decoded).enumerate() {
            agree(tag, &lv, 3, from_hex16(e.need("out")), d, &specs[i]);
        }

        // The first spec names both halves at different addresses: our decode must carry
        // BOTH, and neither may be the other's address.
        let raw = from_hex16(encoded[0].need("out"));
        let PteDecode::Pde { edge, also } = Ga10xGmmu::new().decode_entry(3, raw) else {
            panic!("[{tag}] a dual PDE naming two sub-tables must decode as a Pde");
        };
        let also = also.unwrap_or_else(|| {
            panic!(
                "[{tag}] the driver encoded BOTH halves and we returned one. A decode that \
                 can only express one edge drops a whole sub-tree with no diagnostic."
            )
        });
        assert_ne!(
            edge.next, also.next,
            "[{tag}] the two halves have different addresses AND different shifts"
        );
        assert_eq!(
            edge.child_level, small,
            "[{tag}] `edge` is the small-page table"
        );
        assert_eq!(
            also.child_level, big,
            "[{tag}] `also` is the big-page table"
        );
    }
}

#[test]
fn the_pde_and_pte_aperture_tables_are_not_the_same_table() {
    let os = require_oracle!("the_pde_and_pte_aperture_tables_are_not_the_same_table");
    for (tag, o) in os {
        let lv = ask_levels(o);
        // ⚠ A PDE's `1` is video memory and a PTE's `0` is. A decoder that shared one
        // function between them would put every leaf one aperture out — and would still
        // pass every test written from the same misreading.
        let mut specs = Vec::new();
        for ap in ["VIDEO", "SYS_COH", "SYS_NONCOH"] {
            specs.push(format!("pde {ap} 1234000 0"));
        }
        for ap in ["VIDEO", "PEER", "SYS_COH", "SYS_NONCOH"] {
            specs.push(format!("pte {ap} 1234000 1 0 0 0 0 0 0 3"));
        }
        let (encoded, trailer) = ask(o, "encode", &specs);
        assert_eq!(trailer.need("asserts.total"), "0");

        // Decode each PDE at a pure directory level and each PTE at the 4 KiB table.
        let mut lines = Vec::new();
        for (i, e) in encoded.iter().enumerate() {
            lines.push(format!("{} {}", if i < 3 { 1 } else { 5 }, e.need("out")));
        }
        let (decoded, _) = ask(o, "decode", &lines);
        for (i, (e, d)) in encoded.iter().zip(&decoded).enumerate() {
            let level = if i < 3 { 1 } else { 5 };
            agree(tag, &lv, level, from_hex16(e.need("out")), d, &specs[i]);
        }

        // …and the two encodings of "video memory" really are different bits, so the test
        // above is not vacuous.
        let pde_video = from_hex16(encoded[0].need("out"));
        let pte_video = from_hex16(encoded[3].need("out"));
        assert_ne!(
            (pde_video >> 1) & 0x3,
            (pte_video >> 1) & 0x3,
            "[{tag}] the driver encodes VIDEO the same way in a PDE and a PTE — then the \
             hazard this test exists for does not exist, and the test should be deleted \
             rather than left passing for the wrong reason"
        );
    }
}

#[test]
fn the_drivers_own_sparse_templates_decode_as_sparse() {
    let os = require_oracle!("the_drivers_own_sparse_templates_decode_as_sparse");
    let fmt = Ga10xGmmu::new();
    for (tag, o) in os {
        let lv = ask_levels(o);
        assert!(
            lv.flag("fmt.sparse_hw"),
            "[{tag}] this regime supports sparse in HW; without it the templates below are \
             not the driver's statement about anything"
        );
        // ★★ `kgmmuFmtFamiliesInit_GM200` is the ONLY place the driver says what sparse
        // IS on this regime — *valid clear, volatile set* for a PTE, *aperture INVALID,
        // volatile set* for a PDE. Nothing in the format description says it, so a
        // transcribed decoder has nothing to check its VER2_VOL claim against.
        for (key, levels) in [
            ("sparse.pte", vec![4u8, 5]),
            ("sparse.pde", vec![0u8, 1]),
            ("sparse.pde_multi", vec![3u8]),
        ] {
            let raw = from_hex16(lv.need(key));
            assert_ne!(
                raw, 0,
                "[{tag}] {key} is all-zero — the driver set no bit at all"
            );
            for l in levels {
                assert_eq!(
                    fmt.decode_entry(l, raw),
                    PteDecode::Sparse,
                    "[{tag}] the driver's own {key} must decode as Sparse at level {l}. \
                     Folded into Invalid, the guest's declaration disappears; folded into \
                     Leaf, a valid→sparse transition binds a mapping declared backing-free."
                );
            }
        }
        // A zero entry is INVALID and not sparse: the two must stay distinguishable.
        assert_eq!(
            fmt.decode_entry(5, 0),
            PteDecode::Invalid,
            "[{tag}] zero is not sparse"
        );
        assert_eq!(
            fmt.decode_entry(0, 0),
            PteDecode::Invalid,
            "[{tag}] zero is not sparse"
        );
    }
}

#[test]
fn the_nv4k_template_is_a_named_gap_and_not_a_silent_one() {
    let os = require_oracle!("the_nv4k_template_is_a_named_gap_and_not_a_silent_one");
    let fmt = Ga10xGmmu::new();
    for (tag, o) in os {
        let lv = ask_levels(o);
        // ⊘ A FINDING, recorded as a test rather than as a comment. `kgmmuFmtFamiliesInit_
        // GV100` — which is what the driver binds for GA106 — builds a FOURTH template
        // besides the three sparse ones: NV4K, *valid 0, volatile 1, privilege 1*, meaning
        // "no valid 4 KiB page in this 64 KiB region". We do not model it, so it decodes
        // as Sparse, which is a different statement.
        assert_eq!(
            lv.need("hal.families_init"),
            "kgmmuFmtFamiliesInit_GV100",
            "[{tag}] GA106 gets the Volta template builder; without it there is no NV4K \
             template and this test is measuring nothing"
        );
        let raw = from_hex16(lv.need("nv4k.pte"));
        assert_ne!(
            raw, 0,
            "[{tag}] the NV4K template is empty — the GV100 slice did not run, so the gap \
             below is not the gap this test names"
        );
        assert_eq!(
            fmt.decode_entry(5, raw),
            PteDecode::Sparse,
            "[{tag}] KNOWN GAP: we decode the driver's NV4K template as Sparse. If this \
             starts failing because NV4K grew its own decode, that is the fix landing — \
             update this test rather than widening it."
        );
        // What makes NV4K distinguishable from sparse at all is the privilege bit, and
        // the oracle can show it is there even though we ignore it.
        let (recs, _) = ask(o, "decode", &[format!("5 {}", lv.need("nv4k.pte"))]);
        assert!(
            recs[0].flag("pte.privilege"),
            "[{tag}] NV4K differs from sparse ONLY in the privilege bit; if the driver \
             stopped setting it, the two would be genuinely indistinguishable"
        );
        let (sparse, _) = ask(o, "decode", &[format!("5 {}", lv.need("sparse.pte"))]);
        assert!(
            !sparse[0].flag("pte.privilege"),
            "[{tag}] sparse leaves it clear"
        );
    }
}

#[test]
fn entries_the_driver_encodes_decode_back_to_what_it_was_asked_for() {
    let os = require_oracle!("entries_the_driver_encodes_decode_back_to_what_it_was_asked_for");
    for (tag, o) in os {
        let lv = ask_levels(o);
        // A matrix over the axes an entry HAS: kind, aperture, address, and every boolean
        // the format carries. Encoded by the driver's own setters, read back by the
        // driver's own getters, and judged against our decode of the same bytes.
        let mut specs: Vec<String> = Vec::new();
        let mut levels: Vec<u8> = Vec::new();
        for (leaf_level, addr) in [
            (5u8, "1000"),
            (4u8, "10000"),
            (3u8, "200000"),
            (2u8, "20000000"),
        ] {
            for ap in ["VIDEO", "PEER", "SYS_COH", "SYS_NONCOH"] {
                for ro in [0, 1] {
                    for vol in [0, 1] {
                        specs.push(format!("pte {ap} {addr} 1 {vol} {ro} 0 0 0 5 2"));
                        levels.push(leaf_level);
                    }
                }
            }
            // …and the same slot with VALID clear, which is a different statement at every
            // one of those levels.
            specs.push(format!("pte VIDEO {addr} 0 1 0 0 0 0 0 0"));
            levels.push(leaf_level);
            specs.push(format!("pte VIDEO {addr} 0 0 0 0 0 0 0 0"));
            levels.push(leaf_level);
        }
        for dir_level in [0u8, 1, 2] {
            for ap in ["INVALID", "VIDEO", "SYS_COH", "SYS_NONCOH"] {
                for vol in [0, 1] {
                    specs.push(format!("pde {ap} 3f000 {vol}"));
                    levels.push(dir_level);
                }
            }
        }
        let (encoded, trailer) = ask(o, "encode", &specs);
        assert_eq!(trailer.need("asserts.total"), "0");
        assert_eq!(
            encoded.len(),
            specs.len(),
            "[{tag}] one line out per line in"
        );
        let lines: Vec<String> = encoded
            .iter()
            .zip(&levels)
            .map(|(e, l)| format!("{l} {}", e.need("out")))
            .collect();
        let (decoded, _) = ask(o, "decode", &lines);
        assert_eq!(decoded.len(), specs.len());
        for (i, ((e, d), l)) in encoded.iter().zip(&decoded).zip(&levels).enumerate() {
            assert_eq!(
                e.num("seq"),
                i as u64,
                "[{tag}] the oracle's output desynchronised from its input"
            );
            agree(tag, &lv, *l, from_hex16(e.need("out")), d, &specs[i]);
        }
    }
}

#[test]
fn a_randomised_entry_decodes_the_same_way_in_both() {
    let os = require_oracle!("a_randomised_entry_decodes_the_same_way_in_both");
    // ★★★ THE HEADLINE DIFFERENTIAL. The fixtures above test the entries we thought to
    // write down; this tests the ones we did not. Every bit of a 16-byte slot is random,
    // so reserved fields, impossible combinations and the aperture/valid interactions all
    // get exercised — and the judge is the driver's own reading, never ours.
    const PER_LEVEL: usize = 4096;
    const SEED: u64 = 0x6D4D_5541_5F31_3439; // "mMUA_149"
    for (tag, o) in os {
        let lv = ask_levels(o);
        let mut rng = Rng(SEED);
        let mut raws: Vec<(u8, u128)> = Vec::new();
        for level in 0u8..=6 {
            for _ in 0..PER_LEVEL {
                let lo = u128::from(rng.next());
                let hi = u128::from(rng.next());
                raws.push((level, lo | (hi << 64)));
            }
        }
        let lines: Vec<String> = raws
            .iter()
            .map(|(l, r)| format!("{l} {}", hex16(*r)))
            .collect();
        let (decoded, trailer) = ask(o, "decode", &lines);
        assert_eq!(
            trailer.need("asserts.total"),
            "0",
            "[{tag}] the driver's own NV_ASSERTs fired while READING random entries \
             ({}) — a slot it refuses to interpret is not a slot we can be judged on",
            trailer.get("asserts.first").unwrap_or("?"),
        );
        assert_eq!(decoded.len(), raws.len());
        for (i, ((level, raw), d)) in raws.iter().zip(&decoded).enumerate() {
            assert_eq!(d.num("seq"), i as u64, "[{tag}] output desynchronised");
            agree(
                tag,
                &lv,
                *level,
                *raw,
                d,
                &format!("random seed {SEED:#x} #{i}"),
            );
        }
    }
}

#[test]
fn the_address_fields_have_the_drivers_own_widths() {
    let os = require_oracle!("the_address_fields_have_the_drivers_own_widths");
    for (tag, o) in os {
        let lv = ask_levels(o);
        // The vidmem address field is 25 bits and the sysmem one 46 — they are DIFFERENT
        // fields at the same offset, and an address too wide for one is truncated. Which
        // bits survive is the driver's statement, not ours.
        let huge = "fffffffff000";
        let specs = [
            format!("pte VIDEO {huge} 1 0 0 0 0 0 0 0"),
            format!("pte SYS_COH {huge} 1 0 0 0 0 0 0 0"),
            format!("pde VIDEO {huge} 0"),
            format!("pde SYS_COH {huge} 0"),
        ];
        let (encoded, trailer) = ask(o, "encode", &specs);
        assert_eq!(trailer.need("asserts.total"), "0");
        let lines: Vec<String> = encoded
            .iter()
            .enumerate()
            .map(|(i, e)| format!("{} {}", if i < 2 { 5 } else { 1 }, e.need("out")))
            .collect();
        let (decoded, _) = ask(o, "decode", &lines);
        for (i, (e, d)) in encoded.iter().zip(&decoded).enumerate() {
            let level = if i < 2 { 5 } else { 1 };
            agree(tag, &lv, level, from_hex16(e.need("out")), d, &specs[i]);
        }
        // …and the truncation really happens, so the agreement above is not trivial.
        let vid = decoded[0].num("pte.address");
        let sys = decoded[1].num("pte.address");
        assert_ne!(
            vid, sys,
            "[{tag}] the driver truncated a {huge} address to the same value through the \
             25-bit vidmem field and the 46-bit sysmem field — then this test proves nothing"
        );
        assert!(
            vid < sys,
            "[{tag}] the vidmem field is the narrower of the two"
        );
    }
}

#[test]
fn a_deliberately_wrong_decode_is_caught() {
    let os = require_oracle!("a_deliberately_wrong_decode_is_caught");
    // ★★ The instrument, tested. `agree` is the whole oracle; a version of it that passed
    // everything would make every test above green and empty. So: take an entry the driver
    // reads one way, hand `agree` the OTHER level's verdict, and require a panic.
    for (tag, o) in os {
        let lv = ask_levels(o);
        let entry = hex16(0x0000_0000_0001_0007);
        let (recs, _) = ask(o, "decode", &[format!("2 {entry}"), format!("1 {entry}")]);
        // Sanity: the two really do disagree.
        assert!(
            recs[0].flag("is_pte") && !recs[1].flag("is_pte"),
            "[{tag}] fixture"
        );
        let raw = from_hex16(&entry);
        let swapped = std::panic::catch_unwind(|| {
            agree("bite", &lv, 1, raw, &recs[0], "PD2 judged by PD1's verdict");
        });
        assert!(
            swapped.is_err(),
            "[{tag}] `agree` accepted a PDE where the driver reported a PTE — the \
             comparison is not comparing, and every test in this file is vacuous"
        );
        let swapped = std::panic::catch_unwind(|| {
            agree("bite", &lv, 2, raw, &recs[1], "PD1 judged by PD2's verdict");
        });
        assert!(
            swapped.is_err(),
            "[{tag}] `agree` accepted a PTE where the driver reported a PDE"
        );
    }
}

#[test]
fn a_tree_this_oracle_cannot_judge_says_exactly_why() {
    let _ = require_oracle!("a_tree_this_oracle_cannot_judge_says_exactly_why");
    // ⊘ A vendored tree can be PRESENT and still not serve this oracle: 610.43.02's
    // `gmmuFmtPtePhysAddrFld` takes an `MMU_FMT_LEVEL *` and a `GMMU_PEER_TYPE` that
    // 580.159.04's does not. That is `ogkm is VERSIONED, not the spec`, and it is a fact
    // about the DRIVER, not a missing capability — so it is not spelled as a
    // `GMMU-ORACLE-GATE: SKIPPED` line, which would make the full-suite census red on a
    // healthy box for a reason that is not a defect.
    //
    // ★ But it must not be silent either. `tests/build.rs` diagnoses the divergence
    // structurally (it counts declared parameters) and carries the reason here, so the
    // exclusion is readable at TEST time rather than only in a build log nobody keeps.
    let mut err = std::io::stderr();
    for (tag, reason) in [
        ("580", option_env!("KAYFABE_GMMU_ORACLE_SKIP_580")),
        ("610", option_env!("KAYFABE_GMMU_ORACLE_SKIP_610")),
    ] {
        let Some(reason) = reason else { continue };
        let _ = writeln!(err, "GMMU-ORACLE-NOTE: tree {tag} not judged — {reason}");
        assert!(
            reason.contains("parameter") && reason.contains("gmmuFmt"),
            "the skip reason for the {tag} tree does not name the function and arity that \
             diverged: `{reason}`. An undiagnosed skip is how an oracle quietly stops \
             covering a tree."
        );
    }
}

#[test]
fn the_oracle_reports_which_driver_code_it_compiled() {
    let os = require_oracle!("the_oracle_reports_which_driver_code_it_compiled");
    // ★ A build script that derived the WRONG implementation would produce a perfectly
    // green differential against the wrong generation. The harness echoes the symbol names
    // it was `-D`'d, so the binding is asserted here rather than trusted.
    for (tag, o) in os {
        let lv = ask_levels(o);
        for (key, want) in [
            ("hal.init_levels", "kgmmuFmtInitLevels_GA10X"),
            ("hal.init_pde", "kgmmuFmtInitPde_GP10X"),
            ("hal.init_pde_multi", "kgmmuFmtInitPdeMulti_GP10X"),
            ("hal.init_pte", "kgmmuFmtInitPte_GP10X"),
            ("hal.families_init", "kgmmuFmtFamiliesInit_GV100"),
        ] {
            assert_eq!(
                lv.need(key),
                want,
                "[{tag}] the driver's dispatch table binds {key} elsewhere for GA106. That \
                 is not necessarily wrong — but it means this suite is judging our decoder \
                 against different code than it was written against, and somebody has to \
                 look."
            );
        }
    }
}
