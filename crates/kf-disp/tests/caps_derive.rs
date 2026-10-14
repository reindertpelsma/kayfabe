// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The derived display capabilities page, held to the measured AD104 page FIELD BY FIELD
//! (`docs/design/V3_DISPLAY_CAPS_DERIVATION_20261014.md`). The measured page is a TEST ORACLE in
//! `tests/fixtures/`; no product code reads it.

use kf_disp::caps::{CapsPage, FieldDiff, diff_pages, sdr_page};
use kf_disp::class::for_version;
use kf_disp::regs::Regs;

const VER: &str = "580.65.06";

fn fixture() -> CapsPage {
    let text = include_str!("fixtures/caps_page_ad104_580.65.06.txt");
    let mut words = Vec::new();
    for line in text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let mut f = line.split_whitespace();
        let off = u32::from_str_radix(f.next().unwrap().trim_start_matches("0x"), 16).unwrap();
        let val = u32::from_str_radix(f.next().unwrap().trim_start_matches("0x"), 16).unwrap();
        words.push((off, val));
    }
    CapsPage { base: 0x0064_0000, words }
}

fn ada() -> (&'static kf_disp::class::ClassTable, Regs, &'static kf_chip::display::DisplayRow) {
    let row = &kf_chip::display::ADA;
    (for_version(VER).unwrap(), Regs::for_ip(VER, row.ip_version).unwrap(), row)
}

/// Prints the field diff of the CURRENT authored page against the oracle (the doc's table).
#[test]
#[ignore = "prints the diff for the design document"]
fn dump_authored_vs_measured() {
    let (t, r, row) = ada();
    let c = &row.classes;
    let authored = sdr_page(t, &r, c.caps, c.core, row.heads, row.windows).unwrap();
    let d: Vec<FieldDiff> = diff_pages(t, c.caps, &fixture(), &authored);
    for x in d {
        println!(
            "DIFF {:#05x} {}[{}] {} measured={} authored={}",
            x.offset,
            x.reg,
            x.index.map_or("-".into(), |i| i.to_string()),
            x.field,
            x.a,
            x.b
        );
    }
}
