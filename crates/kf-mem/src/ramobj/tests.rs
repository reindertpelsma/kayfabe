//! Tests of the RAM objects: the table, the cut, the plan from a layout, and the cross-check that
//! the plan + table reach exactly the host bytes `translate` names.

use super::*;
use crate::guestmem::testvmm::TestVmm;
use crate::guestmem::{GuestMemory, LayoutGuard, RamBlockSpec};

const P: u64 = RAM_OBJ_ALIGN;

fn seg(ram_off: u64, len: u64, obj: u32, obj_off: u64) -> RamSegment {
    RamSegment {
        ram_off,
        len,
        obj,
        obj_off,
    }
}

fn blk(gpa: u64, len: u64) -> RamBlockSpec {
    RamBlockSpec { gpa, len }
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
            if let Ok(v) = t.split(o, n) {
                assert!(v.len() <= t.segments().len());
            }
        }
    }
}

// --------------------------------------------------------------- the plan

/// q35-like: below the hole, above 4 GiB.
fn q35() -> RamLayout {
    layout(&[blk(0, 0x8000_0000), blk(0x1_0000_0000, 0x8000_0000)])
}

#[test]
fn numbering_by_gpa_makes_one_object_per_block_and_a_hole_costs_nothing() {
    let p = RamPlan::by_gpa(&q35()).expect("plan");
    assert_eq!(
        p.segments(),
        &[
            PlanSegment {
                block: 0,
                gpa: 0,
                ram_off: 0,
                len: 0x8000_0000
            },
            PlanSegment {
                block: 1,
                gpa: 0x1_0000_0000,
                ram_off: 0x1_0000_0000,
                len: 0x8000_0000
            },
        ]
    );
    assert_eq!(p.total_bytes(), 0x1_0000_0000);
    assert_eq!(p.ram_offset(0x2100, 0x10), Some(0x2100));
    assert_eq!(p.ram_offset(0x1_0000_0100, 0x10), Some(0x1_0000_0100));
    assert_eq!(p.ram_offset(0x8000_0000, 8), None, "the hole");
    assert_eq!(p.ram_offset(0x7fff_fff8, 16), None, "across the hole");
}

#[test]
fn cumulative_numbering_is_dense_and_adjacent_blocks_are_one_run() {
    let l = layout(&[blk(0, P), blk(P, P), blk(4 * P, P)]);
    let p = RamPlan::cumulative(&l).expect("plan");
    assert_eq!(
        p.ram_offset(P - 8, 16),
        Some(P - 8),
        "across two adjacent blocks"
    );
    assert_eq!(p.ram_offset(4 * P, 8), Some(2 * P));
    assert_eq!(p.ram_offset(2 * P - 8, 16), None, "into the hole");
    assert_eq!(p.gpa_of(2 * P + 8), Some((4 * P + 8, P - 8)));
    assert_eq!(p.gpa_of(3 * P), None);
}

#[test]
fn a_run_is_cut_when_the_numbers_are_not_adjacent_even_if_the_blocks_are() {
    // Adjacent in guest-physical space, but the second block is numbered far away (a file whose
    // layout is not dense): the range across them has no single number.
    let l = layout(&[blk(0, P), blk(P, P)]);
    let p = RamPlan::new(&l, &[0, 10 * P]).expect("plan");
    assert_eq!(p.ram_offset(P - 8, 16), None);
    assert_eq!(p.ram_offset(P, 16), Some(10 * P));
    assert_eq!(p.gpa_of(10 * P + 8), Some((P + 8, P - 8)));
}

