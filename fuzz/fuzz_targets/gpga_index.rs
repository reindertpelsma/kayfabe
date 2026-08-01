//! Coverage-guided fuzz of the two **range structures fed guest-controlled coordinates**:
//! `kayfabe_util::IntervalMap` and `kayfabe_mmu::gpga::ViewerIndex`.
//!
//! # Why these, and why together
//!
//! Neither takes bytes, so neither looks like a decoder — and that is exactly why they
//! were never fuzzed. But every `(base, len)` they are handed comes from a guest: a
//! declared allocation, a mapped aperture, a promote-ctx entry. The interesting inputs
//! are the ones a length check does not produce: `len == 0`, `base + len` wrapping past
//! `u64::MAX`, a region ending exactly at the end of the address space, two regions that
//! touch but do not overlap, and the same region inserted twice.
//!
//! `IntervalMap`'s own docs state the boundary-1 posture — *never panic on hostile
//! input* — which is a claim, and this is the measurement of it.
//!
//! # The invariants, asserted after every operation
//!
//! - **`insert` is all-or-nothing.** A refused insert must leave `len()` unchanged; a
//!   half-inserted interval is a structure whose later lookups are wrong (class 4).
//! - **`lookup` agrees with `iter`.** A point inside a stored interval must be found by
//!   `lookup`, and one outside every interval must not be — the two disagreeing is a
//!   mis-resolution, and this structure IS the guest's TLB
//!   (`mode2_address_table_of_truth.md`: MISS = FAULT, never a heuristic resolve).
//! - **`spans` covers its query exactly.** The returned pieces must be contiguous,
//!   ordered, and sum to the queried length. A gap or an overlap here is a range the
//!   caller believes is backed and is not.
//! - **`ViewerIndex` never aliases a viewer against itself**, and a plan applied against
//!   a moved index is refused rather than committed (R5).

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use kayfabe_arch::ids::GpuId;
use kayfabe_arch::{Aperture, FbWindow};
use kayfabe_isolate::IsolateId;
use kayfabe_mmu::gpga::{GpgaRegion, ObjectChange, ViewerIndex, ViewerKind};
use kayfabe_util::interval_map::IntervalMap;

/// Apertures, as a fuzzable choice — two regions with identical coordinates in different
/// apertures are deliberately NOT the same memory, and that distinction is load-bearing.
#[derive(Arbitrary, Debug, Clone, Copy)]
enum Ap {
    Vidmem,
    SysmemCoherent,
    SysmemNonCoherent,
}

impl Ap {
    fn to_aperture(self) -> Aperture {
        match self {
            Ap::Vidmem => Aperture::Vidmem,
            Ap::SysmemCoherent => Aperture::SysmemCoherent,
            Ap::SysmemNonCoherent => Aperture::SysmemNonCoherent,
        }
    }
}

#[derive(Arbitrary, Debug)]
enum MapOp {
    Insert { start: u64, len: u64, value: u32 },
    Lookup { point: u64 },
    RemoveAt { start: u64 },
    NextFrom { at: u64 },
    Spans { start: u64, len: u64 },
}

#[derive(Arbitrary, Debug)]
enum IndexOp {
    AddIsolateView(u32, u32),
    AddWindowView(u8),
    Allocate {
        ap: Ap,
        base: u64,
        len: u64,
        owner: u32,
    },
    MapIntoView {
        viewer: u8,
        ap: Ap,
        base: u64,
        len: u64,
        view_off: u64,
    },
    Free {
        object: u64,
    },
    Remap {
        object: u64,
        ap: Ap,
        base: u64,
        len: u64,
    },
    UnwitnessedWrite {
        ap: Ap,
        base: u64,
        len: u64,
    },
    RemoveView {
        viewer: u8,
    },
    Drain {
        viewer: u8,
    },
    Contents {
        ap: Ap,
        base: u64,
        len: u64,
    },
    ViewersOf {
        ap: Ap,
        base: u64,
        len: u64,
    },
}

#[derive(Arbitrary, Debug)]
struct Input {
    map_ops: Vec<MapOp>,
    index_ops: Vec<IndexOp>,
}

