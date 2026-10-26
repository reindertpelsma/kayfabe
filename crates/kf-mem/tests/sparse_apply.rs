//! SPR1: observe every target transition, including unchanged neighbours.

use kf_cuda::abi::{KFWR_ACK_APPLIED, KFWR_ACK_FAILED, KFWR_ACK_HELD, KfMapRun};
use kf_mem::apply::{Applied, ApplyCfg, DiffRun, PermPolicy, apply_entry};
use kf_mem::ledger::{Desired, MapTarget, Mapped, OwnView};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

const P: u64 = 4096;
const L: u64 = 65536;
const VA: u64 = 0x20_0000;
const SPARSE: u32 = 1 << 7;
const SPARSE_KIND: u8 = 0x80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Invalid,
    Sparse,
    Backed(bool, u64),
}

struct Target {
    enabled: bool,
    block_sparse: Cell<bool>,
    pages: RefCell<BTreeMap<u64, State>>,
    calls: RefCell<Vec<&'static str>>,
    guard: RefCell<BTreeMap<u64, State>>,
    violations: RefCell<Vec<String>>,
    refuse_sparse_retire: bool,
    refuse_backed_unmap: bool,
    reserved: bool,
    no_invalid: Cell<bool>,
}

impl Target {
    fn new(sparse: bool) -> Self {
        Self {
            enabled: true,
            block_sparse: Cell::new(sparse),
            pages: RefCell::new(
                (0..L / P)
                    .map(|i| {
                        (
                            VA + i * P,
                            if sparse {
                                State::Sparse
                            } else {
                                State::Invalid
                            },
                        )
                    })
                    .collect(),
            ),
            calls: RefCell::new(vec![]),
            guard: RefCell::new(BTreeMap::new()),
            violations: RefCell::new(vec![]),
            refuse_sparse_retire: false,
            refuse_backed_unmap: false,
            reserved: false,
            no_invalid: Cell::new(false),
        }
    }

    fn seed(&self, ram: bool, off: u64) {
        for i in 0..L / P {
            self.pages
                .borrow_mut()
                .insert(VA + i * P, State::Backed(ram, off + i * P));
        }
    }

    fn check(&self) {
        let pages = self.pages.borrow();
        if self.no_invalid.get() && pages.values().any(|s| *s == State::Invalid) {
            self.violations
                .borrow_mut()
                .push("transient invalid gap".into());
        }
        for (&va, state) in self.guard.borrow().iter() {
            if pages.get(&va) != Some(state) {
                self.violations
                    .borrow_mut()
                    .push(format!("unchanged {va:#x}"));
            }
        }
    }

    fn all(&self, expected: State) -> bool {
        self.pages.borrow().values().all(|s| *s == expected)
    }
}

impl MapTarget for Target {
    fn own_view(&self, va: u64, end: u64) -> Option<OwnView> {
        Some(OwnView {
            owned: self
                .pages
                .borrow()
                .iter()
                .filter_map(|(&p, s)| {
                    (matches!(s, State::Backed(..)) && p < end && va < p + P)
                        .then_some((p.max(va), (p + P).min(end)))
                })
                .collect(),
            rigid: vec![],
        })
    }
    fn sparse_twin(&self) -> bool {
        self.enabled
    }
    fn gpu_space(&self) -> bool {
        true
    }

    fn map_sparse(&self, va: u64, len: u64, _: bool) -> Result<Mapped, String> {
        self.calls.borrow_mut().push("sparse-intent");
        assert!(self.enabled);
        if !self.block_sparse.get() {
            if self
                .pages
                .borrow()
                .values()
                .any(|s| matches!(s, State::Backed(..)))
            {
                return Ok(Mapped::HeldByHost);
            }
            assert_eq!((va, len), (VA, L));
            self.block_sparse.set(true);
            self.pages
                .borrow_mut()
                .values_mut()
                .for_each(|s| *s = State::Sparse);
        }
        self.check();
        Ok(Mapped::Placed)
    }

