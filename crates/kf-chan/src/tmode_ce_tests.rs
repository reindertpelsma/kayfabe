// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ 2026-10-19 — the CE-IMPLICIT binding of a guest-KERNEL copy-engine ring. The defect, as
//! measured: Windows run 3017 (`/var/lib/kf-windows-20261005/boundary-kayfabe-3017/qemu.log:40883`,
//! the only `ring REFUSED` of the run), chan `0xc1d00019:0xff040004`, engine `0xb` (COPY2),
//! token `0x806`, born at boot and idle for 12 minutes; during the guest's TDR recovery GP entry
//! 34 carried a copy-engine semaphore release on subchannel 4 and nothing before it had sent a
//! `SET_OBJECT` — `NoCeObject { subch: 4, method: 576 }`, the channel declared DEAD
//! ("kernel channel host 0x1002d REFUSED-AND-POISONED").
//!
//! The same class of defect as the 2026-10-14 NVDEC fix (`crate::vtables`, module doc): the
//! binding of hardware subchannels 0-4 is FIXED by the channel's engine and `SET_OBJECT` is not
//! required by any engine (public `dev_ram.ref`, as recorded in `docs/design/V3_USERD_RELAY.md`
//! §2.5). So the absence of `SET_OBJECT` in a kernel CE ring's pushbuffer is legitimate, and the
//! class the ring decodes against is the one its OWN host channel holds from birth
//! (`HostRing::on_engine_layout` allocates the host CE object) — never a number taken from guest
//! bytes. [`TState::ce_object`] carries that class; `0` (the default: a user-tier or
//! unconfigured ring) keeps the strict behaviour.

use super::{Ir, TState, decode};
use crate::translated::Refusal;
use kf_abi::submit::method_header_inc;

fn is_ce(c: u32) -> bool {
    kf_chip::is_any_dma_copy_class(c)
}

/// The Ada CE class of the measured guest (`RUN31_CE` in `tmode_gr_tests.rs` binds `0xC7B5`).
const C7B5: u32 = 0xC7B5;

/// GP 34 of the kernel CE ring, all 6 words as logged by `ring REFUSED segment`:
/// `SET_SEMAPHORE_A/B/PAYLOAD` (subch 4, `0x240`..), then `LAUNCH_DMA` `0x4c` (subch 4, `0x300`)
/// = FLUSH_ENABLE, SEMAPHORE_TYPE RELEASE_ONE_WORD_SEMAPHORE, INTERRUPT_TYPE NON_BLOCKING,
/// no data transfer.
const RUN3017_GP34: [u32; 6] = [
    0x2003_8090,
    0x0000_0001,
    0x2023_3060,
    0x0000_476c,
    0x2001_80c0,
    0x0000_004c,
];