#[test]
fn plan_refuses_unaligned_empty_and_overlapping_numbers_by_name() {
    assert_eq!(
        RamPlan::by_gpa(&RamLayout::default()),
        Err(PlanError::Empty)
    );
    assert_eq!(
        RamPlan::by_gpa(&layout(&[blk(0, P), blk(P + 8, P)])),
        Err(PlanError::Unaligned { index: 1 })
    );
    assert_eq!(
        RamPlan::by_gpa(&layout(&[blk(0, P + 8)])),
        Err(PlanError::Unaligned { index: 0 })
    );
    assert_eq!(
        RamPlan::by_gpa(&layout(&[blk(8, P)])),
        Err(PlanError::Unaligned { index: 0 })
    );
    let l = layout(&[blk(0, 2 * P), blk(2 * P, P)]);
    assert_eq!(
        RamPlan::new(&l, &[0, P]),
        Err(PlanError::Numbering { index: 1 })
    );
    assert_eq!(
        RamPlan::new(&l, &[0]),
        Err(PlanError::Numbering { index: 1 })
    );
    assert_eq!(
        RamPlan::new(&l, &[u64::MAX - P + 1, 0]),
        Err(PlanError::Overflow)
    );
}

#[test]
fn numbers_by_gpa_do_not_move_when_another_block_goes() {
    let three = layout(&[blk(0, P), blk(4 * P, P), blk(8 * P, P)]);
    let two = layout(&[blk(0, P), blk(8 * P, P)]);
    let a = RamPlan::by_gpa(&three).expect("plan");
    let b = RamPlan::by_gpa(&two).expect("plan");
    for gpa in [0, 8 * P, 8 * P + 100] {
        assert_eq!(a.ram_offset(gpa, 16), b.ram_offset(gpa, 16), "{gpa:#x}");
    }
    // Cumulative numbering is NOT stable: the last block's number shifts.
    let ca = RamPlan::cumulative(&three).expect("plan");
    let cb = RamPlan::cumulative(&two).expect("plan");
    assert_ne!(ca.ram_offset(8 * P, 16), cb.ram_offset(8 * P, 16));
}

#[test]
fn plan_plus_table_cut_exactly_where_locate_cuts() {
    let mut r = Rng(0x00C0_FFEE_1234_5678);
    for _ in 0..1500 {
        // A random layout on the object grain, some blocks adjacent.
        let n = 1 + r.below(8);
        let mut gpa = r.below(3) * P;
        let mut blocks = Vec::new();
        for _ in 0..n {
            let len = (1 + r.below(4)) * P;
            blocks.push(blk(gpa, len));
            gpa += len;
            if r.below(3) == 0 {
                gpa += (1 + r.below(3)) * P;
            }
        }
        let l = layout(&blocks);
        let p = RamPlan::by_gpa(&l).expect("plan");
        let segs: Vec<RamSegment> = p
            .segments()
            .iter()
            .map(|s| seg(s.ram_off, s.len, 0x100 + s.block as u32, 0))
            .collect();
        let t = RamObjects::new(&segs).expect("table");
        let top = gpa + 2 * P;
        let (mut ok, mut refused) = (0u32, 0u32);
        for _ in 0..40 {
            let g = r.below(top);
            let len = 1 + r.below(3 * P);
            let located = l.locate(g, len);
            // With numbers = gpa, the plan numbers a range iff it is wholly RAM...
            match (p.ram_offset(g, len), &located) {
                (Some(off), Ok(runs)) => {
                    ok += 1;
                    assert_eq!(off, g);
                    // ...and the table cuts it exactly where the layout does: same blocks, lengths.
                    let pieces = t.split(off, len).expect("split");
                    assert_eq!(pieces.len(), runs.len(), "{blocks:?} {g:#x}+{len:#x}");
                    for (piece, run) in pieces.iter().zip(runs) {
                        assert_eq!(piece.obj, 0x100 + run.index() as u32);
                        assert_eq!(piece.obj_off, run.off());
                        assert_eq!(piece.len, run.len());
                    }
                }
                (None, Err(_)) => refused += 1,
                // Adjacent-in-gpa blocks are number-adjacent under by_gpa, so a located range is
                // always numbered and vice versa.
                (a, b) => panic!("plan {a:?} vs locate {b:?} for {g:#x}+{len:#x} in {blocks:?}"),
            }
        }
        assert!(ok + refused == 40);
    }
}