    fn unmap_sparse(&self, _: u64, len: u64, replacement: bool, _: bool) -> Result<u64, String> {
        self.calls.borrow_mut().push("sparse-retire");
        if self.refuse_sparse_retire {
            return Err("sparse retire refused".into());
        }
        if replacement {
            return Ok(0);
        }
        let backed = self
            .pages
            .borrow()
            .values()
            .any(|s| matches!(s, State::Backed(..)));
        if len == L && !backed {
            self.block_sparse.set(false);
            self.pages
                .borrow_mut()
                .values_mut()
                .for_each(|s| *s = State::Invalid);
            self.check();
            Ok(0)
        } else {
            self.check();
            Ok(len)
        }
    }

    fn map(&self, d: &Desired, _: bool) -> Result<Mapped, String> {
        self.calls.borrow_mut().push("backed-map");
        for i in 0..d.len / P {
            self.pages
                .borrow_mut()
                .insert(d.va + i * P, State::Backed(d.ram, d.off + i * P));
        }
        self.check();
        Ok(Mapped::Placed)
    }

    fn unmap(&self, _: u64, _: bool) -> Result<(), String> {
        panic!("range-capable target must use exact backed ranges")
    }

    fn unmap_range(&self, va: u64, len: u64, _: bool) -> Result<(), String> {
        self.calls.borrow_mut().push("backed-unmap");
        if self.refuse_backed_unmap {
            return Err("backed unmap refused".into());
        }
        for i in 0..len / P {
            let at = va + i * P;
            if !matches!(self.pages.borrow().get(&at), Some(State::Backed(..))) {
                self.violations
                    .borrow_mut()
                    .push("unmap without a backed mapping".into());
            }
            self.pages.borrow_mut().insert(
                at,
                if self.block_sparse.get() {
                    State::Sparse
                } else {
                    State::Invalid
                },
            );
        }
        self.check();
        Ok(())
    }

    fn invalidate(&self) -> Result<(), String> {
        self.calls.borrow_mut().push("invalidate");
        self.check();
        Ok(())
    }

    fn reserved(&self) -> Vec<(u64, u64)> {
        if self.reserved {
            vec![(VA, VA + L)]
        } else {
            vec![]
        }
    }
}

fn run(sparse: bool, unmap: bool, va: u64, len: u64, off: u64, ram: bool) -> DiffRun {
    PermPolicy::default().diff_run(&KfMapRun {
        va,
        len,
        gpga: off,
        flags: if sparse {
            SPARSE
        } else if ram {
            2
        } else {
            0
        },
        op: if unmap { 2 } else { 1 },
        pdb_index: 0,
    })
}

fn apply(t: &Target, runs: &[DiffRun]) -> Applied {
    let out = apply_entry(
        t,
        runs,
        &ApplyCfg {
            store_bytes: 1 << 30,
            grain: P,
            ram_offset: &|g, _| Some(g),
            usermode: None,
            per_map_kind: true,
            carve: 1 << 30,
            carve_refuse: false,
        },
    );
    assert!(
        t.violations.borrow().is_empty(),
        "{:?}",
        t.violations.borrow()
    );
    out
}

fn backed_to_sparse(ram: bool, off: u64) {
    let t = Target::new(true);
    t.seed(ram, off);
    t.no_invalid.set(true);
    let out = apply(
        &t,
        &[
            run(false, true, VA, L, off, ram),
            run(true, false, VA, L, 0, false),
        ],
    );
    assert_eq!(out.codes, [KFWR_ACK_APPLIED, KFWR_ACK_APPLIED]);
    assert!(t.all(State::Sparse));
    assert_eq!(
        &*t.calls.borrow(),
        &["sparse-intent", "backed-unmap", "invalidate"]
    );
}

#[test]
fn vidmem_to_sparse_is_one_backed_unmap() {
    backed_to_sparse(false, 0x10_0000);
}
#[test]
fn dma_to_sparse_is_one_backed_unmap() {
    backed_to_sparse(true, 0x20_0000);
}
#[test]
fn framebuffer_zero_is_not_kept_when_it_becomes_sparse() {
    backed_to_sparse(false, 0);
}