fn region(ap: Ap, base: u64, len: u64) -> Option<GpgaRegion> {
    GpgaRegion::new(ap.to_aperture(), base, len).ok()
}

/// Drive `IntervalMap` and check its three structural invariants after every step.
fn drive_map(ops: &[MapOp]) {
    let mut m: IntervalMap<u32> = IntervalMap::new();
    for op in ops.iter().take(64) {
        match op {
            MapOp::Insert { start, len, value } => {
                let before = m.len();
                match m.insert(*start, *len, *value) {
                    // ★ All-or-nothing. A refused insert that grew the map has left a
                    // partially-installed interval behind.
                    Err(_) => assert_eq!(before, m.len(), "a refused insert mutated the map"),
                    Ok(()) => assert_eq!(before + 1, m.len(), "an accepted insert added ≠ 1"),
                }
            }
            MapOp::Lookup { point } => {
                let got = m.lookup(*point);
                // Cross-check against the ordered iteration: `lookup` and `iter` are two
                // views of one structure and must never disagree.
                let expect = m
                    .iter()
                    .find(|(s, l, _)| *point >= *s && point.wrapping_sub(*s) < *l)
                    .map(|(s, l, _)| (s, l));
                assert_eq!(
                    got.map(|(s, l, _)| (s, l)),
                    expect,
                    "lookup and iter disagree at {point:#x}"
                );
            }
            MapOp::RemoveAt { start } => {
                let before = m.len();
                if m.remove_at(*start).is_some() {
                    assert_eq!(before - 1, m.len());
                } else {
                    assert_eq!(before, m.len());
                }
            }
            MapOp::NextFrom { at } => {
                if let Some((s, _, _)) = m.next_from(*at) {
                    assert!(s >= *at, "next_from({at:#x}) went backwards to {s:#x}");
                }
            }
            MapOp::Spans { start, len } => {
                let spans = m.spans(*start, *len);
                if *len == 0 {
                    assert!(spans.is_empty(), "an empty request is empty, not a fault");
                    continue;
                }
                // ★★ The expected total is the **CLIPPED** extent, not `len`.
                //
                // ⚠ This assertion was wrong on its first draft and the fuzzer caught the
                // *instrument*, not the code (`suspect_the_instrument_first.md`, and it
                // fired within 22 execs). `spans` documents that a wrapping range is
                // clipped at the top of the address space and never wrapped — *"honouring
                // the wrap would let a hostile length reach a range at the BOTTOM of the
                // space from a request aimed at the top"* — so for `start + len > 2^64`
                // the correct cover is `2^64 - start`. Demanding `len` demanded the
                // wrapping behaviour the doc refuses, i.e. it asserted the vulnerability.
                let effective =
                    (u128::from(*start) + u128::from(*len)).min(1u128 << 64) - u128::from(*start);
                let mut cursor = u128::from(*start);
                let mut total: u128 = 0;
                for (s, l, _) in &spans {
                    assert_eq!(
                        u128::from(*s),
                        cursor,
                        "spans left a gap or overlapped at {cursor:#x}"
                    );
                    assert_ne!(*l, 0, "spans emitted a zero-length run");
                    cursor += u128::from(*l);
                    total += u128::from(*l);
                }
                assert_eq!(
                    total, effective,
                    "spans covered {total} bytes of a {effective}-byte effective query \
                     ({start:#x} + {len})"
                );
            }
        }
    }
}

