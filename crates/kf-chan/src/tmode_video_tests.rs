// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ 2026-10-14 — the VIDEO tier ([`crate::vtables`]) on the exact words the Windows kernel driver
//! sent on its NVDEC channel (run 2990 at `0c5fedff`, `qemu-2990-0c5fedff-full.log:111267`, chan
//! `0xc1d00018:0xff040003`, engine `0x13`, class `0xC9B0`, GP 0 — refused before this tier as
//! `NoCeObject { subch: 4, method: 576 }`, the channel declared DEAD, the guest TDR'd).

use super::{GrConfig, HostSem, HostSemForm, Ir, TState, VideoSem, decode};
use crate::translated::Refusal;
use kf_abi::submit::method_header_inc;

fn is_ce(c: u32) -> bool {
    kf_chip::is_any_dma_copy_class(c)
}

/// The kernel NVDEC channel's first segment, all 32 words as logged.
const EDGE_NVDEC_GP0: [u32; 32] = [
    0x2005_0017,
    0x2023_3040,
    0x0000_0001,
    0x0000_0008,
    0x0000_0000,
    0x0000_1003,
    0x2005_0017,
    0x2023_3260,
    0x0000_0001,
    0x0000_0009,
    0x0000_0000,
    0x0000_0001,
    0x2005_0017,
    0x2023_31d0,
    0x0000_0001,
    0x0000_feed,
    0x0000_0000,
    0x0000_1000,
    0x2003_8090,
    0x0000_0001,
    0x2050_f060,
    0x0000_0001,
    0x2001_80c1,
    0x0000_0000,
    0x2001_80c1,
    0x0003_0000,
    0x2005_a017,
    0x2050_f060,
    0x0000_0001,
    0x0000_0001,
    0x0000_0000,
    0x0000_0000,
];

fn video(class: u32) -> TState {
    TState {
        gr: GrConfig {
            video: class,
            ..GrConfig::default()
        },
        ..TState::default()
    }
}

