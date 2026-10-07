// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ Owner rulings 2026-10-07 — the kernel-GR tier and the software-subchannel rule, on the exact
//! words Windows sent in run31 (`traces/windows_code43_walls_20261007/README.md`, run31).

use super::{GrConfig, Ir, SwCall, TState, decode};
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
            deferred_api: false,
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
        case(&[h(1, 0, 1), 0xc9b0]),
        Err(Refusal::ForeignClass {
            subch: 1,
            class: 0xc9b0
        })
    );
    // A notifier address must be 4-byte aligned, and its upper word is 7:0.
    assert!(matches!(
        case(&[h(0, 0, 1), 0xc997, h(0, 0x104, 2), 1, 0x2023_b3a2]),
        Err(Refusal::GrField { method: 0x108, .. })
    ));
    assert!(matches!(
        case(&[h(0, 0, 1), 0xc997, h(0, 0x104, 1), 0x100]),
        Err(Refusal::GrField { method: 0x104, .. })
    ));
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

/// Run32's GR segment (c1d00015:ff040001, GP 0): its last 14 words, as logged (46 in all).
const RUN32_GR_TAIL: [u32; 14] = [
    0x2001_8000,
    0x0000_c7b5,
    0x2001_0000,
    0x0000_c997,
    0x2002_0041,
    0x0000_0001,
    0x2023_b3a0,
    0x2001_2000,
    0x0000_c9c0,
    0x2002_2041,
    0x0000_0001,
    0x2023_b3a0,
    0x2001_a000,
    0x0000_0001,
];

/// One placement row: `[0x1_2020_0000, +1 MiB)` → store offset `0x10_0000`, read-write.
struct OneRow;
impl super::Rows for OneRow {
    fn resolve(&self, va: u64, len: u64) -> Result<Vec<super::Span>, u64> {
        let (lo, n) = (0x1_2020_0000u64, 0x10_0000u64);
        if va < lo || va + len > lo + n {
            return Err(va);
        }
        Ok(vec![super::Span {
            ram: false,
            off: 0x10_0000 + (va - lo),
            len,
            perm: kf_host::MapPerm::READ_WRITE,
        }])
    }
    fn dma_to_file_range(&self, _: u64, _: u64) -> Option<u64> {
        None
    }
}