#[test]
fn ram_offset_hostile_inputs() {
    let p = RamPlan::by_gpa(&q35()).expect("plan");
    for (g, n) in [
        (0, 0),
        (u64::MAX, 1),
        (u64::MAX, u64::MAX),
        (1, u64::MAX),
        (0x1_0000_0000, u64::MAX),
        (0x1_7fff_ffff, 2),
    ] {
        assert_eq!(p.ram_offset(g, n), None, "{g:#x}+{n:#x}");
    }
    assert_eq!(p.gpa_of(u64::MAX), None);
    assert_eq!(p.gpa_of(0x9000_0000), None);
}

// ---------------------------------------------------- the book of objects

use std::cell::RefCell;

/// A hook run during the Nth allocation.
type During = (usize, Box<dyn Fn(&TestVmm)>);

/// A host that records every allocation and release, and can be told to fail or to move the
/// VMM's layout in the middle of an allocation (the "RAM block freed between translate and the
/// RM pin" race).
struct FakeHost<'v> {
    vmm: &'v TestVmm,
    next: u32,
    live: Vec<u32>,
    log: std::rc::Rc<RefCell<Vec<String>>>,
    fail_on_alloc: Option<usize>,
    allocs: usize,
    /// Run during the Nth allocation (after the segment was handed over, before it returns).
    during: Option<During>,
}

impl<'v> FakeHost<'v> {
    fn new(vmm: &'v TestVmm) -> Self {
        Self {
            vmm,
            next: 0x500,
            live: Vec::new(),
            log: std::rc::Rc::default(),
            fail_on_alloc: None,
            allocs: 0,
            during: None,
        }
    }
}

impl RamObjectHost for FakeHost<'_> {
    fn alloc(&mut self, segment: &kf_linux_raw::RamSegment<'_>) -> Result<u32, String> {
        self.allocs += 1;
        if self.fail_on_alloc == Some(self.allocs) {
            return Err("RM said no".into());
        }
        if let Some((n, f)) = &self.during
            && *n == self.allocs
        {
            f(self.vmm);
        }
        // The pin reads the memory: prove the segment is usable (the guard holds the block).
        let mut b = [0u8; 1];
        segment
            .read_into(kf_linux_raw::HostOffset::ZERO, &mut b)
            .map_err(|e| e.to_string())?;
        self.next += 1;
        self.live.push(self.next);
        self.log
            .borrow_mut()
            .push(format!("alloc {:#x} len {:#x}", self.next, segment.len()));
        Ok(self.next)
    }
    fn free(&mut self, obj: u32) {
        self.live.retain(|o| *o != obj);
        self.log.borrow_mut().push(format!("free {obj:#x}"));
    }
}

fn by_gpa(b: &RamBlockSpec) -> Option<u64> {
    Some(b.gpa)
}

fn vmm3() -> TestVmm {
    let v = TestVmm::new();
    v.add(0, P);
    v.add(4 * P, 2 * P);
    v.add(8 * P, P);
    v
}

