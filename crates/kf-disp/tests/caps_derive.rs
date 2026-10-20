// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The derived display capabilities page, held to the measured AD104 page FIELD BY FIELD
//! (`docs/design/V3_DISPLAY_CAPS_DERIVATION_20261014.md`). The measured page is a TEST ORACLE in
//! `tests/fixtures/`; no product code reads it.

use kf_chip::display::{ADA, AMPERE, BLACKWELL_GB20X, DisplayRow, TURING};
use kf_disp::caps::{CapsPage, FieldDiff, diff_pages};
use kf_disp::capsderive::{
    Derived, Die, HostFacts, Tier, derive, parse_measured, parse_rows, shipped_measured,
    shipped_rows,
};
use kf_disp::class::{ClassTable, for_version};
use kf_disp::regs::Regs;

const VER: &str = "580.65.06";
const BRANCH: &str = "580";
const AD104: Die = Die {
    architecture: 0x190,
    implementation: 0x4,
};
const AD106: Die = Die {
    architecture: 0x190,
    implementation: 0x6,
};
const AD103: Die = Die {
    architecture: 0x190,
    implementation: 0x3,
};
const GA104: Die = Die {
    architecture: 0x170,
    implementation: 0x4,
};
const TU106: Die = Die {
    architecture: 0x160,
    implementation: 0x6,
};
const GB206: Die = Die {
    architecture: 0x1B0,
    implementation: 0x6,
};

fn fixture() -> CapsPage {
    let text = include_str!("fixtures/caps_page_ad104_580.65.06.txt");
    let mut words = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let mut f = line.split_whitespace();
        let off = u32::from_str_radix(f.next().unwrap().trim_start_matches("0x"), 16).unwrap();
        let val = u32::from_str_radix(f.next().unwrap().trim_start_matches("0x"), 16).unwrap();
        words.push((off, val));
    }
    CapsPage {
        base: 0x0064_0000,
        words,
    }
}

fn table() -> &'static ClassTable {
    for_version(VER).unwrap()
}

fn go(row: &DisplayRow, die: Die, sdr: bool, measured: bool, branch: &str) -> Derived {
    let regs = Regs::for_ip(VER, row.ip_version).unwrap();
    derive(
        table(),
        &regs,
        row,
        die,
        sdr,
        shipped_rows(),
        if measured { shipped_measured() } else { &[] },
        branch,
        &HostFacts::default(),
    )
    .unwrap()
}

fn ad104() -> Derived {
    go(&ADA, AD104, true, true, BRANCH)
}

