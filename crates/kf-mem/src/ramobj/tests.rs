//! Tests of the RAM objects: the table, the cut, the plan from a layout, and the cross-check that
//! the plan + table reach exactly the host bytes `translate` names.

use super::*;
use crate::guestmem::{GuestMemory, HostBlock, RamBlockSpec};

const P: u64 = RAM_OBJ_ALIGN;

fn seg(ram_off: u64, len: u64, obj: u32, obj_off: u64) -> RamSegment {
    RamSegment {
        ram_off,
        len,
        obj,
        obj_off,
    }
}

fn blk(gpa: u64, hva: u64, len: u64) -> RamBlockSpec {
    RamBlockSpec { gpa, hva, len }
}

fn layout(b: &[RamBlockSpec]) -> RamLayout {
    RamLayout::new(b).expect("valid layout")
}

/// xorshift64*: deterministic.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

// ------------------------------------------------------------ the table

#[test]
fn validation_refuses_by_name() {
    assert_eq!(
        RamObjects::new(&[seg(0, 0, 1, 0)]),
        Err(RamObjError::ZeroLength { index: 0 })
    );
    assert_eq!(
        RamObjects::new(&[seg(0, P + 1, 1, 0)]),
        Err(RamObjError::Unaligned { index: 0 })
    );
    assert_eq!(
        RamObjects::new(&[seg(0, P, 1, 0), seg(P + 1, P, 2, 0)]),
        Err(RamObjError::Unaligned { index: 1 })
    );
    assert_eq!(
        RamObjects::new(&[seg(0, P, 1, 3 * P + 8)]),
        Err(RamObjError::Unaligned { index: 0 })
    );
    assert_eq!(
        RamObjects::new(&[seg(u64::MAX - P + 1, 2 * P, 1, 0)]),
        Err(RamObjError::Overflow { index: 0 })
    );
    assert_eq!(
        RamObjects::new(&[seg(0, P, 1, u64::MAX - P + 1)]),
        Err(RamObjError::Overflow { index: 0 })
    );
    assert_eq!(
        RamObjects::new(&[seg(0, 2 * P, 1, 0), seg(P, P, 2, 0)]),
        Err(RamObjError::Overlap { index: 1 })
    );
    assert_eq!(
        RamObjects::new(&[seg(2 * P, P, 1, 0), seg(0, P, 2, 0)]),
        Err(RamObjError::Overlap { index: 1 })
    );
    let many: Vec<RamSegment> = (0..=MAX_RAM_SEGMENTS as u64)
        .map(|i| seg(i * 2 * P, P, 1, 0))
        .collect();
    assert_eq!(RamObjects::new(&many), Err(RamObjError::TooMany));
    assert!(RamObjects::new(&[]).expect("empty is a table").is_empty());
}

fn two_objects() -> RamObjects {
    // Object 7 carries RAM [0, 4P) at its offset 0; object 9 carries [4P, 6P) at its offset P.
    RamObjects::new(&[seg(0, 4 * P, 7, 0), seg(4 * P, 2 * P, 9, P)]).expect("table")
}

#[test]
fn resolve_names_the_object_and_the_offset_inside_it() {
    let t = two_objects();
    assert_eq!(t.resolve(0, 8), Ok((7, 0)));
    assert_eq!(t.resolve(3 * P + 5, 3), Ok((7, 3 * P + 5)));
    assert_eq!(t.resolve(4 * P, 8), Ok((9, P)));
    assert_eq!(t.resolve(5 * P + 1, P - 1), Ok((9, 2 * P + 1)));
    // Exactly to the end of the table.
    assert_eq!(t.resolve(4 * P, 2 * P), Ok((9, P)));
}

#[test]
fn resolve_refuses_a_range_across_an_object_edge_and_one_past_the_end() {
    let t = two_objects();
    assert_eq!(
        t.resolve(4 * P - 1, 2),
        Err(RamObjError::Crosses { at: 4 * P })
    );
    assert_eq!(t.resolve(0, 6 * P), Err(RamObjError::Crosses { at: 4 * P }));
    assert_eq!(
        t.resolve(6 * P, 1),
        Err(RamObjError::Uncovered { at: 6 * P })
    );
    assert_eq!(
        t.resolve(5 * P, 2 * P),
        Err(RamObjError::Crosses { at: 6 * P })
    );
    assert_eq!(t.resolve(0, 0), Err(RamObjError::ZeroRange));
    assert_eq!(t.resolve(u64::MAX, 2), Err(RamObjError::RangeOverflow));
    assert_eq!(t.resolve(1, u64::MAX), Err(RamObjError::RangeOverflow));
    assert_eq!(
        RamObjects::default().resolve(0, 1),
        Err(RamObjError::Uncovered { at: 0 })
    );
}