fn cfg(ce_object: u32) -> TState {
    TState {
        ce_object,
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

fn h(sub: u32, m: u32, n: u32) -> u32 {
    method_header_inc(sub, m, n).expect("header")
}

// ---- the defect, and the guards that must not move -------------------------------------------

/// CURRENT behaviour, kept as the guard for every ring that is NOT a kernel CE ring: with no CE
/// object configured, the first CE method before any `SET_OBJECT` is refused by name.
#[test]
fn an_unconfigured_ring_refuses_the_measured_segment_as_before() {
    assert_eq!(
        decode(&RUN3017_GP34, is_ce, &mut TState::default(), None),
        Err(Refusal::NoCeObject {
            subch: 4,
            method: 0x240
        })
    );
}

/// CORRECT behaviour: a kernel CE ring whose host channel holds a `C7B5` object decodes the
/// measured segment against that class — the release is one `Ir::Launch` on subchannel 4 at the
/// guest's semaphore VA, payload `0x476c`, four bytes — and authors no `SET_OBJECT`.
#[test]
fn the_measured_run3017_segment_is_decoded_against_the_rings_own_ce_class() {
    let mut st = cfg(C7B5);
    let ir = decode(&RUN3017_GP34, is_ce, &mut st, None)
        .expect("a kernel CE ring needs no SET_OBJECT (dev_ram.ref: subchannels 0-4 are fixed)");
    let launches: Vec<_> = ir
        .iter()
        .filter_map(|i| match i {
            Ir::Launch(l) => Some(l),
            _ => None,
        })
        .collect();
    assert_eq!(launches.len(), 1, "{ir:?}");
    let l = launches[0];
    assert_eq!(l.sub, 4);
    assert_eq!(l.tier, crate::ttables::Tier::C7b5);
    let sema = l.sema.expect("a semaphore release");
    assert_eq!(sema.va, 0x1_2023_3060);
    assert_eq!(sema.payload, 0x476c);
    assert_eq!(sema.bytes, 4);
    assert!(l.src.is_none() && l.dst.is_none(), "no data transfer");
    // The binding is derived, not recorded as a guest bind: no word binds a class.
    assert!(
        !ir.iter()
            .any(|i| matches!(i, Ir::Words(w) if w.iter().any(|&(_, m, _)| m == 0))),
        "no SET_OBJECT is authored: {ir:?}"
    );
    assert_eq!(st.tier, None, "the guest has bound nothing");
}

/// Bound through the VM's windows: the emitted semaphore address is the WINDOW address of the
/// guest's row, the payload is the guest's, and the guest VA's low word is never emitted.
#[test]
fn the_measured_segment_binds_through_the_windows_and_emits_no_guest_va() {
    let mut st = cfg(C7B5);
    let ir = decode(&RUN3017_GP34, is_ce, &mut st, None).expect("decoded");
    let w = windows();
    let mut out = Vec::new();
    for i in &ir {
        super::bind(i, &Rows, &w, &mut out).expect("bound");
    }
    let win = 0x10_0000_0000u64 + 0x10_0000 + (0x1_2023_3060 - 0x1_2020_0000);
    let at = out
        .iter()
        .position(|&x| x == h(4, 0x240, 1))
        .expect("SET_SEMAPHORE_A emitted");
    assert_eq!(
        out[at..at + 6],
        [
            h(4, 0x240, 1),
            (win >> 32) as u32,
            h(4, 0x244, 1),
            (win & 0xFFFF_FFFF) as u32,
            h(4, 0x248, 1),
            0x476c
        ]
    );
    assert!(
        !out.contains(&0x2023_3060),
        "the guest VA's low word is never emitted"
    );
    assert!(
        out.contains(&h(4, 0x300, 1)),
        "the launch is authored on subchannel 4"
    );
}

// ---- hostile cases ---------------------------------------------------------------------------

/// A class that is not a copy-engine class gives no leniency: the ring stays strict. A video
/// class, a graphics class and an arbitrary number all fail closed (no default tier is guessed).
#[test]
fn a_ce_object_that_is_not_a_known_copy_class_gives_no_leniency() {
    for class in [0xC9B0, 0xC997, 0xC7C0, 0x5080, 0xDEAD, 0xC7B4, 0xC7B6] {
        assert_eq!(
            decode(&RUN3017_GP34, is_ce, &mut cfg(class), None),
            Err(Refusal::NoCeObject {
                subch: 4,
                method: 0x240
            }),
            "class {class:#x}"
        );
    }
}

/// The decode is against the RING'S class, not a fixed tier: the tier follows the configured CE
/// class (the bit-23 `MEMORY_SCRUB` and the semaphore upper mask differ from `C8B5`).
#[test]
fn the_tier_follows_the_configured_class() {
    for (class, tier) in [
        (0xC5B5, crate::ttables::Tier::C5b5),
        (0xC6B5, crate::ttables::Tier::C6b5),
        (0xC7B5, crate::ttables::Tier::C7b5),
        (0xC8B5, crate::ttables::Tier::C8b5),
        (0xC9B5, crate::ttables::Tier::C9b5),
        (0xCAB5, crate::ttables::Tier::Cab5),
    ] {
        let ir = decode(&RUN3017_GP34, is_ce, &mut cfg(class), None).expect("decoded");
        let Some(Ir::Launch(l)) = ir.last() else {
            panic!("{ir:?}")
        };
        assert_eq!(l.tier, tier, "class {class:#x}");
    }
    // A method only a newer class defines is unclassified on an older one (REQ_ATTR, CAB5).
    assert_eq!(
        decode(&[h(4, 0x754, 1), 0], is_ce, &mut cfg(0xC7B5), None),
        Err(Refusal::Unclassified {
            subch: 4,
            method: 0x754
        })
    );
    assert!(decode(&[h(4, 0x754, 1), 0], is_ce, &mut cfg(0xCAB5), None).is_ok());
}

/// The leniency removes ONE refusal (no object bound) and nothing else: a method outside the CE
/// table is unclassified, a method the table refuses by name is refused by name, and a field the
/// table does not name is refused — exactly as with a guest `SET_OBJECT`.
#[test]
fn methods_outside_the_ce_set_are_still_refused() {
    let run = |w: &[u32]| decode(w, is_ce, &mut cfg(C7B5), None);
    // not a CE method at all
    assert_eq!(
        run(&[h(4, 0x104, 1), 0]),
        Err(Refusal::Unclassified {
            subch: 4,
            method: 0x104
        })
    );
    assert_eq!(
        run(&[h(0, 0x1FC, 1), 0]),
        Err(Refusal::Unclassified {
            subch: 0,
            method: 0x1FC
        }),
        "an unnamed host method"
    );
    // a monitored-fence ADDRESS base: an address no rewriter authors
    assert!(matches!(
        run(&[h(4, 0x220, 1), 0x1234]),
        Err(Refusal::RefusedMethod {
            subch: 4,
            method: 0x220,
            ..
        })
    ));
    // a LAUNCH_DMA word with a conditional-interrupt semaphore type
    assert!(run(&[h(4, 0x300, 1), 0x18]).is_err());
    // a LAUNCH_DMA with the blocking interrupt type
    assert!(run(&[h(4, 0x300, 1), 0x20]).is_err());
    // SubDeviceMask headers are refused before any method
    assert_eq!(
        run(&[(1 << 16) | (1 << 4)]),
        Err(Refusal::SubDeviceMask { at: 0 })
    );
}

/// Only the HARDWARE subchannels 0-4 are fixed to the engine. Software subchannels 5-7 need a
/// binding the host never invents: a CE-looking method there is refused as before.
#[test]
fn software_subchannels_gain_nothing() {
    for sub in 5..8 {
        assert_eq!(
            decode(&[h(sub, 0x240, 1), 1], is_ce, &mut cfg(C7B5), None),
            Err(Refusal::UnboundSubchannel {
                subch: sub,
                method: 0x240
            }),
            "subchannel {sub}"
        );
    }
}

/// A guest `SET_OBJECT` keeps its meaning: it binds (authored, class alone) and from then on the
/// guest's class decides the tier; a non-copy class is still a foreign class.
#[test]
fn a_guest_set_object_still_binds_and_a_foreign_class_is_still_refused() {
    let mut st = cfg(C7B5);
    let ir = decode(&[h(4, 0, 1), 0xC8B5], is_ce, &mut st, None).expect("bind");
    assert_eq!(st.tier, Some(crate::ttables::Tier::C8b5));
    assert!(matches!(&ir[..], [Ir::Words(w)] if w[..] == [(4, 0, 0xC8B5)]));
    assert_eq!(
        decode(&[h(4, 0, 1), 0xC997], is_ce, &mut cfg(C7B5), None),
        Err(Refusal::ForeignClass {
            subch: 4,
            class: 0xC997
        })
    );
}

/// Emission stays bounded by the segment: a maximal run of releases yields exactly one launch per
/// `LAUNCH_DMA` the guest wrote and at most a fixed number of words per launch — the leniency adds
/// no amplification.
#[test]
fn emission_is_bounded_by_the_segment() {
    const N: usize = 1024;
    let mut seg = Vec::new();
    for _ in 0..N {
        seg.extend_from_slice(&RUN3017_GP34);
    }
    let ir = decode(&seg, is_ce, &mut cfg(C7B5), None).expect("decoded");
    let launches = ir.iter().filter(|i| matches!(i, Ir::Launch(_))).count();
    assert_eq!(launches, N);
    let w = windows();
    let mut out = Vec::new();
    for i in &ir {
        super::bind(i, &Rows, &w, &mut out).expect("bound");
    }
    assert!(out.len() <= N * 16, "{} words for {N} launches", out.len());
}

/// The channel's decode width follows the same derived class for HOST methods: a Hopper+ host
/// `SEM_EXECUTE` whose `SEM_ADDR_HI` is wider than 8 bits (high kernel VAs) is admitted on a ring
/// whose host CE class is `C8B5`+ (wide), and refused on `C7B5` and before (the field is 8 bits).
/// With no class at all the field stays the narrow, stricter one.
#[test]
fn the_host_semaphore_width_follows_the_derived_class() {
    let sem_hi: u32 = 0x100; // bit 8: beyond the 8-bit field
    let pb = [h(0, 0x5C, 5), 0x2023_3060, sem_hi, 7, 0, 1];
    for class in [0xC8B5, 0xC9B5, 0xCAB5] {
        let ir = decode(&pb, is_ce, &mut cfg(class), None).expect("wide tier");
        assert!(
            matches!(&ir[..], [Ir::HostSem(_)]),
            "class {class:#x}: {ir:?}"
        );
    }
    for class in [0xC5B5, 0xC6B5, 0xC7B5, 0] {
        assert!(
            matches!(
                decode(&pb, is_ce, &mut cfg(class), None),
                Err(Refusal::FieldValue { .. })
            ),
            "class {class:#x}"
        );
    }
}

/// The guest's own `SET_OBJECT` decides from then on, in BOTH directions: a newer class than the
/// ring's host class admits that class's methods (REQ_ATTR is `CAB5`-only), an older one refuses
/// the methods only the host's newer class defines (`SCRUB_PARAMETERS` is `C8B5`+).
#[test]
fn the_guests_binding_wins_over_the_derived_class_when_newer() {
    let mut st = cfg(C7B5);
    decode(&[h(4, 0, 1), 0xCAB5], is_ce, &mut st, None).expect("bind");
    assert!(decode(&[h(4, 0x754, 1), 0], is_ce, &mut st, None).is_ok());
}

#[test]
fn the_guests_binding_wins_over_the_derived_class_when_older() {
    let mut st = cfg(0xC8B5);
    decode(&[h(4, 0, 1), 0xC7B5], is_ce, &mut st, None).expect("bind");
    assert_eq!(
        decode(&[h(4, 0x6FC, 1), 0], is_ce, &mut st, None),
        Err(Refusal::Unclassified {
            subch: 4,
            method: 0x6FC
        })
    );
    // and the derived class alone does admit it
    assert!(decode(&[h(4, 0x6FC, 1), 0], is_ce, &mut cfg(0xC8B5), None).is_ok());
}

/// The wide host-semaphore field has its own limit (25 bits on `C8B5`+): a bit above it is refused.
#[test]
fn the_wide_host_semaphore_field_is_still_bounded() {
    let pb = |hi: u32| [h(0, 0x5C, 5), 0x2023_3060, hi, 7, 0, 1];
    assert!(decode(&pb(0x1FF_FFFF), is_ce, &mut cfg(0xC8B5), None).is_ok());
    assert!(matches!(
        decode(&pb(0x200_0000), is_ce, &mut cfg(0xC8B5), None),
        Err(Refusal::FieldValue { .. })
    ));
}

/// The count-only census files a derived-class method under the DERIVED class (not under class 0,
/// "no CE bound"), so a census of a kernel CE ring says which table decoded it.
#[test]
fn the_census_names_the_derived_class() {
    use crate::census::{Census, SubKind};
    let mut c = Census::default();
    decode(&RUN3017_GP34, is_ce, &mut cfg(C7B5), Some(&mut c)).expect("decoded");
    assert_eq!(c.count(C7B5, SubKind::Ce, 0x240), 1);
    assert_eq!(c.count(C7B5, SubKind::Ce, 0x300), 1);
    assert_eq!(c.launches_of(C7B5, 0x4c), 1);
    assert_eq!(
        c.count(0, SubKind::Ce, 0x240),
        0,
        "not filed under 'no class'"
    );
}

#[test]
fn the_census_files_a_refused_unconfigured_method_under_no_class() {
    use crate::census::{Census, SubKind};
    let mut c = Census::default();
    assert!(decode(&RUN3017_GP34, is_ce, &mut cfg(0), Some(&mut c)).is_err());
    assert_eq!(c.count(0, SubKind::Ce, 0x240), 1);
}

#[test]
fn the_census_counts_a_run_under_the_derived_class() {
    use crate::census::{Census, SubKind};
    let mut c = Census::default();
    let mut seg = Vec::new();
    for _ in 0..8 {
        seg.extend_from_slice(&RUN3017_GP34);
    }
    decode(&seg, is_ce, &mut cfg(0xC9B5), Some(&mut c)).expect("decoded");
    assert_eq!(c.count(0xC9B5, SubKind::Ce, 0x244), 8);
    assert_eq!(c.launches_of(0xC9B5, 0x4c), 8);
    assert_eq!(c.count(0, SubKind::Ce, 0x244), 0);
}