/// One guest row: VA `[0x1_2020_0000, +4 MiB)` at store offset `0x10_0000 + (va - lo)`.
struct Rows;
impl super::Rows for Rows {
    fn resolve(&self, va: u64, len: u64) -> Result<Vec<super::Span>, u64> {
        let (lo, n) = (0x1_2020_0000u64, 0x40_0000u64);
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

fn windows() -> crate::tspace_unsafe::TWindows {
    crate::tspace_unsafe::TWindows::new(
        (0x10_0000_0000, 0x1000_0000),
        (0x20_0000_0000, 0x100_0000),
        1 << 40,
    )
    .expect("windows")
}

#[test]
fn the_measured_nvdec_segment_is_refused_without_the_video_tier() {
    // The defect as it was: decoded as CE work on a ring that holds no CE object.
    assert_eq!(
        decode(&EDGE_NVDEC_GP0, is_ce, &mut TState::default(), None),
        Err(Refusal::NoCeObject {
            subch: 4,
            method: 0x240
        })
    );
}

#[test]
fn the_measured_nvdec_segment_is_admitted_and_bound_through_the_windows() {
    let mut st = video(0xC9B0);
    let ir = decode(&EDGE_NVDEC_GP0, is_ce, &mut st, None).expect("admitted");
    let host: Vec<(u32, u64)> = ir
        .iter()
        .filter_map(|i| match i {
            Ir::HostSem(HostSem {
                sub,
                va,
                form: HostSemForm::Execute { .. },
            }) => Some((*sub, *va)),
            _ => None,
        })
        .collect();
    assert_eq!(
        host,
        vec![
            (0, 0x1_2023_3040),
            (0, 0x1_2023_3260),
            (0, 0x1_2023_31d0),
            (5, 0x1_2050_f060)
        ]
    );
    let vsem: Vec<&VideoSem> = ir
        .iter()
        .filter_map(|i| match i {
            Ir::VideoSem(s) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(vsem.len(), 2, "a release, then a non-stall trap");
    assert!(
        vsem.iter()
            .all(|s| s.sub == 4 && s.va == 0x1_2050_f060 && s.payload == 1)
    );
    assert_eq!((vsem[0].d.word, vsem[1].d.word), (0, 0x3_0000));
    assert_eq!(st.video_methods, 5);

    // Bound: every address is the window address of the guest's row, never the guest VA.
    let w = windows();
    let mut out = Vec::new();
    for i in &ir {
        super::bind(i, &Rows, &w, &mut out).expect("bound");
    }
    let win = 0x10_0000_0000u64 + 0x10_0000 + (0x1_2050_f060 - 0x1_2020_0000);
    let h = |m| method_header_inc(4, m, 1).expect("header");
    let at = out
        .iter()
        .position(|&x| x == h(0x240))
        .expect("SEMAPHORE_A emitted");
    assert_eq!(
        out[at..at + 8],
        [
            h(0x240),
            (win >> 32) as u32,
            h(0x244),
            (win & 0xFFFF_FFFF) as u32,
            h(0x248),
            1,
            h(0x304),
            0
        ]
    );
    assert!(
        !out.contains(&0x2050_f060),
        "the guest VA's low word is never emitted"
    );
    assert!(
        !out.contains(&method_header_inc(4, 0x300, 1).unwrap()),
        "no EXECUTE"
    );
}

#[test]
fn hostile_video_words_are_refused_by_name() {
    let h = |sub, m, n| method_header_inc(sub, m, n).expect("header");
    let run = |words: &[u32]| decode(words, is_ce, &mut video(0xC9B0), None);
    // codec work and codec operands: refused by name, never forwarded
    assert!(matches!(
        run(&[h(4, 0x300, 1), 0]),
        Err(Refusal::VideoMethod {
            method: 0x300,
            name: "EXECUTE",
            ..
        })
    ));
    assert!(matches!(
        run(&[h(4, 0x404, 1), 0x1234]),
        Err(Refusal::VideoMethod {
            name: "SET_DRV_PIC_SETUP_OFFSET",
            ..
        })
    ));
    assert!(matches!(
        run(&[h(4, 0x25C, 1), 0x1234]),
        Err(Refusal::VideoMethod {
            name: "SET_MONITORED_FENCE_SIGNAL_ADDRESS_BASE_A",
            ..
        })
    ));
    // an offset the class header does not name
    assert_eq!(
        run(&[h(4, 0x104, 1), 0]),
        Err(Refusal::Unclassified {
            subch: 4,
            method: 0x104
        })
    );
    // the semaphore rows' fields
    assert!(matches!(
        run(&[h(4, 0x240, 1), 0x100]),
        Err(Refusal::FieldValue {
            what: "SEMAPHORE_A beyond its UPPER field",
            ..
        })
    ));
    assert!(matches!(
        run(&[h(4, 0x244, 1), 0x2050_f062, h(4, 0x304, 1), 0]),
        Err(Refusal::FieldValue {
            what: "SEMAPHORE_B below bit 2",
            ..
        })
    ));
    assert!(matches!(
        run(&[h(4, 0x304, 1), 1 << 16]),
        Err(Refusal::FieldValue { method: 0x304, .. })
    ));
    // software subchannels stay software; another engine's class never binds a video ring
    assert!(matches!(
        run(&[h(5, 0x240, 1), 1]),
        Err(Refusal::UnboundSubchannel { subch: 5, .. })
    ));
    assert_eq!(
        run(&[h(4, 0, 1), 0xC7B5]),
        Err(Refusal::ForeignClass {
            subch: 4,
            class: 0xC7B5
        })
    );
    // the ring's own class binds (authored), and a class with no public header has no tier
    assert!(run(&[h(4, 0, 1), 0xC9B0]).is_ok());
    assert!(matches!(
        decode(&[h(4, 0x240, 1), 1], is_ce, &mut video(0xCFB0), None),
        Err(Refusal::ForeignClass { class: 0xCFB0, .. })
    ));
}

#[test]
fn a_video_semaphore_outside_the_vms_rows_is_refused_at_bind() {
    let h = |sub, m, n| method_header_inc(sub, m, n).expect("header");
    let mut st = video(0xC9B0);
    let ir = decode(
        &[h(4, 0x240, 2), 0x5, 0x0, h(4, 0x304, 1), 1],
        is_ce,
        &mut st,
        None,
    )
    .expect("decoded");
    let Some(Ir::VideoSem(s)) = ir.last() else {
        panic!("{ir:?}")
    };
    assert_eq!(s.d.bytes, 16, "STRUCTURE_SIZE_FOUR: payload and timestamp");
    assert!(matches!(
        super::bind(&ir[ir.len() - 1], &Rows, &windows(), &mut Vec::new()),
        Err(Refusal::VirtualUnresolved { .. })
    ));
    // the last 8 bytes of the row: a 16-byte operation does not fit; a 4-byte one does
    let edge = 0x1_2020_0000u64 + 0x40_0000 - 8;
    for (d, ok) in [(1u32, false), (0, true)] {
        let mut st = video(0xC9B0);
        let ir = decode(
            &[
                h(4, 0x240, 2),
                (edge >> 32) as u32,
                edge as u32,
                h(4, 0x304, 1),
                d,
            ],
            is_ce,
            &mut st,
            None,
        )
        .expect("decoded");
        let r = super::bind(&ir[ir.len() - 1], &Rows, &windows(), &mut Vec::new());
        assert_eq!(r.is_ok(), ok, "SEMAPHORE_D {d:#x}: {r:?}");
    }
}