fn sparse_to_backed(ram: bool) {
    let t = Target::new(true);
    t.no_invalid.set(true);
    let out = apply(
        &t,
        &[
            run(true, true, VA, L, 0, false),
            run(false, false, VA, L, 0x10_0000, ram),
        ],
    );
    assert_eq!(out.codes, [KFWR_ACK_APPLIED, KFWR_ACK_APPLIED]);
    assert_eq!(t.pages.borrow()[&VA], State::Backed(ram, 0x10_0000));
    assert_eq!(
        &*t.calls.borrow(),
        &["sparse-retire", "backed-map", "invalidate"]
    );
}

#[test]
fn sparse_to_vidmem_is_one_map_without_backed_unmap() {
    sparse_to_backed(false);
}
#[test]
fn sparse_to_dma_is_one_map_without_backed_unmap() {
    sparse_to_backed(true);
}

fn normal_fallback(ram: bool) {
    let t = Target::new(false);
    t.seed(ram, 0x10_0000);
    let out = apply(
        &t,
        &[
            run(false, true, VA, L, 0x10_0000, ram),
            run(true, false, VA, L, 0, false),
        ],
    );
    assert_eq!(out.codes, [KFWR_ACK_APPLIED, KFWR_ACK_HELD]);
    assert!(
        t.all(State::Invalid),
        "populated normal block must not be sparsified"
    );
    assert_eq!(
        &*t.calls.borrow(),
        &["sparse-intent", "backed-unmap", "invalidate"]
    );
}

#[test]
fn a_populated_normal_vidmem_block_falls_back_to_invalid() {
    normal_fallback(false);
}
#[test]
fn a_populated_normal_dma_block_falls_back_to_invalid() {
    normal_fallback(true);
}

#[test]
fn sparse_unmap_can_return_an_empty_block_to_invalid() {
    let t = Target::new(true);
    let out = apply(&t, &[run(true, true, VA, L, 0, false)]);
    assert_eq!(out.codes, [KFWR_ACK_APPLIED]);
    assert!(t.all(State::Invalid));
    assert_eq!(&*t.calls.borrow(), &["sparse-retire", "invalidate"]);
}

#[test]
fn sparse_unmap_preserves_a_backed_neighbour() {
    let t = Target::new(true);
    t.pages
        .borrow_mut()
        .insert(VA + P, State::Backed(false, 0x80_0000));
    t.guard
        .borrow_mut()
        .insert(VA + P, State::Backed(false, 0x80_0000));
    let out = apply(&t, &[run(true, true, VA, P, 0, false)]);
    assert_eq!(out.codes, [KFWR_ACK_APPLIED]);
    assert_eq!(out.sparse_retained_bytes, P);
    assert_eq!(t.pages.borrow()[&VA], State::Sparse);
    assert_eq!(&*t.calls.borrow(), &["sparse-retire", "invalidate"]);
}

#[test]
fn partial_sparse_retirement_is_counted_when_the_block_cannot_be_freed() {
    let t = Target::new(true);
    let out = apply(&t, &[run(true, true, VA + P, 2 * P, 0, false)]);
    assert_eq!(out.sparse_retained_bytes, 2 * P);
    assert!(t.all(State::Sparse));
}

fn failed_backed_unmap_retires_new_sparse_intent(ram: bool) {
    let mut t = Target::new(true);
    t.seed(ram, 0x10_0000);
    t.refuse_backed_unmap = true;
    let out = apply(
        &t,
        &[
            run(false, true, VA, L, 0x10_0000, ram),
            run(true, false, VA, L, 0, false),
        ],
    );
    assert_eq!(out.codes, [KFWR_ACK_FAILED, KFWR_ACK_FAILED]);
    assert!(t.calls.borrow().contains(&"sparse-intent"));
    assert!(t.calls.borrow().contains(&"sparse-retire"));
    assert!(!t.calls.borrow().contains(&"backed-map"));
}

#[test]
fn a_failed_vidmem_unmap_retires_the_new_sparse_intent() {
    failed_backed_unmap_retires_new_sparse_intent(false);
}

#[test]
fn a_failed_dma_unmap_retires_the_new_sparse_intent() {
    failed_backed_unmap_retires_new_sparse_intent(true);
}