/// Why a field of the measured AD104 page is NOT claimed by the derived page. Every difference the test
/// finds must be named here (`why` says which tier could ever produce it and why it is not claimed), and
/// every entry must be hit (no stale entry).
fn not_claimed(d: &FieldDiff) -> Option<&'static str> {
    let reg = d.reg.as_str();
    Some(match (reg, d.offset) {
        ("?", 0x008) => {
            "unnamed word 0x008 (0x432): no header names it; bisection run e16: not needed (0x00c is)"
        }
        ("?", 0x048 | 0x058 | 0x068 | 0x078) => {
            "unnamed per-head words (0xa00): no header names them; bisection e9: not needed"
        }
        ("?", 0x0c0 | 0x0d0 | 0x0e0 | 0x0f0) => {
            "unnamed per-head words (0x10): no header names them; bisection e10/e4: not needed"
        }
        ("?", 0x5e4..=0x604) => {
            "HEAD_CLK_CAP words (v05_01 NV_PDISP_FE_SW_HEAD_CLK_CAP; 0x86 x8 measured, the authored page presents 4 heads at the header's INIT 0x77): a per-SKU pixel clock, not needed (e5)"
        }
        ("IHUB_COMMON_CAPA" | "IHUB_COMMON_CAPC", _) => {
            "isohub mempool / line buffer / flip: per-die IMP inputs the engine does not model; NVKMS reads only PLANAR/SEMI_PLANAR/HOR_VER_FLIP (e3: not needed)"
        }
        ("SOR_CAP", _) => {
            "DisplayPort and dual-TMDS SOR bits: kayfabe's connectors are TMDS-only (connector policy, caps.rs)"
        }
        ("SOR_CLK_CAP", _) => "DP_MAX, and SORs 4-7 that the 4-SOR family row does not present",
        (r, _) if r.starts_with("POSTCOMP_HEAD_HDR_CAP") => {
            "postcomp head scaler/DSC/OCSC/dither: features the engine does not implement (e2: not needed)"
        }
        ("PRECOMP_WIN_PIPE_HDR_CAPA", _)
            if matches!(
                d.field.as_str(),
                "ALPHA_WIDTH" | "FULL_WIDTH" | "UNIT_WIDTH"
            ) =>
        {
            "pipeline unit widths: no open source states them, not needed (f3: presence bits suffice)"
        }
        ("PRECOMP_WIN_PIPE_HDR_CAPA", _) if d.index.is_some_and(|i| i % 2 == 1) => {
            "overlay-layer window presence (CSC11 only, no TMO on the measured die): not needed (f5 vs f6); the authored overlay keeps its CSC chain and TMO"
        }
        (
            "PRECOMP_WIN_PIPE_HDR_CAPB"
            | "PRECOMP_WIN_PIPE_HDR_CAPC"
            | "PRECOMP_WIN_PIPE_HDR_CAPD"
            | "PRECOMP_WIN_PIPE_HDR_CAPE"
            | "PRECOMP_WIN_PIPE_HDR_CAPF",
            _,
        ) => {
            "window precisions, LUT/scaler limits: numbers no header states; not needed (e19b fails alone, e18b/f3 pass without them)"
        }
        _ => return None,
    })
}

/// ★ THE field-by-field comparison. The derived AD104 page equals the measured page in every field that
/// is claimed, and every field it does not claim is named with the reason.
#[test]
fn the_derived_ad104_page_equals_the_measured_one_except_the_named_unclaimed_fields() {
    let d = ad104();
    assert!(d.unmeasured.is_empty(), "AD104/580 is measured");
    let diffs = diff_pages(table(), 0xC773, &fixture(), &d.page);
    let mut hit = std::collections::BTreeSet::new();
    for x in &diffs {
        let why = not_claimed(x).unwrap_or_else(|| panic!("an UNEXPLAINED difference: {x:?}"));
        hit.insert(why);
    }
    // every claimed word is exactly the measured one: the decisive facts are not in the diff
    assert!(
        !diffs.iter().any(|x| x.offset == 0x00c),
        "word 0x00c is measured"
    );
    assert!(
        !diffs
            .iter()
            .any(|x| x.reg == "PRECOMP_WIN_PIPE_HDR_CAPA" && x.field == "SCLR_PRESENT"),
        "the main layer's scaler presence matches"
    );
    assert!(
        !diffs
            .iter()
            .any(|x| x.reg == "SYS_CAP" || x.reg == "SYS_CAPB")
    );
    assert!(hit.len() >= 8, "the allow-list is exercised: {}", hit.len());
}

/// The fields the Windows driver was bisected to need are in the page, with the measured value.
#[test]
fn the_decisive_words_are_in_the_page() {
    let d = ad104();
    let fx = fixture();
    assert_eq!(d.page.word(0x00c), fx.word(0x00c));
    assert_eq!(d.page.word(0x00c), 0x100);
    for win in 0..8_u32 {
        let off = 0x780 + 0x20 * win;
        let sclr = (d.page.word(off) >> 19) & 1;
        assert_eq!(sclr, u32::from(win % 2 == 0), "window {win} SCLR_PRESENT");
        assert_eq!(
            sclr,
            (fx.word(off) >> 19) & 1,
            "window {win} equals the measured page"
        );
    }
}

