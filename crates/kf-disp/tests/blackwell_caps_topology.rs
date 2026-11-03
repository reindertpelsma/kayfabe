// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ 2026-11-03 (Blackwell Windows display, `traces/blackwell_win_20261103/`): the GB20x capabilities page
//! (`NVCA73`, BAR0 `0x640000`) has three EXISTENCE registers that no earlier caps class has —
//! `SYS_CAPC.TILE_EXISTS(i)`, `SYS_CAPE.VIRWIN_EXISTS(i)` (`clca73.h` 0x34) and `LINK_CAP.PHYCTRL_EXISTS(i)`
//! (`clca73.h` 0x30). The derived page stated only the first (one tile per head); the other two stayed 0,
//! the shape an Ada (`C773`) page has, so the page said "no virtual window and no output link exists".
//! The GB203's own page (root read, `traces/blackwell_hw_20261103/gb203_caps/caps_host595.txt`) has
//! `SYS_CAPE = 0xff` (eight windows) and `LINK_CAP = 0x1f`.
//!
//! What the page must state is the VIRTUAL engine's own topology (a T1 rule, never the die's word):
//! - one virtual window per window the engine presents (`SYS_CAPB.WINDOW_EXISTS`): public NVKMS for GB20x
//!   says "the number of physical windows is same as the number of windows" (ogkm 595.84
//!   `nvkms-evo4.c` EvoGetCapabilitiesCA, `numHwPhywins = numWindows`);
//! - one output link (PHY controller) per SOR the engine presents (`SYS_CAP.SOR_EXISTS`): connector `i` is
//!   TMDS on SOR `i` (`model.rs`), each SOR drives one link.
//!
//! Every offset and field below comes from the generated class table (`data/classes-*.tsv`), never a
//! literal.

use kf_chip::display::{ADA, AMPERE, BLACKWELL_GB20X, DisplayRow, TURING};
use kf_disp::caps::{CapsPage, page, sdr_page};
use kf_disp::capsderive::{Die, HostFacts, derive, shipped_measured, shipped_rows};
use kf_disp::class::{ClassTable, for_version, get};
use kf_disp::regs::Regs;

/// Every guest branch kf-disp has a generated class table for.
const VERSIONS: [&str; 5] = [
    "580.159.04",
    "580.65.06",
    "580.94.02",
    "590.48.01",
    "595.91.07",
];

const GB203: Die = Die {
    architecture: 0x1b0,
    implementation: 0x3,
};

fn table(v: &str) -> &'static ClassTable {
    for_version(v).unwrap_or_else(|| panic!("no class table for {v}"))
}

/// The page's value of the indexed existence field `name(i)` for `i < n`, as a mask.
fn mask(t: &ClassTable, caps: u32, p: &CapsPage, reg: &str, field: &str, n: u32) -> u32 {
    let off = t
        .v(caps, reg)
        .unwrap_or_else(|| panic!("NV{caps:04X}_{reg} is not in the generated class table"));
    (0..n)
        .map(|i| {
            let f = t
                .fa(caps, field, i)
                .unwrap_or_else(|| panic!("NV{caps:04X}_{field}({i}) is not in the class table"));
            get(p.word(off), f) << i
        })
        .fold(0, |m, b| m | b)
}

fn size(t: &ClassTable, caps: u32, field: &str) -> u32 {
    t.v(caps, &format!("{field}__SIZE_1"))
        .unwrap_or_else(|| panic!("NV{caps:04X}_{field}__SIZE_1 is not in the class table"))
}

fn pages(v: &str, row: &DisplayRow) -> [CapsPage; 2] {
    let t = table(v);
    let regs = Regs::for_ip(v, row.ip_version).unwrap();
    let c = row.classes.caps;
    [
        page(t, &regs, c, row.heads, row.windows).unwrap(),
        sdr_page(t, &regs, c, 0, row.heads, row.windows).unwrap(),
    ]
}

#[test]
fn gb20x_page_states_one_virtual_window_per_presented_window() {
    let row = &BLACKWELL_GB20X;
    for v in VERSIONS {
        let t = table(v);
        let c = row.classes.caps;
        for p in pages(v, row) {
            let all = size(t, c, "SYS_CAPE_VIRWIN_EXISTS");
            let windows = mask(t, c, &p, "SYS_CAPB", "SYS_CAPB_WINDOW_EXISTS", all);
            let virwins = mask(t, c, &p, "SYS_CAPE", "SYS_CAPE_VIRWIN_EXISTS", all);
            assert_eq!(windows, (1 << row.windows) - 1, "{v}: SYS_CAPB");
            assert_eq!(
                virwins, windows,
                "{v}: SYS_CAPE.VIRWIN_EXISTS must name exactly the presented windows"
            );
        }
    }
}

#[test]
fn gb20x_page_states_one_output_link_per_presented_sor() {
    let row = &BLACKWELL_GB20X;
    for v in VERSIONS {
        let t = table(v);
        let c = row.classes.caps;
        for p in pages(v, row) {
            let sors = mask(
                t,
                c,
                &p,
                "SYS_CAP",
                "SYS_CAP_SOR_EXISTS",
                size(t, c, "SYS_CAP_SOR_EXISTS"),
            );
            let links = mask(
                t,
                c,
                &p,
                "LINK_CAP",
                "LINK_CAP_PHYCTRL_EXISTS",
                size(t, c, "LINK_CAP_PHYCTRL_EXISTS"),
            );
            assert_eq!(sors, (1 << row.heads) - 1, "{v}: SYS_CAP.SOR_EXISTS");
            assert_eq!(
                links, sors,
                "{v}: LINK_CAP.PHYCTRL_EXISTS must name one link per presented SOR"
            );
        }
    }
}

/// The production derivation (tiers T1 + T3) keeps the two rules for a measured Blackwell die.
#[test]
fn the_derived_gb203_page_keeps_the_topology_words() {
    let row = &BLACKWELL_GB20X;
    for v in VERSIONS {
        let t = table(v);
        let c = row.classes.caps;
        let regs = Regs::for_ip(v, row.ip_version).unwrap();
        let branch = v.split('.').next().unwrap();
        let d = derive(
            t,
            &regs,
            row,
            GB203,
            true,
            shipped_rows(),
            shipped_measured(),
            branch,
            &HostFacts::default(),
        )
        .unwrap();
        assert_eq!(
            mask(
                t,
                c,
                &d.page,
                "SYS_CAPE",
                "SYS_CAPE_VIRWIN_EXISTS",
                row.windows
            ),
            (1 << row.windows) - 1,
            "{v}"
        );
        assert_eq!(
            mask(
                t,
                c,
                &d.page,
                "LINK_CAP",
                "LINK_CAP_PHYCTRL_EXISTS",
                row.heads
            ),
            (1 << row.heads) - 1,
            "{v}"
        );
    }
}

/// Turing..Ada name neither register: their pages gain no word.
#[test]
fn earlier_families_have_neither_register() {
    for v in VERSIONS {
        let t = table(v);
        for row in [&TURING, &AMPERE, &ADA] {
            let c = row.classes.caps;
            assert_eq!(t.v(c, "SYS_CAPE"), None, "{v} NV{c:04X}");
            assert_eq!(t.v(c, "LINK_CAP"), None, "{v} NV{c:04X}");
        }
    }
}