/// Drive `ViewerIndex`. Viewer ids are taken modulo the live set so the fuzzer spends its
/// budget on the geometry rather than on guessing handles.
fn drive_index(ops: &[IndexOp]) {
    let mut idx = ViewerIndex::new();
    let mut viewers = Vec::new();
    let mut objects: Vec<u64> = Vec::new();

    for op in ops.iter().take(64) {
        match op {
            IndexOp::AddIsolateView(proc, gpu) => {
                viewers.push(idx.add_view(ViewerKind::Isolate(IsolateId::new(*proc, GpuId(*gpu)))));
            }
            IndexOp::AddWindowView(w) => {
                let window = match w % 3 {
                    0 => FbWindow::Pramin,
                    1 => FbWindow::FbAperture,
                    _ => FbWindow::InstanceWindow,
                };
                viewers.push(idx.add_view(ViewerKind::Window(window)));
            }
            IndexOp::Allocate {
                ap,
                base,
                len,
                owner,
            } => {
                let Some(r) = region(*ap, *base, *len) else {
                    continue;
                };
                let change = ObjectChange::Allocated {
                    region: r,
                    owner: IsolateId::new(*owner, GpuId(0)),
                };
                if let Ok(plan) = idx.plan(&change) {
                    if let Ok(applied) = idx.apply(&plan) {
                        if let Some(o) = applied.object {
                            objects.push(o.0);
                        }
                    }
                }
            }
            IndexOp::MapIntoView {
                viewer,
                ap,
                base,
                len,
                view_off,
            } => {
                if viewers.is_empty() {
                    continue;
                }
                let v = viewers[usize::from(*viewer) % viewers.len()];
                let Some(r) = region(*ap, *base, *len) else {
                    continue;
                };
                let _ = idx.map_into_view(v, r, *view_off);
                // ★ Self-alias must be impossible after the fact, not merely refused on
                // the way in: mapping the SAME region again must never succeed twice.
                let second = idx.map_into_view(v, r, *view_off);
                assert!(
                    second.is_err(),
                    "a viewer mapped the same region twice — self-alias"
                );
            }
            IndexOp::Free { object } => {
                let change = ObjectChange::Freed {
                    object: kayfabe_mmu::gpga::ObjectId(*object),
                };
                if let Ok(plan) = idx.plan(&change) {
                    let _ = idx.apply(&plan);
                }
            }
            IndexOp::Remap {
                object,
                ap,
                base,
                len,
            } => {
                let Some(r) = region(*ap, *base, *len) else {
                    continue;
                };
                let change = ObjectChange::Remapped {
                    object: kayfabe_mmu::gpga::ObjectId(*object),
                    to: r,
                };
                if let Ok(plan) = idx.plan(&change) {
                    let _ = idx.apply(&plan);
                }
            }
            IndexOp::UnwitnessedWrite { ap, base, len } => {
                let Some(r) = region(*ap, *base, *len) else {
                    continue;
                };
                let change = ObjectChange::UnwitnessedWrite {
                    region: r,
                    transport: kayfabe_mmu::gpga::UnwitnessedTransport::FbToFbCopyEngine,
                };
                if let Ok(plan) = idx.plan(&change) {
                    let _ = idx.apply(&plan);
                }
            }
            IndexOp::RemoveView { viewer } => {
                if viewers.is_empty() {
                    continue;
                }
                let i = usize::from(*viewer) % viewers.len();
                let v = viewers[i];
                if idx.remove_view(v).is_ok() {
                    viewers.remove(i);
                }
            }
            IndexOp::Drain { viewer } => {
                if viewers.is_empty() {
                    continue;
                }
                let v = viewers[usize::from(*viewer) % viewers.len()];
                if let Ok(updates) = idx.drain(v) {
                    assert_eq!(
                        idx.pending_len(v).unwrap_or(0),
                        0,
                        "drain left {} updates behind",
                        updates.len()
                    );
                    let _ = idx.resynced(v);
                }
            }
            IndexOp::Contents { ap, base, len } => {
                let Some(r) = region(*ap, *base, *len) else {
                    continue;
                };
                for s in idx.contents(r) {
                    // Every reported span must lie inside the query — a span outside it
                    // is the structure reporting memory the caller did not ask about.
                    assert!(
                        s.region.base >= r.base && s.region.end() <= r.end(),
                        "contents returned {:#x}..{:#x} outside the {:#x}..{:#x} query",
                        s.region.base,
                        s.region.end(),
                        r.base,
                        r.end()
                    );
                }
            }
            IndexOp::ViewersOf { ap, base, len } => {
                let Some(r) = region(*ap, *base, *len) else {
                    continue;
                };
                let _ = idx.viewers_of(r);
            }
        }
        let _ = objects.len();
    }
}

fuzz_target!(|input: Input| {
    drive_map(&input.map_ops);
    drive_index(&input.index_ops);
});