#[test]
fn unchanged_sparse_neighbours_stay_accessible_during_a_middle_map() {
    let t = Target::new(true);
    for i in 0..L / P {
        if i != 4 {
            t.guard.borrow_mut().insert(VA + i * P, State::Sparse);
        }
    }
    let out = apply(
        &t,
        &[
            run(true, true, VA, L, 0, false),
            run(true, false, VA, 4 * P, 0, false),
            run(false, false, VA + 4 * P, P, 0x80_0000, false),
            run(true, false, VA + 5 * P, L - 5 * P, 0, false),
        ],
    );
    assert!(out.codes.iter().all(|&c| c == KFWR_ACK_APPLIED));
    assert_eq!(
        t.calls
            .borrow()
            .iter()
            .filter(|&&c| c == "sparse-retire")
            .count(),
        1
    );
    assert!(!t.calls.borrow().contains(&"backed-unmap"));
    assert!(!t.calls.borrow().contains(&"sparse-intent"));
}

#[test]
fn unchanged_backed_neighbours_stay_accessible_during_a_sparse_change() {
    let t = Target::new(true);
    t.seed(false, 0x10_0000);
    for i in 0..L / P {
        if i != 4 {
            t.guard
                .borrow_mut()
                .insert(VA + i * P, State::Backed(false, 0x10_0000 + i * P));
        }
    }
    let out = apply(
        &t,
        &[
            run(false, true, VA, L, 0x10_0000, false),
            run(false, false, VA, 4 * P, 0x10_0000, false),
            run(true, false, VA + 4 * P, P, 0, false),
            run(
                false,
                false,
                VA + 5 * P,
                L - 5 * P,
                0x10_0000 + 5 * P,
                false,
            ),
        ],
    );
    assert!(out.codes.iter().all(|&c| c == KFWR_ACK_APPLIED));
    assert_eq!(t.pages.borrow()[&(VA + 4 * P)], State::Sparse);
}

#[test]
fn sparse_decoder_carries_a_third_kind_without_a_backing() {
    let d = run(true, false, VA, L, 0, false);
    assert_eq!(d.ap, SPARSE_KIND);
}

#[test]
fn sparse_decoder_canonicalizes_payload_that_has_no_meaning() {
    let d = PermPolicy::default().diff_run(&KfMapRun {
        va: VA,
        len: L,
        gpga: u64::MAX,
        flags: SPARSE | 0xff_0078,
        op: 1,
        pdb_index: 0,
    });
    assert_eq!(
        (d.ap, d.at, d.kind, d.privileged),
        (SPARSE_KIND, 0, 0, false)
    );
    assert_eq!(d.perm, kf_host::MapPerm::READ_WRITE);
}

#[test]
fn a_refused_sparse_retirement_does_not_publish_the_new_vidmem_map() {
    let mut t = Target::new(true);
    t.refuse_sparse_retire = true;
    let out = apply(
        &t,
        &[
            run(true, true, VA, L, 0, false),
            run(false, false, VA, L, 0x10_0000, false),
        ],
    );
    assert_eq!(out.codes, [KFWR_ACK_FAILED, KFWR_ACK_FAILED]);
    assert!(!t.calls.borrow().contains(&"backed-map"));
    assert_eq!(out.unmap_refused, 1);
}

#[test]
fn a_refused_sparse_retirement_does_not_publish_the_new_dma_map() {
    let mut t = Target::new(true);
    t.refuse_sparse_retire = true;
    let out = apply(
        &t,
        &[
            run(true, true, VA, L, 0, false),
            run(false, false, VA, L, 0x10_0000, true),
        ],
    );
    assert_eq!(out.codes, [KFWR_ACK_FAILED, KFWR_ACK_FAILED]);
    assert!(!t.calls.borrow().contains(&"backed-map"));
}

#[test]
fn disabled_sparse_never_creates_a_backing_or_a_sparse_block() {
    let mut t = Target::new(false);
    t.enabled = false;
    let out = apply(&t, &[run(true, false, VA, L, 0, false)]);
    assert_eq!(out.codes, [KFWR_ACK_HELD]);
    assert!(t.all(State::Invalid));
    assert!(t.calls.borrow().is_empty());
}