#[test]
fn split_cuts_at_every_object_edge_and_never_across_one() {
    let t = two_objects();
    assert_eq!(
        t.split(4 * P - 8, 16),
        Ok(vec![
            RamSlice {
                ram_off: 4 * P - 8,
                len: 8,
                obj: 7,
                obj_off: 4 * P - 8
            },
            RamSlice {
                ram_off: 4 * P,
                len: 8,
                obj: 9,
                obj_off: P
            },
        ])
    );
    assert_eq!(t.split(0, 6 * P).map(|v| v.len()), Ok(2));
    assert_eq!(
        t.split(P, P),
        Ok(vec![RamSlice {
            ram_off: P,
            len: P,
            obj: 7,
            obj_off: P
        }])
    );
    assert_eq!(t.cuts(0, 6 * P), vec![4 * P]);
    assert_eq!(t.cuts(0, 4 * P), Vec::<u64>::new());
    assert_eq!(t.cuts(4 * P, 2 * P), Vec::<u64>::new());
    assert_eq!(t.cuts(4 * P - 1, 2), vec![4 * P]);
}

#[test]
fn a_gap_between_objects_is_named_not_bridged() {
    // RAM [0, P) and [3P, 4P): nothing carries [P, 3P).
    let t = RamObjects::new(&[seg(0, P, 1, 0), seg(3 * P, P, 2, 0)]).expect("table");
    assert_eq!(t.split(0, 4 * P), Err(RamObjError::Uncovered { at: P }));
    assert_eq!(t.split(P - 1, 2), Err(RamObjError::Uncovered { at: P }));
    assert_eq!(t.split(2 * P, 8), Err(RamObjError::Uncovered { at: 2 * P }));
    assert_eq!(
        t.split(3 * P - 4, 8),
        Err(RamObjError::Uncovered { at: 3 * P - 4 })
    );
    assert!(t.split(3 * P, P).is_ok());
}

#[test]
fn whole_is_the_single_object_shape() {
    let t = RamObjects::whole(5, 1 << 30).expect("table");
    assert_eq!(t.resolve(1 << 20, 4096), Ok((5, 1 << 20)));
    assert_eq!(t.cuts(0, 1 << 30), Vec::<u64>::new());
    assert_eq!(
        t.resolve(0, (1 << 30) + 1),
        Err(RamObjError::Crosses { at: 1 << 30 })
    );
}

/// The model: for every BYTE of the range, which segment carries it (linear scan), grouped by
/// consecutive segment.
fn model_split(segs: &[RamSegment], off: u64, len: u64) -> Result<Vec<RamSlice>, RamObjError> {
    if len == 0 {
        return Err(RamObjError::ZeroRange);
    }
    let end = off.checked_add(len).ok_or(RamObjError::RangeOverflow)?;
    let mut out: Vec<(usize, RamSlice)> = Vec::new();
    for b in off..end {
        let Some(k) = segs
            .iter()
            .position(|s| b >= s.ram_off && b - s.ram_off < s.len)
        else {
            return Err(RamObjError::Uncovered { at: b });
        };
        let s = &segs[k];
        match out.last_mut() {
            Some((lk, l)) if *lk == k => l.len += 1,
            _ => out.push((
                k,
                RamSlice {
                    ram_off: b,
                    len: 1,
                    obj: s.obj,
                    obj_off: s.obj_off + (b - s.ram_off),
                },
            )),
        }
    }
    Ok(out.into_iter().map(|(_, s)| s).collect())
}