#[test]
fn the_book_makes_one_object_per_block_and_a_second_sync_at_the_same_epoch_is_free() {
    let vmm = vmm3();
    let mut host = FakeHost::new(&vmm);
    let mut book = RamObjectBook::new();
    let mut unmaps = 0;
    let r = book
        .sync(&vmm, &by_gpa, &mut host, &mut |_| {
            unmaps += 1;
            Ok(())
        })
        .expect("sync");
    assert_eq!((r.added, r.kept, r.removed), (3, 0, 0));
    assert_eq!(host.live.len(), 3);
    assert!(book.is_current(&vmm));
    let t = book.objects().expect("table");
    assert_eq!(t.segments().len(), 3);
    assert_eq!(t.resolve(4 * P + 8, 8).map(|(_, off)| off), Ok(8));
    // The middle block's object is the second one made.
    assert_eq!(t.resolve(4 * P, 8).map(|(o, _)| o), Ok(0x502));
    assert!(t.resolve(2 * P, 8).is_err(), "a hole is not carried");
    assert!(t.resolve(5 * P, 2 * P).is_err() || t.resolve(5 * P, P).is_ok());
    let before = host.log.borrow().len();
    let again = book
        .sync(&vmm, &by_gpa, &mut host, &mut |_| Ok(()))
        .expect("sync");
    assert_eq!((again.added, again.kept, again.removed), (0, 3, 0));
    assert_eq!(
        host.log.borrow().len(),
        before,
        "no host call at the same epoch"
    );
    assert_eq!(unmaps, 0);
}

#[test]
fn a_layout_change_between_translate_and_descriptor_creation_is_refused_by_name() {
    let vmm = vmm3();
    let mut host = FakeHost::new(&vmm);
    // While the SECOND object is being made, the VMM frees the third block.
    host.during = Some((2, Box::new(|v: &TestVmm| v.remove(8 * P))));
    let mut book = RamObjectBook::new();
    let e = book
        .sync(&vmm, &by_gpa, &mut host, &mut |_| Ok(()))
        .expect_err("must refuse");
    assert_eq!(e, SyncError::LayoutChanged);
    assert!(e.to_string().starts_with("LAYOUT_CHANGED"));
    // Everything this call made was freed; nothing was committed; the epoch did not advance.
    assert!(host.live.is_empty(), "leaked {:?}", host.live);
    assert!(book.blocks().is_empty());
    assert_eq!(book.epoch(), None);
    assert!(!book.is_current(&vmm));
    // Not retried silently: the SECOND call (a new refresh) sees two blocks and succeeds.
    host.during = None;
    let r = book
        .sync(&vmm, &by_gpa, &mut host, &mut |_| Ok(()))
        .expect("retry");
    assert_eq!(r.added, 2);
    assert_eq!(book.blocks().len(), 2);
    assert!(book.is_current(&vmm));
}

#[test]
fn removing_a_block_unmaps_its_slices_then_frees_its_object_and_keeps_the_others() {
    let vmm = vmm3();
    let mut host = FakeHost::new(&vmm);
    let mut book = RamObjectBook::new();
    book.sync(&vmm, &by_gpa, &mut host, &mut |_| Ok(()))
        .expect("first");
    let objs_before = book.blocks().to_vec();
    let order: RefCell<Vec<String>> = RefCell::new(Vec::new());
    vmm.remove(4 * P);
    let log = host.log.clone();
    let r = {
        let mut unmap = |t: &Retiring| {
            // The object must still be live while its GPU slices are taken down.
            order.borrow_mut().push(format!(
                "unmap {:#x}+{:#x} obj {:#x}",
                t.ram_off, t.len, t.obj
            ));
            assert!(
                !log.borrow()
                    .iter()
                    .any(|l| l == &format!("free {:#x}", t.obj)),
                "freed before unmapped"
            );
            Ok(())
        };
        book.sync(&vmm, &by_gpa, &mut host, &mut unmap)
            .expect("sync")
    };
    assert_eq!((r.added, r.kept, r.removed), (0, 2, 1));
    assert_eq!(
        order.borrow().as_slice(),
        [format!(
            "unmap {:#x}+{:#x} obj {:#x}",
            4 * P,
            2 * P,
            objs_before[1].obj
        )]
    );
    assert!(!host.live.contains(&objs_before[1].obj));
    // The kept blocks keep their object AND their numbers.
    assert_eq!(book.blocks(), &[objs_before[0], objs_before[2]]);
    assert!(book.objects().expect("table").resolve(4 * P, 8).is_err());
}