#[test]
fn run32_gr_segment_binds_its_notifier_addresses_through_the_windows() {
    let mut seg = RUN31_GR.to_vec();
    seg.extend_from_slice(&RUN32_GR_TAIL);
    assert_eq!(seg.len(), 46);
    let mut s = st(true, true);
    let ir = decode(&seg, is_ce, &mut s, None).expect("admitted");
    let addrs: Vec<(u32, u64, u64)> = ir
        .iter()
        .filter_map(|i| match i {
            Ir::GrAddress { sub, va, bytes, .. } => Some((*sub, *va, *bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(addrs, vec![(0, 0x1_2023_b3a0, 16), (1, 0x1_2023_b3a0, 16)]);
    assert_eq!(s.gr_subch, [0xc997, 0xc9c0, 0xa140, 0x902d]);
    assert_eq!(s.inert_binds, 1);
    assert_eq!(s.gr_methods, 21);
    let w = crate::tspace_unsafe::TWindows::new(
        (0x10_0000_0000, 0x1000_0000),
        (0x20_0000_0000, 0x100_0000),
        1 << 40,
    )
    .expect("windows");
    let mut out = Vec::new();
    for i in &ir {
        super::bind(i, &OneRow, &w, &mut out).expect("bound");
    }
    // The notifier address emitted is the window address of the guest's row, never the VA.
    let want = 0x10_0000_0000u64 + 0x10_0000 + (0x1_2023_b3a0 - 0x1_2020_0000);
    let pos = out
        .iter()
        .position(|&x| x == method_header_inc(0, 0x104, 1).expect("header"))
        .expect("SET_NOTIFY_A emitted");
    assert_eq!(out[pos + 1], (want >> 32) as u32);
    assert_eq!(
        out[pos + 2],
        method_header_inc(0, 0x108, 1).expect("header")
    );
    assert_eq!(out[pos + 3], (want & 0xFFFF_FFFF) as u32);
    assert!(!out.contains(&0x2023_b3a0));
    // An address no row covers is refused at bind, by name.
    let bad = Ir::GrAddress {
        sub: 0,
        upper: 0x104,
        va: 0x5_0000_0000,
        bytes: 16,
    };
    assert!(matches!(
        super::bind(&bad, &OneRow, &w, &mut Vec::new()),
        Err(Refusal::VirtualUnresolved { .. })
    ));
}

/// Run44's refused GR segment (c1d00015:ff040001, GP 1, 39 words, as logged), on a channel whose
/// run32 segment bound 3D on 0, compute on 1, I2M on 2, 2D on 3 and software subchannel 5 to 1.
const RUN44_GR_GP1: [u32; 39] = [
    0x2005_0017,
    0x2023_b2e0,
    0x0000_0001,
    0x0000_0002,
    0x0000_0000,
    0x0000_1000,
    0x2001_4000,
    0x0000_a140,
    0x2004_06c0,
    0x0000_0001,
    0x2028_6060,
    0x0000_0001,
    0x1000_f010,
    0x2005_0017,
    0x2028_6060,
    0x0000_0001,
    0x0000_0001,
    0x0000_0000,
    0x0000_0000,
    0x2001_a080,
    0x4000_0002,
    0x2004_06c0,
    0x0000_0001,
    0x2028_6060,
    0x0000_0002,
    0x1000_f010,
    0x2005_0017,
    0x2028_6060,
    0x0000_0001,
    0x0000_0002,
    0x0000_0000,
    0x0000_0000,
    0x2001_a080,
    0x4000_0003,
    0x2004_06c0,
    0x0000_0001,
    0x2023_b300,
    0x0000_0003,
    0x1000_f010,
];

/// ★ Batch 3 (run44): the 3D report-semaphore release is admitted — address validated and emitted
/// through the windows, `D` re-authored — and the segment's NEXT method, a software method on
/// subchannel 5 (bound to the non-class value 1), is refused by name under ruling 4. This pins the
/// prediction for the next run: the GR channel dies there, not at `0x1B00`.
#[test]
fn run44_gr_segment_admits_the_3d_report_semaphore_and_stops_at_the_software_method() {
    let mut s = st(true, true);
    let mut seg = RUN31_GR.to_vec();
    seg.extend_from_slice(&RUN32_GR_TAIL);
    decode(&seg, is_ce, &mut s, None).expect("run32 segment");
    assert_eq!(s.gr_subch, [0xc997, 0xc9c0, 0xa140, 0x902d]);
    let before = s.gr_methods;
    // The first 13 words: host semaphore acquire, I2M rebind, the 3D release — admitted.
    let ir = decode(&RUN44_GR_GP1[..13], is_ce, &mut s.clone(), None).expect("admitted");
    let addrs: Vec<(u32, u32, u64, u64)> = ir
        .iter()
        .filter_map(|i| match i {
            Ir::GrAddress {
                sub,
                upper,
                va,
                bytes,
            } => Some((*sub, *upper, *va, *bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(addrs, vec![(0, 0x1B00, 0x1_2028_6060, 4)]);
    let w = words(&ir);
    assert!(w.contains(&(0, 0x1B08, 1)), "payload C re-authored: {w:x?}");
    assert!(
        w.contains(&(0, 0x1B0C, 0x1000_f010)),
        "trigger D re-authored: {w:x?}"
    );
    // The whole segment: refused at word 19, the software method on subchannel 5.
    let r = decode(&RUN44_GR_GP1, is_ce, &mut s, None);
    assert_eq!(
        r,
        Err(Refusal::InertSubchannelMethod {
            subch: 5,
            method: 0x200,
            value: 1
        })
    );
    assert_eq!(
        s.gr_methods - before,
        4,
        "A, B, C, D admitted once before the refusal"
    );
}

/// ★ OWNER_RULINGS §U (`KF3_DEFERRED_API`): the same run44 segment with the deferred-API path on —
/// subchannel 5's value 1 names the channel's first software object (the 5080, `[measured: the
/// 2026-10-05 VFIO boots 8/9/10]` always its first `ENG_SW` child), so the method is not refused in the decoder: it is a
/// [`Ir::SwMethod`] split carrying the guest's `hApiHandle` (`0x40000002`, the handle VFIO
/// registered first on that object), and the words after it decode on.
#[test]
fn run44_gr_segment_with_the_deferred_api_splits_at_the_software_method() {
    let mut s = st(true, true);
    s.gr.deferred_api = true;
    let mut seg = RUN31_GR.to_vec();
    seg.extend_from_slice(&RUN32_GR_TAIL);
    decode(&seg, is_ce, &mut s, None).expect("run32 segment");
    assert_eq!(
        s.swobj_subch,
        1 << 5,
        "subchannel 5 is a software-object subchannel"
    );
    assert_eq!(
        s.inert_subch, 0,
        "not inert: the deferred path takes precedence"
    );
    let ir = decode(&RUN44_GR_GP1, is_ce, &mut s, None).expect("no refusal in the decoder");
    let calls: Vec<SwCall> = ir
        .iter()
        .filter_map(|i| match i {
            Ir::SwMethod(c) => Some(*c),
            _ => None,
        })
        .collect();
    assert_eq!(
        calls,
        vec![
            SwCall {
                sub: 5,
                value: 1,
                method: 0x200,
                data: 0x4000_0002
            },
            SwCall {
                sub: 5,
                value: 1,
                method: 0x200,
                data: 0x4000_0003
            }
        ]
    );
    // A SET_OBJECT to a real class clears it; a CE class on 5 stays refused as before.
    let mut t = s;
    decode(&[0x2001_a000, 0x0000_c7b5], is_ce, &mut t, None).ok();
    assert_eq!(t.swobj_subch, 0);
}