#[test]
fn split_agrees_with_the_per_byte_model_on_random_tables() {
    let mut r = Rng(0xA5A5_1234_DEAD_BEEF);
    for _ in 0..3000 {
        let mut segs: Vec<RamSegment> = Vec::new();
        let mut at = r.below(3) * P;
        for _ in 0..(1 + r.below(6)) {
            let len = (1 + r.below(3)) * P;
            // Objects: sometimes the same object continues, sometimes a new one (any offset).
            segs.push(seg(at, len, 100 + r.below(3) as u32, r.below(4) * P));
            at += len + if r.below(3) == 0 { r.below(3) * P } else { 0 };
        }
        let t = RamObjects::new(&segs).expect("table");
        let top = at + 2 * P;
        for _ in 0..40 {
            let edge = if r.below(2) == 0 {
                segs[r.below(segs.len() as u64) as usize].ram_off
            } else {
                r.below(top)
            };
            let off = edge.saturating_sub(r.below(6));
            let len = r.below(48);
            let got = t.split(off, len);
            assert_eq!(
                got,
                model_split(&segs, off, len),
                "{segs:?} {off:#x}+{len:#x}"
            );
            // `cuts` is exactly the segment starts strictly inside the range.
            let want_cuts: Vec<u64> = segs
                .iter()
                .map(|s| s.ram_off)
                .filter(|&s| len != 0 && s > off && s < off + len)
                .collect();
            assert_eq!(t.cuts(off, len), want_cuts);
            // `resolve` is `split` when the answer is one slice.
            match (&got, t.resolve(off, len)) {
                (Ok(v), Ok((o, oo))) => assert_eq!((v.len(), v[0].obj, v[0].obj_off), (1, o, oo)),
                (Ok(v), Err(RamObjError::Crosses { .. })) => assert!(v.len() > 1),
                (Err(RamObjError::Uncovered { at }), Err(RamObjError::Uncovered { at: a2 })) => {
                    assert_eq!(*at, a2)
                }
                // The range starts in a segment, runs off its end into a gap: resolve says Crosses.
                (Err(RamObjError::Uncovered { .. }), Err(RamObjError::Crosses { .. })) => {}
                (Err(RamObjError::ZeroRange), Err(RamObjError::ZeroRange)) => {}
                (g, rr) => panic!("{g:?} vs {rr:?}"),
            }
        }
    }
}

#[test]
fn hostile_ranges_never_panic_or_allocate_unboundedly() {
    let t = two_objects();
    let vals = [
        0u64,
        1,
        P,
        4 * P,
        6 * P,
        1 << 40,
        1 << 63,
        u64::MAX - 1,
        u64::MAX,
    ];
    for &o in &vals {
        for &n in &vals {
            let _ = t.resolve(o, n);
            let _ = t.cuts(o, n);
            match t.split(o, n) {
                Ok(v) => assert!(v.len() <= t.segments().len()),
                Err(_) => {}
            }
        }
    }
}

// --------------------------------------------------------------- the plan

/// q35-like: below the hole, above 4 GiB, separate host mappings.
fn q35() -> RamLayout {
    layout(&[
        blk(0, 0x7f00_0000_0000, 0x8000_0000),
        blk(0x1_0000_0000, 0x7f00_8000_0000, 0x8000_0000),
    ])
}

#[test]
fn one_object_per_block_and_a_hole_costs_no_ram_offset() {
    let p = RamPlan::new(&q35()).expect("plan");
    // The second block continues the first in the VMM, but a block is a VMM mapping: two objects.
    assert_eq!(
        p.segments(),
        &[
            PlanSegment {
                ram_off: 0,
                hva: 0x7f00_0000_0000,
                len: 0x8000_0000
            },
            PlanSegment {
                ram_off: 0x8000_0000,
                hva: 0x7f00_8000_0000,
                len: 0x8000_0000
            },
        ]
    );
    // RAM offsets skip the hole: gpa 4 GiB is RAM offset 2 GiB.
    assert_eq!(p.ram_offset(0x1_0000_0000, 8), Some(0x8000_0000));
    assert_eq!(p.ram_offset(0x7fff_fff8, 8), Some(0x7fff_fff8));
    assert_eq!(
        p.ram_offset(0x7fff_fff8, 16),
        None,
        "a range across the hole is not RAM"
    );
    assert_eq!(p.ram_offset(0x8000_0000, 8), None);
    assert_eq!(p.total_bytes(), 0x1_0000_0000);
}

#[test]
fn adjacent_gpa_blocks_are_adjacent_ram_offsets_and_a_range_across_them_is_cut() {
    let l = layout(&[blk(0, 0x10_0000, 4 * P), blk(4 * P, 0x90_0000, 2 * P)]);
    let p = RamPlan::new(&l).expect("plan");
    assert_eq!(
        p.segments(),
        &[
            PlanSegment {
                ram_off: 0,
                hva: 0x10_0000,
                len: 4 * P
            },
            PlanSegment {
                ram_off: 4 * P,
                hva: 0x90_0000,
                len: 2 * P
            },
        ]
    );
    // A guest range across the two blocks is ONE range in RAM offsets...
    assert_eq!(p.ram_offset(3 * P, 2 * P), Some(3 * P));
    // ...and the table cuts it at the object edge.
    let t = RamObjects::new(&[seg(0, 4 * P, 1, 0), seg(4 * P, 2 * P, 2, 0)]).expect("table");
    assert_eq!(t.cuts(3 * P, 2 * P), vec![4 * P]);
}