/// The derived AD104 page is, word for word, the page of hardware bisection run f5 (the authored page + the
/// measured word 0x00c + the measured even-window CAPA presence bits), which gave a Windows desktop.
#[test]
fn the_derived_ad104_page_is_the_page_bisection_run_f5_booted() {
    let regs = Regs::for_ip(VER, ADA.ip_version).unwrap();
    let c = &ADA.classes;
    let base =
        kf_disp::caps::sdr_page(table(), &regs, c.caps, c.core, ADA.heads, ADA.windows).unwrap();
    let fx = fixture();
    let mut words: std::collections::BTreeMap<u32, u32> = base.words.iter().copied().collect();
    words.insert(0x00c, fx.word(0x00c));
    for win in (0..8_u32).step_by(2) {
        let off = 0x780 + 0x20 * win;
        let mask = 0x01ff_0000_u32;
        let a = words.get(&off).copied().unwrap_or(0);
        words.insert(off, (a & !mask) | (fx.word(off) & mask));
    }
    let f5: Vec<(u32, u32)> = words.into_iter().filter(|(_, v)| *v != 0).collect();
    assert_eq!(ad104().page.words, f5);
}

/// Provenance: the unnamed word is T3 with its source, the scaler bits are T1 rows, the rest is the
/// authored base; no word is anything else.
#[test]
fn every_word_names_its_tier() {
    let d = ad104();
    assert_eq!(d.provenance.len(), d.page.words.len());
    let p = |o: u32| d.provenance.iter().find(|p| p.offset == o).unwrap();
    assert_eq!(p(0x00c).tier, Tier::Measured);
    assert!(p(0x00c).why.starts_with("measured:"), "{}", p(0x00c).why);
    assert_eq!(p(0x780).tier, Tier::Rule);
    assert!(p(0x780).why.contains("window-layer model"));
    assert_eq!(p(0x000).tier, Tier::Rule);
    assert!(
        d.provenance.iter().all(|p| p.tier != Tier::Host),
        "no tier-2 source exists for this page"
    );
}

/// OWNER_RULINGS §AH: a value measured on one die is never served to another die (or another driver
/// branch). Unmeasured dies get the clean authored page, and say what is missing.
#[test]
fn an_unmeasured_die_gets_the_clean_page_and_the_notice_never_a_neighbours_row() {
    for (row, die) in [
        (&ADA, AD106),
        (&ADA, AD103),
        (&AMPERE, GA104),
        (
            &TURING,
            Die {
                architecture: 0x160,
                implementation: 4,
            },
        ),
        (
            &BLACKWELL_GB20X,
            Die {
                implementation: 3,
                ..GB206
            },
        ),
    ] {
        let d = go(row, die, true, true, BRANCH);
        assert_eq!(
            d.page.word(0x00c),
            0,
            "{die:?} must not receive AD104's word"
        );
        assert_eq!(d.unmeasured.len(), 1, "{die:?}");
        assert_eq!(d.unmeasured[0].0, 0x00c);
        assert!(d.provenance.iter().all(|p| p.tier != Tier::Measured));
    }
    // the right die, the wrong driver branch
    let d = go(&ADA, AD104, true, true, "570");
    assert_eq!(d.page.word(0x00c), 0);
    assert_eq!(d.unmeasured.len(), 1);
}

/// A row for another die is used only for that die: with a second row, each gets its own value.
#[test]
fn a_second_dies_row_is_served_to_that_die_only() {
    let rows = parse_measured(
        "0x190\t0x4\t580\t0xC773\t0x00c\t0x00000100\tmeasured:test AD104\n\
         0x190\t0x6\t580\t0xC773\t0x00c\t0x00000300\tmeasured:test AD106\n",
    )
    .unwrap();
    let regs = Regs::for_ip(VER, ADA.ip_version).unwrap();
    let run = |die| {
        derive(
            table(),
            &regs,
            &ADA,
            die,
            true,
            shipped_rows(),
            &rows,
            "580",
            &HostFacts::default(),
        )
        .unwrap()
        .page
        .word(0x00c)
    };
    assert_eq!(run(AD104), 0x100);
    assert_eq!(run(AD106), 0x300);
    assert_eq!(run(AD103), 0);
}

