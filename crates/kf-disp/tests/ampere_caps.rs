// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ 2026-11-03 — the Ampere (C673) display caps page, derived per die (`traces/ampere_hw_20261103/README.md`).
//! Before this lane no Ampere die had a tier-3 row, so kf3 served every GA10x the clean authored page and
//! warned at realize that a Windows guest's display may not come up (the Turing storm and GB203 bwhw1 show what
//! that page costs). `[measured, Vast lease 58969637, GA102 RTX 3090 10de:2204, 2026-11-03]` the die's own
//! `0x00c` read with `tools/read_display_caps_page.py` (read-only BAR0, root) under the host driver 580.159.04.
//! These tests pin: GA102 is fully derived (nothing unmeasured), its word is its own, and the other Ampere dies
//! still get the clean page and the notice — never a neighbour's row (OWNER_RULINGS §AH).

use kf_disp::capsderive::{Die, HostFacts, derive, shipped_measured, shipped_rows};

const GA102: Die = Die {
    architecture: 0x170,
    implementation: 0x2,
};
const GA104: Die = Die {
    architecture: 0x170,
    implementation: 0x4,
};
/// The GA102 read (`traces/ampere_hw_20261103/caps/`).
const GA102_00C: u32 = 0x100;
/// `PRECOMP_WIN_PIPE_HDR_CAPA(w)` = `0x780 + 0x20 * w`; `SCLR_PRESENT` is bit 19 in every family's caps class.
const SCLR: u32 = 1 << 19;

fn ampere(die: Die, sdr: bool, branch: &str) -> kf_disp::capsderive::Derived {
    let t = kf_disp::class::for_version("580.65.06").expect("580.65.06 class table");
    let r = kf_disp::regs::Regs::for_ip("580.65.06", kf_chip::display::AMPERE.ip_version)
        .expect("Ampere display registers");
    derive(
        t,
        &r,
        &kf_chip::display::AMPERE,
        die,
        sdr,
        shipped_rows(),
        shipped_measured(),
        branch,
        &HostFacts::default(),
    )
    .expect("Ampere caps page derives")
}

#[test]
fn ga102_is_fully_derived_with_its_own_measured_word() {
    for sdr in [false, true] {
        let d = ampere(GA102, sdr, "580");
        assert!(
            d.unmeasured.is_empty(),
            "GA102 has a measured 0x00c row for branch 580: {:?}",
            d.unmeasured
        );
        assert_eq!(d.page.word(0x00c), GA102_00C, "the GA102 read on lease 58969637");
        for w in 0..8u32 {
            let capa = d.page.word(0x780 + 0x20 * w);
            assert_eq!(
                capa & SCLR != 0,
                w % 2 == 0,
                "window {w}: only main-layer windows claim a scaler"
            );
        }
    }
}

#[test]
fn the_ga102_row_is_scoped_to_its_driver_branch() {
    let d = ampere(GA102, true, "595");
    assert_eq!(d.page.word(0x00c), 0, "no 580-branch read is served to a 595 guest");
    assert_eq!(
        d.unmeasured.iter().map(|(o, _)| *o).collect::<Vec<_>>(),
        vec![0x00c]
    );
}

#[test]
fn an_unmeasured_ampere_die_gets_the_clean_page_and_a_notice_never_a_neighbours_row() {
    let d = ampere(GA104, true, "580");
    assert_eq!(d.page.word(0x00c), 0, "no GA102 value is served to a GA104");
    assert_eq!(
        d.unmeasured.iter().map(|(o, _)| *o).collect::<Vec<_>>(),
        vec![0x00c]
    );
    assert_ne!(
        d.page.word(0x780) & SCLR,
        0,
        "the T1 rule still holds on an unmeasured die"
    );
}