#[test]
fn plan_refuses_unaligned_and_empty_layouts_by_name() {
    assert_eq!(RamPlan::new(&RamLayout::default()), Err(PlanError::Empty));
    assert_eq!(
        RamPlan::new(&layout(&[blk(0, 0x10_0000, P), blk(P, 0x20_0001, P)])),
        Err(PlanError::Unaligned { index: 1 })
    );
    assert_eq!(
        RamPlan::new(&layout(&[blk(0, 0x10_0000, P + 8)])),
        Err(PlanError::Unaligned { index: 0 })
    );
    assert_eq!(
        RamPlan::new(&layout(&[blk(8, 0x10_0000, P)])),
        Err(PlanError::Unaligned { index: 0 })
    );
}

/// Objects for a plan: object `1000 + k` is segment `k`, described from its start.
fn objects_of(p: &RamPlan) -> RamObjects {
    let segs: Vec<RamSegment> = p
        .segments()
        .iter()
        .enumerate()
        .map(|(k, s)| seg(s.ram_off, s.len, 1000 + k as u32, 0))
        .collect();
    RamObjects::new(&segs).expect("table from plan")
}

/// The host bytes a plan + table reach for guest-physical `[gpa, gpa+len)`, as `translate` would
/// name them: `None` when the plan does not call it RAM.
fn via_objects(p: &RamPlan, t: &RamObjects, gpa: u64, len: u64) -> Option<Vec<HostBlock>> {
    let off = p.ram_offset(gpa, len)?;
    let mut out: Vec<HostBlock> = Vec::new();
    for s in t.split(off, len).ok()? {
        let k = (s.obj - 1000) as usize;
        let hva = p.segments()[k].hva + s.obj_off;
        match out.last_mut() {
            Some(l) if l.hva + l.len == hva => l.len += s.len,
            _ => out.push(HostBlock { hva, len: s.len }),
        }
    }
    Some(out)
}

#[test]
fn plan_plus_table_reach_exactly_the_host_bytes_translate_names() {
    let mut r = Rng(0x0123_4567_89AB_CDEF);
    let (mut agree_ok, mut agree_none) = (0u64, 0u64);
    for _ in 0..3000 {
        // Page-grain random layouts: holes, adjacent-contiguous, adjacent-discontiguous, aliases.
        let n = 1 + r.below(8);
        let mut gpa = r.below(3) * P;
        let mut hva = 0x100_0000 + r.below(16) * P;
        let mut blocks: Vec<RamBlockSpec> = Vec::new();
        for _ in 0..n {
            let len = (1 + r.below(4)) * P;
            blocks.push(blk(gpa, hva, len));
            gpa += len;
            if r.below(3) == 0 {
                gpa += (1 + r.below(3)) * P;
            }
            match r.below(4) {
                0 => hva += len,
                1 => hva += len + (1 + r.below(5)) * P,
                2 => hva = hva.saturating_sub(len + P),
                _ => hva = 0x200_0000 + r.below(64) * P,
            }
        }
        let l = layout(&blocks);
        let p = RamPlan::new(&l).expect("plan");
        let t = objects_of(&p);
        let top = gpa + 2 * P;
        for _ in 0..60 {
            let edge = if r.below(2) == 0 {
                blocks[r.below(blocks.len() as u64) as usize].gpa
            } else {
                r.below(top)
            };
            let g = edge.saturating_sub(r.below(9));
            let len = 1 + r.below(3 * P);
            let want = l.translate(g, len).ok();
            let got = via_objects(&p, &t, g, len);
            // Where `translate` serves the range, the plan serves it and reaches the same host
            // bytes; where it refuses, the plan refuses too (never bridges a hole).
            assert_eq!(got, want, "{blocks:?} {g:#x}+{len:#x}");
            if want.is_some() {
                agree_ok += 1;
            } else {
                agree_none += 1;
            }
        }
    }
    assert!(
        agree_ok > 10_000 && agree_none > 10_000,
        "{agree_ok} {agree_none}"
    );
}

#[test]
fn a_changed_layout_is_detectable() {
    let a = RamPlan::new(&q35()).expect("plan");
    let b = RamPlan::new(&layout(&[blk(0, 0x7f00_0000_0000, 0x8000_0000)])).expect("plan");
    assert_ne!(a.layout(), b.layout());
    assert_eq!(a.layout(), &q35());
}

#[test]
fn ram_offset_hostile_inputs() {
    let p = RamPlan::new(&q35()).expect("plan");
    for &g in &[0u64, 1, u64::MAX, u64::MAX - 1, 1 << 63, 0x7fff_ffff] {
        for &n in &[0u64, 1, u64::MAX, 1 << 63, 0x8000_0000, 0x1_0000_0000] {
            let _ = p.ram_offset(g, n);
        }
    }
    assert_eq!(p.ram_offset(0, 0), None);
    assert_eq!(p.ram_offset(u64::MAX, 1), None);
}
