// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ 2026-11-01 — the Turing Windows reset storm is the display caps page
//! (`traces/windows_gate_20261101/turing_caps/README.md`). `[measured, Vast lease 58721898, TU106 RTX 2060 SUPER,
//! 2026-11-01]` with kf3's authored page Windows never programs a window after the modeset and the RM unloads every
//! ~25 s (gate t1/t2 FAIL); with the authored page plus the two facts the AD104 bisection found (main-layer
//! `SCLR_PRESENT`, the unnamed word `0x00c` read on the die) the same gate PASSES (arm d2), as with the die's own page
//! (arm d1). These tests pin that shape for Turing's C573 page: derived per die, never captured, never a neighbour's row.

use kf_disp::capsderive::{Die, HostFacts, derive, shipped_measured, shipped_rows};

const TU106: Die = Die {
    architecture: 0x160,
    implementation: 0x6,
};
const TU116: Die = Die {
    architecture: 0x160,
    implementation: 0x8,
};
/// `PRECOMP_WIN_PIPE_HDR_CAPA(w)` = `0x780 + 0x20 * w`; `SCLR_PRESENT` is bit 19 in every family's caps class.
const SCLR: u32 = 1 << 19;

fn turing(die: Die, sdr: bool) -> kf_disp::capsderive::Derived {
    let t = kf_disp::class::for_version("580.65.06").expect("580.65.06 class table");
    let r = kf_disp::regs::Regs::for_ip("580.65.06", kf_chip::display::TURING.ip_version)
        .expect("Turing display registers");
    derive(
        t,
        &r,
        &kf_chip::display::TURING,
        die,
        sdr,
        shipped_rows(),
        shipped_measured(),
        "580",
        &HostFacts::default(),
    )
    .expect("Turing caps page derives")
}

#[test]
fn tu106_claims_the_main_layer_scaler_and_its_measured_word() {
    for sdr in [false, true] {
        let d = turing(TU106, sdr);
        assert!(d.unmeasured.is_empty(), "TU106 has a measured 0x00c row");
        assert_eq!(d.page.word(0x00c), 0x100, "the TU106 read (native Windows 580.88 and box 58721898)");
        for w in 0..8u32 {
            let capa = d.page.word(0x780 + 0x20 * w);
            assert_eq!(capa & SCLR != 0, w % 2 == 0, "window {w}: only main-layer windows claim a scaler");
        }
    }
}

#[test]
fn an_unmeasured_turing_die_gets_the_clean_page_and_a_notice_never_a_neighbours_row() {
    let d = turing(TU116, true);
    assert_eq!(d.page.word(0x00c), 0, "no TU106 value is served to a TU116");
    assert_eq!(d.unmeasured.iter().map(|(o, _)| *o).collect::<Vec<_>>(), vec![0x00c]);
    assert_ne!(d.page.word(0x780) & SCLR, 0, "the T1 rule still holds on an unmeasured die");
}