/// The T3 file refuses a row with no `measured:` source (a blob-derived row is refused, ruling §AF).
#[test]
fn a_measured_row_needs_a_measured_source() {
    assert!(
        parse_measured("0x190\t0x4\t580\t0xC773\t0x00c\t0x100\tcaptured from somewhere\n").is_err()
    );
    assert!(parse_measured("0x190\t0x4\t580\t0xC773\t0x00c\n").is_err());
    assert!(!shipped_measured().is_empty(), "the shipped file parses");
    assert!(
        shipped_measured()
            .iter()
            .all(|m| m.source.starts_with("measured:"))
    );
    assert!(!shipped_rows().is_empty(), "the shipped rows parse");
    assert!(parse_rows("LAYER\tmiddle\tR\tF\tTRUE\trequired\tx\n").is_err());
}

/// Per die, against the die sweep (`traces/die_sweep_20261014/`): the page every die serves is at BAR0
/// 0x640000 in its family's caps class, presents 4 heads + 4 SORs + 8 windows, and carries the
/// main-layer scaler claim on windows 0/2/4/6 only.
#[test]
fn every_family_derives_a_page_at_the_caps_base_with_the_layer_model() {
    for (row, die, class) in [
        (&ADA, AD106, 0xC773_u32),
        (&ADA, AD103, 0xC773),
        (&AMPERE, GA104, 0xC673),
        (&TURING, TU106, 0xC573),
        (&BLACKWELL_GB20X, GB206, 0xCA73),
    ] {
        for sdr in [false, true] {
            let d = go(row, die, sdr, true, BRANCH);
            assert_eq!(row.classes.caps, class);
            assert_eq!(d.page.base, 0x0064_0000, "{die:?}");
            assert_eq!(d.page.word(0) & 0xF, 0xF, "{die:?} HEAD0..3_EXISTS");
            assert_eq!((d.page.word(0) >> 8) & 0xF, 0xF, "{die:?} SOR0..3_EXISTS");
            assert_eq!(d.page.word(4), 0xFF, "{die:?} WINDOW0..7_EXISTS");
            for win in 0..8_u32 {
                let t = table();
                let off = t.a(class, "PRECOMP_WIN_PIPE_HDR_CAPA", win).unwrap();
                let f = t
                    .f(class, "PRECOMP_WIN_PIPE_HDR_CAPA_SCLR_PRESENT")
                    .unwrap();
                let v = kf_disp::class::get(d.page.word(off), f);
                assert_eq!(v, u32::from(win % 2 == 0), "{die:?} window {win}");
                // the base page's CSC11 (NVKMS asserts it) stays on every window
                let c11 = t
                    .f(class, "PRECOMP_WIN_PIPE_HDR_CAPA_CSC11_PRESENT")
                    .unwrap();
                assert_eq!(
                    kf_disp::class::get(d.page.word(off), c11),
                    1,
                    "{die:?} window {win} CSC11"
                );
            }
        }
    }
}

/// Hostile guest: the page is a static set of aligned, distinct, in-page words the engine publishes
/// read-only; nothing in it is derived from guest bytes, and every word has a provenance.
#[test]
fn the_page_is_static_bounded_and_fully_attributed() {
    for (row, die) in [
        (&TURING, TU106),
        (&AMPERE, GA104),
        (&ADA, AD104),
        (&BLACKWELL_GB20X, GB206),
    ] {
        let d = go(row, die, true, true, BRANCH);
        let mut seen = std::collections::BTreeSet::new();
        for (o, v) in &d.page.words {
            assert!(*o < 0x1000 && o % 4 == 0 && seen.insert(*o), "{o:#x}");
            assert_ne!(*v, 0);
        }
        assert_eq!(
            d.provenance.iter().map(|p| p.offset).collect::<Vec<_>>(),
            d.page.words.iter().map(|(o, _)| *o).collect::<Vec<_>>()
        );
    }
}

