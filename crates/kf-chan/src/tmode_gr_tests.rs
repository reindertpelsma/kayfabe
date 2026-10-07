// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ Owner rulings 2026-10-07 — the kernel-GR tier and the software-subchannel rule, on the exact
//! words Windows sent in run31 (`traces/windows_code43_walls_20261007/README.md`, run31).

use super::{GrConfig, Ir, TState, decode};
use crate::translated::Refusal;
use kf_abi::submit::method_header_inc;

fn is_ce(c: u32) -> bool {
    kf_chip::is_any_dma_copy_class(c)
}

/// Run31's GR segment (c1d00015:ff040001, GP 0): its first 32 words, as logged.
const RUN31_GR: [u32; 32] = [
    0x2002_0017,
    0x2023_b000,
    0x0000_0001,
    0x2001_4000,
    0x0000_a140,
    0x2001_6000,
    0x0000_902d,
    0x2001_6222,
    0x0000_0001,
    0x2001_60a4,
    0x0000_0000,
    0x2001_60a7,
    0x0000_0000,
    0x2001_60ab,
    0x0000_0003,
    0x2001_6201,
    0x0000_00cf,
    0x2004_6210,
    0,
    1,
    0,
    1,
    0x2004_6214,
    0,
    0,
    0,
    0,
    0x2004_6230,
    0,
    1,
    0,
    1,
];

/// Run31's CE segment (c1d00013:ff040000, GP 0): all 7 words.
const RUN31_CE: [u32; 7] = [
    0x2002_0017,
    0x2023_b060,
    0x0000_0001,
    0x2001_8000,
    0x0000_c7b5,
    0x2001_a000,
    0x0000_0001,
];

fn st(tier: bool, inert: bool) -> TState {
    TState {
        gr: GrConfig {
            tier,
            inert_sw_subch: inert,
        },
        ..TState::default()
    }
}

fn words(ir: &[Ir]) -> Vec<(u32, u32, u32)> {
    ir.iter()
        .flat_map(|i| match i {
            Ir::Words(v) => v.clone(),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn run31_gr_segment_is_reauthored_with_the_tier_and_refused_without() {
    let mut s = st(false, false);
    assert_eq!(
        decode(&RUN31_GR, is_ce, &mut s, None),
        Err(Refusal::ForeignClass {
            subch: 2,
            class: 0xa140
        })
    );
    let mut s = st(true, false);
    let ir = decode(&RUN31_GR, is_ce, &mut s, None).expect("admitted");
    let w = words(&ir);
    assert_eq!(w[0], (2, 0, 0xa140));
    assert_eq!(w[1], (3, 0, 0x902d));
    // Every 2D method is authored on subchannel 3 with the value Windows sent.
    let want = [
        (0x888, 1),
        (0x290, 0),
        (0x29c, 0),
        (0x2ac, 3),
        (0x804, 0xcf),
        (0x840, 0),
        (0x844, 1),
        (0x848, 0),
        (0x84c, 1),
        (0x850, 0),
        (0x854, 0),
        (0x858, 0),
        (0x85c, 0),
        (0x8c0, 0),
        (0x8c4, 1),
        (0x8c8, 0),
        (0x8cc, 1),
    ];
    let got: Vec<(u32, u32)> = w[2..].iter().map(|&(_, m, v)| (m, v)).collect();
    assert_eq!(got, want);
    assert!(w[2..].iter().all(|&(sub, _, _)| sub == 3));
    assert_eq!(s.gr_methods, 17);
    assert_eq!(s.gr_subch, [0, 0, 0xa140, 0x902d]);
}

#[test]
fn hostile_gr_words_are_refused_by_name() {
    let h = |sub, m, n| method_header_inc(sub, m, n).expect("header");
    let bound = [h(3, 0, 1), 0x902d, h(2, 0, 1), 0xa140];
    let case = |extra: &[u32]| {
        let mut v = bound.to_vec();
        v.extend_from_slice(extra);
        decode(&v, is_ce, &mut st(true, false), None)
    };
    match case(&[h(3, 0x118, 1), 0]) {
        Err(Refusal::GrMethod { name, method, .. }) => {
            assert_eq!(method, 0x118);
            assert!(name.contains("LOAD_MME"));
        }
        r => panic!("{r:?}"),
    }
    assert!(matches!(
        case(&[h(3, 0x224, 1), 0x1000]),
        Err(Refusal::GrMethod { method: 0x224, .. })
    ));
    assert!(matches!(
        case(&[h(3, 0x2ac, 1), 9]),
        Err(Refusal::GrField { method: 0x2ac, .. })
    ));
    assert!(matches!(
        case(&[h(2, 0x180, 1), 4]),
        Err(Refusal::GrMethod { class: 0xa140, .. })
    ));
    assert_eq!(
        case(&[h(4, 0, 1), 0x902d]),
        Err(Refusal::GrSubchannel {
            subch: 4,
            class: 0x902d
        })
    );
    assert_eq!(
        case(&[h(1, 0, 1), 0xc997]),
        Err(Refusal::ForeignClass {
            subch: 1,
            class: 0xc997
        })
    );
    // Rebinding a GR subchannel to a CE class returns it to the CE path.
    let ir = case(&[h(3, 0, 1), 0xc7b5]).expect("CE rebind");
    assert_eq!(words(&ir).last(), Some(&(3, 0, 0xc7b5)));
}

#[test]
fn run31_ce_segment_binds_subchannel_5_inertly_and_refuses_its_methods() {
    assert_eq!(
        decode(&RUN31_CE, is_ce, &mut st(false, false), None),
        Err(Refusal::ForeignClass { subch: 5, class: 1 })
    );
    let mut s = st(false, true);
    let ir = decode(&RUN31_CE, is_ce, &mut s, None).expect("inert bind");
    assert_eq!(words(&ir), vec![(4, 0, 0xc7b5)]);
    assert_eq!(s.inert_binds, 1);
    let later = [method_header_inc(5, 0x100, 1).expect("header"), 0];
    assert_eq!(
        decode(&later, is_ce, &mut s.clone(), None),
        Err(Refusal::InertSubchannelMethod {
            subch: 5,
            method: 0x100,
            value: 1
        })
    );
    // A host method on that subchannel still applies to the channel.
    let host = [method_header_inc(5, 0x78, 1).expect("header"), 0];
    assert!(decode(&host, is_ce, &mut s, None).is_ok());
    // A known engine class on a software subchannel is never inert.
    let known = [method_header_inc(6, 0, 1).expect("header"), 0x902d];
    assert_eq!(
        decode(&known, is_ce, &mut st(false, true), None),
        Err(Refusal::ForeignClass {
            subch: 6,
            class: 0x902d
        })
    );
}