#[test]
fn disabled_sparse_still_unmaps_an_old_backing_to_invalid() {
    let mut t = Target::new(false);
    t.enabled = false;
    t.seed(false, 0x10_0000);
    let out = apply(
        &t,
        &[
            run(false, true, VA, L, 0x10_0000, false),
            run(true, false, VA, L, 0, false),
        ],
    );
    assert_eq!(out.codes, [KFWR_ACK_APPLIED, KFWR_ACK_HELD]);
    assert!(t.all(State::Invalid));
    assert_eq!(&*t.calls.borrow(), &["backed-unmap", "invalidate"]);
}

#[test]
fn sparse_never_reserves_a_vmm_owned_address() {
    let mut t = Target::new(false);
    t.reserved = true;
    let out = apply(&t, &[run(true, false, VA, L, 0, false)]);
    assert_eq!(out.codes, [KFWR_ACK_FAILED]);
    assert_eq!(out.vmm_overlaps, 1);
    assert!(t.calls.borrow().is_empty());
}

#[test]
fn sparse_ranges_must_not_wrap() {
    let t = Target::new(false);
    let out = apply(&t, &[run(true, false, u64::MAX - P + 1, 2 * P, 0, false)]);
    assert_eq!(out.codes, [KFWR_ACK_FAILED]);
    assert!(t.calls.borrow().is_empty());
}

#[test]
fn sparse_resegmentation_keeps_every_page_without_a_host_call() {
    let t = Target::new(true);
    t.no_invalid.set(true);
    for i in 0..L / P {
        t.guard.borrow_mut().insert(VA + i * P, State::Sparse);
    }
    let out = apply(
        &t,
        &[
            run(true, true, VA, L, 0, false),
            run(true, false, VA, L / 2, 0, false),
            run(true, false, VA + L / 2, L / 2, 0, false),
        ],
    );
    assert_eq!(out.codes, [KFWR_ACK_APPLIED; 3]);
    assert!(t.calls.borrow().is_empty());
}

#[test]
fn sparse_never_claims_part_of_an_existing_vmm_owned_block() {
    let mut t = Target::new(true);
    t.reserved = true;
    let out = apply(&t, &[run(true, false, VA + P, P, 0, false)]);
    assert_eq!(out.codes, [KFWR_ACK_FAILED]);
    assert_eq!(out.vmm_overlaps, 1);
    assert!(t.calls.borrow().is_empty());
}

#[test]
fn larger_sparse_ranges_must_not_wrap() {
    let t = Target::new(false);
    let out = apply(&t, &[run(true, false, u64::MAX - L + 1, 2 * L, 0, false)]);
    assert_eq!(out.codes, [KFWR_ACK_FAILED]);
    assert!(t.calls.borrow().is_empty());
}

fn held_sparse_retirement_refusal(ram: bool) {
    let mut t = Target::new(true);
    t.refuse_sparse_retire = true;
    let mut old = run(true, true, VA, L, 0, false);
    old.held = true;
    let out = apply(&t, &[old, run(false, false, VA, L, 0x10_0000, ram)]);
    assert_eq!(out.codes, [KFWR_ACK_FAILED, KFWR_ACK_FAILED]);
    assert!(!t.calls.borrow().contains(&"backed-map"));
}

#[test]
fn held_sparse_retirement_failure_blocks_vidmem() {
    held_sparse_retirement_refusal(false);
}
#[test]
fn held_sparse_retirement_failure_blocks_dma() {
    held_sparse_retirement_refusal(true);
}

fn unflagged_aperture_four(flags: u32) {
    let t = Target::new(false);
    let d = PermPolicy::default().diff_run(&KfMapRun {
        va: VA,
        len: L,
        gpga: 0,
        flags,
        op: 1,
        pdb_index: 0,
    });
    let out = apply(&t, &[d]);
    assert_eq!(out.codes, [KFWR_ACK_FAILED]);
    assert!(t.calls.borrow().is_empty());
}
#[test]
fn an_unflagged_reserved_aperture_is_not_sparse() {
    unflagged_aperture_four(4);
}
#[test]
fn an_unflagged_reserved_aperture_with_a_kind_is_not_sparse() {
    unflagged_aperture_four(4 | (5 << 16));
}