#[test]
fn a_resize_retires_the_old_object_and_builds_one_over_the_new_length_without_moving_the_base() {
    let vmm = TestVmm::new();
    vmm.add_resizeable(0, P, 4 * P);
    vmm.fill(0, |i| (i % 251) as u8);
    let mut host = FakeHost::new(&vmm);
    let mut book = RamObjectBook::new();
    book.sync(&vmm, &by_gpa, &mut host, &mut |_| Ok(()))
        .expect("first");
    let old = book.blocks()[0];
    vmm.resize(0, 3 * P);
    let mut retired = Vec::new();
    let r = book
        .sync(&vmm, &by_gpa, &mut host, &mut |t| {
            retired.push((t.ram_off, t.len));
            Ok(())
        })
        .expect("resize");
    assert_eq!((r.added, r.removed, r.kept), (1, 1, 0));
    assert_eq!(retired, vec![(0, P)]);
    let new = book.blocks()[0];
    assert_ne!(new.obj, old.obj);
    assert_eq!(new.len, 3 * P);
    // The base did not move: the bytes of the old range are the same bytes.
    let g = vmm.lock_layout().expect("layout");
    let mut b = [0u8; 4];
    g.translate(8, 4)
        .expect("t")
        .read_into(&mut b)
        .expect("read");
    assert_eq!(b, [8, 9, 10, 11]);
}

#[test]
fn a_failed_unmap_keeps_the_object_and_the_next_sync_retries_it() {
    let vmm = vmm3();
    let mut host = FakeHost::new(&vmm);
    let mut book = RamObjectBook::new();
    book.sync(&vmm, &by_gpa, &mut host, &mut |_| Ok(()))
        .expect("first");
    vmm.remove(8 * P);
    let e = book
        .sync(&vmm, &by_gpa, &mut host, &mut |_| Err("RM busy".into()))
        .expect_err("unmap refused");
    assert_eq!(e, SyncError::Unmap("RM busy".into()));
    assert_eq!(
        host.live.len(),
        3,
        "not freed while its slices may still be mapped"
    );
    assert_eq!(book.blocks().len(), 3);
    assert!(!book.is_current(&vmm));
    let r = book
        .sync(&vmm, &by_gpa, &mut host, &mut |_| Ok(()))
        .expect("retry");
    assert_eq!(r.removed, 1);
    assert_eq!(host.live.len(), 2);
    assert!(book.is_current(&vmm));
}

#[test]
fn a_host_refusal_frees_what_the_call_made_and_commits_nothing() {
    let vmm = vmm3();
    let mut host = FakeHost::new(&vmm);
    host.fail_on_alloc = Some(3);
    let mut book = RamObjectBook::new();
    let e = book
        .sync(&vmm, &by_gpa, &mut host, &mut |_| Ok(()))
        .expect_err("refused");
    assert_eq!(e, SyncError::Host("RM said no".into()));
    assert!(host.live.is_empty());
    assert_eq!(book.epoch(), None);
    host.fail_on_alloc = None;
    assert!(book.sync(&vmm, &by_gpa, &mut host, &mut |_| Ok(())).is_ok());
    assert_eq!(host.live.len(), 3);
}

#[test]
fn a_layout_that_is_not_a_plan_is_refused_by_name_before_any_host_call() {
    let vmm = TestVmm::new();
    vmm.add(8, P); // not on the object grain
    let mut host = FakeHost::new(&vmm);
    let mut book = RamObjectBook::new();
    let e = book
        .sync(&vmm, &by_gpa, &mut host, &mut |_| Ok(()))
        .expect_err("unaligned");
    assert!(matches!(e, SyncError::Layout(_)), "{e:?}");
    assert!(host.log.borrow().is_empty());
    let none = TestVmm::new();
    let mut host2 = FakeHost::new(&none);
    assert!(matches!(
        book.sync(&none, &by_gpa, &mut host2, &mut |_| Ok(())),
        Err(SyncError::Layout(_))
    ));
}
