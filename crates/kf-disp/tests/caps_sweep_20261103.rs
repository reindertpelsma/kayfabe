// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ 2026-11-03 — the caps sweep (`traces/caps_sweep_20261103/README.md`): the display caps word `0x00c` read with
//! `tools/read_display_caps_page.py` (root, read-only BAR0) on plain Vast container leases, one die per lease, host
//! driver branch recorded. `[measured]` GA104, GA106, TU102, TU104, TU116, AD103, AD106 on 580.x hosts; AD107 on a
//! 595.71.05 host (no 580 AD107 offer existed). These tests pin: every measured die is fully derived (nothing
//! unmeasured) with its own read value, only for the branch it was read on; no family wildcard exists; and the dies
//! the sweep could not reach (GA107, TU117, GB207, AD107 on 580) still get the clean page and the notice, never a
//! neighbour's row (OWNER_RULINGS §AH).

use kf_chip::display::{ADA, AMPERE, DisplayRow, TURING};
use kf_disp::capsderive::{Derived, Die, HostFacts, derive, shipped_measured, shipped_rows};

const fn die(architecture: u32, implementation: u32) -> Die {
    Die {
        architecture,
        implementation,
    }
}

/// `(family row, die, branch, trace folder)` — every row this sweep adds.
const SWEPT: [(&DisplayRow, Die, &str, &str); 8] = [
    (&AMPERE, die(0x170, 0x4), "580", "ga104_3070"),
    (&AMPERE, die(0x170, 0x6), "580", "ga106_3060 and ga106_3050"),
    (&TURING, die(0x160, 0x2), "580", "tu102_2080ti"),
    (&TURING, die(0x160, 0x4), "580", "tu104_2070s"),
    (&TURING, die(0x160, 0x8), "580", "tu116_1660s"),
    (&ADA, die(0x190, 0x3), "580", "ad103_4080"),
    (&ADA, die(0x190, 0x6), "580", "ad106_4060ti"),
    (&ADA, die(0x190, 0x7), "595", "ad107_4060"),
];
/// Every swept die read `DECISIVE 0x00c 0x00000100`.
const SWEPT_00C: u32 = 0x100;
/// `PRECOMP_WIN_PIPE_HDR_CAPA(w)` = `0x780 + 0x20 * w`; `SCLR_PRESENT` is bit 19 in every family's caps class.
const SCLR: u32 = 1 << 19;

fn page(row: &DisplayRow, d: Die, sdr: bool, branch: &str) -> Derived {
    let t = kf_disp::class::for_version("580.65.06").expect("580.65.06 class table");
    let r = kf_disp::regs::Regs::for_ip("580.65.06", row.ip_version).expect("display registers");
    derive(
        t,
        &r,
        row,
        d,
        sdr,
        shipped_rows(),
        shipped_measured(),
        branch,
        &HostFacts::default(),
    )
    .expect("caps page derives")
}

#[test]
fn every_swept_die_is_fully_derived_with_its_own_measured_word() {
    for (row, d, branch, folder) in SWEPT {
        for sdr in [false, true] {
            let p = page(row, d, sdr, branch);
            assert!(
                p.unmeasured.is_empty(),
                "{d:?}/{branch} has a measured 0x00c row ({folder}): {:?}",
                p.unmeasured
            );
            assert_eq!(p.page.word(0x00c), SWEPT_00C, "{d:?}/{branch} ({folder})");
            for w in 0..8u32 {
                let capa = p.page.word(0x780 + 0x20 * w);
                assert_eq!(
                    capa & SCLR != 0,
                    w % 2 == 0,
                    "{d:?} window {w}: only main-layer windows claim a scaler"
                );
            }
        }
    }
}

#[test]
fn each_swept_row_is_scoped_to_the_branch_it_was_read_on_and_names_its_die() {
    for (_, d, branch, folder) in SWEPT {
        let rows: Vec<_> = shipped_measured()
            .iter()
            .filter(|m| m.die == d && m.offset == 0x00c)
            .collect();
        assert_eq!(
            rows.len(),
            1,
            "{d:?}: one row, for the branch it was read on"
        );
        assert_eq!(rows[0].branch, branch, "{d:?}");
        assert_eq!(rows[0].value, SWEPT_00C, "{d:?}");
        assert!(rows[0].source.starts_with("measured:"), "{d:?}");
        let first = folder.split(" and ").next().unwrap();
        assert!(
            rows[0]
                .source
                .contains(&format!("traces/caps_sweep_20261103/{first}")),
            "{d:?}: the source names its committed raw output"
        );
    }
}

#[test]
fn the_sweep_adds_no_family_wildcard() {
    // a family rule needs every die measured in the 2026-11-03 sweep to agree plus a mechanism, and the owner's call (OWNER_RULINGS §AH)
    for m in shipped_measured() {
        assert_ne!(m.branch, "*", "{:?}", m.die);
    }
    // one row per (die, branch) for the swept families: no die appears twice for one branch
    let mut seen = std::collections::BTreeSet::new();
    for m in shipped_measured().iter().filter(|m| m.offset == 0x00c) {
        assert!(
            seen.insert((m.die.architecture, m.die.implementation, m.branch.clone())),
            "duplicate row {:?}",
            m.die
        );
    }
}

#[test]
fn a_swept_branch_is_not_served_to_another_branch() {
    // the AD107 read came from a 595 host: no 580 guest receives it
    let p = page(&ADA, die(0x190, 0x7), true, "580");
    assert_eq!(
        p.page.word(0x00c),
        0,
        "no 595 read is served to a 580 guest"
    );
    assert_eq!(
        p.unmeasured.iter().map(|(o, _)| *o).collect::<Vec<_>>(),
        vec![0x00c]
    );
    // and the 580 reads are not served to a 595 guest
    let p = page(&AMPERE, die(0x170, 0x4), true, "595");
    assert_eq!(
        p.page.word(0x00c),
        0,
        "no 580 read is served to a 595 guest"
    );
    assert_eq!(p.unmeasured.len(), 1);
}

#[test]
fn dies_the_sweep_could_not_reach_still_get_the_clean_page_and_a_notice() {
    for (row, d) in [
        (&AMPERE, die(0x170, 0x7)), // GA107: no offer
        (&TURING, die(0x160, 0x7)), // TU117: no offer
    ] {
        let p = page(row, d, true, "580");
        assert_eq!(
            p.page.word(0x00c),
            0,
            "{d:?}: no neighbour's value is served"
        );
        assert_eq!(
            p.unmeasured.iter().map(|(o, _)| *o).collect::<Vec<_>>(),
            vec![0x00c],
            "{d:?}"
        );
        assert_ne!(
            p.page.word(0x780) & SCLR,
            0,
            "{d:?}: the T1 rule still holds on an unmeasured die"
        );
    }
}