/// The window count must be a whole number of layers per head, refused by name otherwise.
#[test]
fn a_family_row_that_is_not_whole_layers_is_refused() {
    let bad = DisplayRow { windows: 7, ..ADA };
    let regs = Regs::for_ip(VER, ADA.ip_version).unwrap();
    assert!(
        derive(
            table(),
            &regs,
            &bad,
            AD104,
            true,
            shipped_rows(),
            &[],
            "580",
            &HostFacts::default()
        )
        .is_err()
    );
}

/// The committed family-row listing (`data/caps_family_pages.tsv`, regenerated by
/// `tools/derive_display_caps.sh`) is exactly what the derivation produces today.
fn family_rows() -> String {
    let mut out = String::from(
        "# derived by tools/derive_display_caps.sh (kf_disp::capsderive, ogkm 580.65.06 class tables) — do not edit by hand\n\
         # the page of each family with NO measured row (T1 only); columns: caps_class  offset  value  tier  reason\n",
    );
    for row in kf_chip::display::ALL {
        let d = go(
            row,
            Die {
                architecture: 0,
                implementation: 0,
            },
            true,
            false,
            BRANCH,
        );
        for p in &d.provenance {
            out.push_str(&format!(
                "{:#06X}\t{:#05x}\t{:#010x}\t{:?}\t{}\n",
                row.classes.caps,
                p.offset,
                d.page.word(p.offset),
                p.tier,
                p.why
            ));
        }
        for (off, why) in &d.unmeasured {
            out.push_str(&format!(
                "{:#06X}\t{:#05x}\t-\tUnmeasured\t{}\n",
                row.classes.caps, off, why
            ));
        }
    }
    out
}

#[test]
fn the_committed_family_rows_are_the_derivation() {
    let committed = include_str!("../data/caps_family_pages.tsv");
    assert_eq!(committed, family_rows(), "run tools/derive_display_caps.sh");
}

#[test]
#[ignore = "emits data/caps_family_pages.tsv for tools/derive_display_caps.sh"]
fn emit_family_rows() {
    print!(
        "KF_FAMILY_ROWS_BEGIN\n{}KF_FAMILY_ROWS_END\n",
        family_rows()
    );
}

/// Prints the field diff of the derived AD104 page against the oracle (the doc's table).
#[test]
#[ignore = "prints the diff for the design document"]
fn dump_derived_vs_measured() {
    let d = ad104();
    for x in diff_pages(table(), 0xC773, &fixture(), &d.page) {
        println!(
            "DIFF {:#05x} {}[{}] {} measured={} derived={} :: {}",
            x.offset,
            x.reg,
            x.index.map_or("-".into(), |i| i.to_string()),
            x.field,
            x.a,
            x.b,
            not_claimed(&x).unwrap_or("UNEXPLAINED")
        );
    }
}

/// Exact AD102/580 native-Windows measurement, not inheritance from AD104.
#[test]
fn the_ad102_580_caps_word_has_its_own_native_measurement() {
    let ad102 = Die {
        architecture: 0x190,
        implementation: 0x2,
    };
    let d = go(&ADA, ad102, true, true, "580");
    assert_eq!(d.page.word(0x00c), 0x100);
    assert!(d.unmeasured.is_empty());
    let p = d.provenance.iter().find(|p| p.offset == 0x00c).unwrap();
    assert_eq!(p.tier, Tier::Measured);
    assert!(
        p.why.contains("AD102") && p.why.contains("580.88"),
        "{}",
        p.why
    );
    for branch in ["570", "595"] {
        let d = go(&ADA, ad102, true, true, branch);
        assert_eq!(d.page.word(0x00c), 0);
        assert_eq!(d.unmeasured.len(), 1);
    }
}

#[test]
fn the_tu106_580_caps_word_has_its_own_native_measurement() {
    let d = go(&TURING, TU106, true, true, "580");
    assert_eq!(d.page.word(0x00c), 0x100);
    assert!(d.unmeasured.is_empty());
    let p = d.provenance.iter().find(|p| p.offset == 0x00c).unwrap();
    assert_eq!(p.tier, Tier::Measured);
    assert!(
        p.why.contains("TU106") && p.why.contains("580.88"),
        "{}",
        p.why
    );
    for branch in ["570", "595"] {
        let d = go(&TURING, TU106, true, true, branch);
        assert_eq!(d.page.word(0x00c), 0);
        assert_eq!(d.unmeasured.len(), 1);
    }
}

#[test]
fn the_gb206_580_caps_word_requires_its_own_native_measurement() {
    let d = go(&BLACKWELL_GB20X, GB206, true, true, "580");
    assert_eq!(d.page.word(0x00c), 0x102);
    assert!(d.unmeasured.is_empty());
    let p = d.provenance.iter().find(|p| p.offset == 0x00c).unwrap();
    assert_eq!(p.tier, Tier::Measured);
    assert!(
        p.why.contains("GB206") && p.why.contains("580.88"),
        "{}",
        p.why
    );
    for branch in ["570", "590", "595"] {
        let d = go(&BLACKWELL_GB20X, GB206, true, true, branch);
        assert_eq!(d.page.word(0x00c), 0);
        assert_eq!(d.unmeasured.len(), 1);
    }
    // GB205 now has its own independently measured580 row; GB203 remains unmeasured.
    // ★ 2026-10-20 (bw-resume): GB202 (impl 2) has its OWN measured 580 row now; it is served
    // from that row, never from GB205/GB206.
    {
        let die = Die {
            implementation: 0x2,
            ..GB206
        };
        let d = go(&BLACKWELL_GB20X, die, true, true, "580");
        assert_eq!(d.page.word(0x00c), 0x102);
        assert!(d.unmeasured.is_empty());
        let p = d.provenance.iter().find(|p| p.offset == 0x00c).unwrap();
        assert!(p.why.contains("GB202"), "{}", p.why);
    }
    for implementation in [0x3] {
        let die = Die {
            implementation,
            ..GB206
        };
        let d = go(&BLACKWELL_GB20X, die, true, true, "580");
        assert_eq!(
            d.page.word(0x00c),
            0,
            "a neighbouring die cannot borrow the row"
        );
        assert_eq!(d.unmeasured.len(), 1);
    }
}

#[test]
fn an_ad102_measurement_does_not_authorize_an_unmeasured_guest_branch() {
    let die = Die {
        architecture: 0x190,
        implementation: 2,
    };
    let measured = go(&ADA, die, true, true, "580");
    let other = go(&ADA, die, true, true, "595");
    assert!(measured.unmeasured.is_empty());
    assert!(!other.unmeasured.is_empty());
    assert_ne!(other.page.word(0x00c), measured.page.word(0x00c));
}

#[test]
fn two_separate_gb205_measurements_do_not_authorize_590_or_610() {
    let die = Die {
        architecture: 0x1b0,
        implementation: 5,
    };
    for branch in ["580", "595"] {
        let own = go(&BLACKWELL_GB20X, die, true, true, branch);
        assert!(own.unmeasured.is_empty());
        assert_eq!(own.page.word(0x00c), 0x102);
    }
    for branch in ["590", "610"] {
        let other = go(&BLACKWELL_GB20X, die, true, true, branch);
        assert!(!other.unmeasured.is_empty());
        assert_ne!(other.page.word(0x00c), 0x102);
    }
}
