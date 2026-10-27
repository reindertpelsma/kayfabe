#[test]
fn ens11_queued_same_space_splits_share_one_post_request_walk() {
    let mut r = rig();
    r.m.queue_split(None, Some(K_A), 710, r.port.trigger());
    r.m.queue_split(None, Some(K_A), 711, r.port.trigger());
    assert!(
        r.m.walker().submits.is_empty(),
        "admit the complete batch before submit"
    );
    r.m.start_queued_splits(r.port.trigger());
    assert_eq!(r.m.walker().submits.len(), 1);
    assert!(r.m.take_splits().is_empty());
    r.m.on_walk_ready(r.port.trigger());
    assert_eq!(r.m.take_splits(), vec![(710, Ok(())), (711, Ok(()))]);
    assert_eq!(r.m.walker().submits.len(), 1);
}
#[test]
fn ens11_queued_same_pdb_splits_are_not_walked_twice() {
    let mut r = rig();
    r.m.queue_split(Some(PDB_A), None, 720, r.port.trigger());
    r.m.queue_split(Some(PDB_A), None, 721, r.port.trigger());
    assert!(r.m.walker().submits.is_empty());
    r.m.start_queued_splits(r.port.trigger());
    r.m.on_walk_ready(r.port.trigger());
    assert_eq!(r.m.take_splits(), vec![(720, Ok(())), (721, Ok(()))]);
    assert_eq!(r.m.walker().submits.len(), 1);
}
#[test]
fn ens11_split_arriving_during_walk_waits_for_the_next_walk() {
    let mut r = rig();
    r.m.queue_split(None, Some(K_A), 730, r.port.trigger());
    r.m.start_queued_splits(r.port.trigger());
    r.m.queue_split(None, Some(K_A), 731, r.port.trigger());
    r.m.start_queued_splits(r.port.trigger());
    r.m.on_walk_ready(r.port.trigger());
    assert_eq!(r.m.take_splits(), vec![(730, Ok(()))]);
    assert_eq!(r.m.walker().submits.len(), 2);
    r.m.on_walk_ready(r.port.trigger());
    assert_eq!(r.m.take_splits(), vec![(731, Ok(()))]);
}
#[test]
fn ens11_a_chain_of_memops_does_not_starve_a_different_space() {
    let mut r = rig();
    for n in 0..100u64 {
        let first = 1000 + n * 3;
        r.m.queue_split(None, Some(K_A), first, r.port.trigger());
        r.m.queue_split(None, Some(K_A), first + 1, r.port.trigger());
        r.m.queue_split(None, Some(K_B), first + 2, r.port.trigger());
        r.m.start_queued_splits(r.port.trigger());
        r.m.on_walk_ready(r.port.trigger());
        let done = r.m.take_splits();
        assert_eq!(
            done,
            vec![(first, Ok(())), (first + 1, Ok(())), (first + 2, Ok(()))]
        );
        assert_eq!(r.m.walker().submits.len(), n as usize + 1);
        assert_eq!(r.m.pending(), 0);
    }
}
#[test]
fn ens11_empty_diff_repeated_splits_do_not_remap_unchanged_rows() {
    let mut r = rig();
    r.tables
        .borrow_mut()
        .insert(PDB_A, vec![(0x4000_0000, 0x0300_0000, 0x1000, 0)]);
    r.m.on_split(Some(PDB_A), 740, r.port.trigger());
    r.m.on_walk_ready(r.port.trigger());
    assert_eq!(r.m.take_splits(), vec![(740, Ok(()))]);
    r.ops.borrow_mut().clear();
    for ticket in 741..751 {
        r.m.on_split(Some(PDB_A), ticket, r.port.trigger());
        let out = r.m.on_walk_ready(r.port.trigger());
        assert_eq!(out.applied[0].1.mapped, 0);
        assert_eq!(out.applied[0].1.unmapped, 0);
        assert_eq!(r.m.take_splits(), vec![(ticket, Ok(()))]);
        assert!(
            r.ops.borrow().is_empty(),
            "unchanged translations make no host mapping call"
        );
    }
}

struct Ens11LayoutTarget {
    refuse_refresh: std::cell::Cell<bool>,
    key: VasKey,
    visits: Rc<RefCell<Vec<VasKey>>>,
}
impl MapTarget for Ens11LayoutTarget {
    fn end_refresh(&self) -> Result<(), String> {
        if self.refuse_refresh.get() {
            Err("ENS11_SPARSE_READY_REFUSED".into())
        } else {
            Ok(())
        }
    }
    fn sparse_twin(&self) -> bool {
        true
    }
    fn retry_deferred_layout(&self) -> Result<bool, String> {
        assert!(!kf_host::sparse::slice::in_split());
        self.visits.borrow_mut().push(self.key);
        Ok(true)
    }
    fn map(&self, _: &Desired, _: bool) -> Result<Mapped, String> {
        Ok(Mapped::Placed)
    }
    fn unmap(&self, _: u64, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn invalidate(&self) -> Result<(), String> {
        Ok(())
    }
}
type Ens11LayoutManager = VaManager<ModelWalker, Ens11LayoutTarget>;
fn ens11_layout_manager() -> (Ens11LayoutManager, Rc<RefCell<Vec<VasKey>>>) {
    let visits = Rc::new(RefCell::new(Vec::new()));
    let mut m = VaManager::new(
        ModelWalker::new(Rc::default()),
        STORE,
        Box::new(|g, _| Some(g)),
    );
    for (key, pdb) in [(K_A, PDB_A), (K_B, PDB_B)] {
        m.table.insert(
            key,
            Ens11LayoutTarget {
                refuse_refresh: std::cell::Cell::new(false),
                key,
                visits: visits.clone(),
            },
        );
        m.table.set_root(key, pdb, PdbAperture::Vidmem).unwrap();
    }
    (m, visits)
}
#[test]
fn ens11_deferred_layout_rotates_between_keys_even_if_one_always_needs_more() {
    let (mut m, visits) = ens11_layout_manager();
    m.deferred_layout.extend([K_A, K_B]);
    for _ in 0..8 {
        assert!(m.retry_deferred_layout());
    }
    assert_eq!(
        *visits.borrow(),
        vec![K_A, K_B, K_A, K_B, K_A, K_B, K_A, K_B]
    );
}
#[test]
fn ens11_deferred_layout_is_retained_until_idle_and_pruned_on_retirement() {
    let (mut m, visits) = ens11_layout_manager();
    m.deferred_layout.extend([K_A, K_B]);
    assert!(m.has_deferred_layout());
    m.remove(K_A);
    assert!(m.retry_deferred_layout());
    assert_eq!(*visits.borrow(), vec![K_B]);
    m.remove(K_B);
    assert!(!m.has_deferred_layout());
}
#[test]
fn ens11_split_receipt_is_available_before_any_optional_layout_continuation() {
    let (mut m, visits) = ens11_layout_manager();
    let port = InvalidatePort::new(InvalidateRegs::from_usermode_base(0xBB_0000).unwrap());
    m.queue_split(None, Some(K_A), 800, port.trigger());
    m.start_queued_splits(port.trigger());
    m.on_walk_ready(port.trigger());
    assert_eq!(m.take_splits(), vec![(800, Ok(()))]);
    assert!(visits.borrow().is_empty());
    assert!(m.has_deferred_layout());
    assert!(m.retry_deferred_layout());
    assert_eq!(*visits.borrow(), vec![K_A]);
}
#[test]
fn ens11_reset_forgets_optional_layout_from_the_old_guest_life() {
    let (mut m, _) = ens11_layout_manager();
    m.deferred_layout.extend([K_A, K_B]);
    m.abandon_life();
    assert!(m.deferred_layout.is_empty());
}

// Seeded schedules drive the real manager and diff/shadow walker, not an alternate scheduler.
fn ens11_check_split_step(r: &mut Rig, issued: &mut BTreeMap<u64, usize>) {
    let submitted = r.m.walker().submits.len();
    let eligible: BTreeSet<_> =
        r.m.inflight
            .as_ref()
            .map(|batch| {
                batch
                    .wants
                    .iter()
                    .filter_map(|(w, _, _)| {
                        if let Want::Split { ticket, .. } = w {
                            Some(*ticket)
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
    r.m.on_walk_ready(r.port.trigger());
    for (ticket, result) in r.m.take_splits() {
        assert_eq!(result, Ok(()));
        assert!(
            eligible.contains(&ticket),
            "only the completed submission can release a ticket"
        );
        assert!(
            submitted > issued.remove(&ticket).unwrap(),
            "walk starts after request admission"
        );
    }
}
#[test]
fn ens11_seeded_memop_chains_preserve_post_request_walk_receipts_and_bounded_backlog() {
    for seed in 1..=256u64 {
        let mut r = rig();
        let mut rng = seed;
        let mut issued = BTreeMap::new();
        let mut ticket = 9000;
        for _ in 0..200 {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            match (rng >> 32) % 6 {
                0..=2 if issued.len() < 32 => {
                    let (key, pdb) = if rng & 1 == 0 {
                        (K_A, PDB_A)
                    } else {
                        (K_B, PDB_B)
                    };
                    issued.insert(ticket, r.m.walker().submits.len());
                    if rng & 2 == 0 {
                        r.m.queue_split(Some(pdb), None, ticket, r.port.trigger());
                    } else {
                        r.m.queue_split(None, Some(key), ticket, r.port.trigger());
                    }
                    ticket += 1;
                }
                3..=4 => r.m.start_queued_splits(r.port.trigger()),
                _ => ens11_check_split_step(&mut r, &mut issued),
            }
            assert!(r.m.pending() <= 32);
            assert!(issued.len() <= 32);
        }
        for _ in 0..64 {
            r.m.start_queued_splits(r.port.trigger());
            ens11_check_split_step(&mut r, &mut issued);
            if issued.is_empty() {
                break;
            }
        }
        assert!(
            issued.is_empty(),
            "fair submissions/polls must drain every seed {seed}"
        );
        assert!(!r.m.in_flight());
        assert_eq!(r.m.pending(), 0);
    }
}

#[test]
fn ens11_an_ordinary_walk_interrupted_by_a_split_also_retains_optional_layout() {
    let (mut m, visits) = ens11_layout_manager();
    let port = InvalidatePort::new(InvalidateRegs::from_usermode_base(0xBB_0000).unwrap());
    let req = guest_invalidate(&port, PDB_B, false);
    m.on_invalidate(req, port.trigger());
    m.on_walk_ready(port.trigger());
    assert!(visits.borrow().is_empty());
    assert!(m.has_deferred_layout());
    assert!(m.retry_deferred_layout());
    assert_eq!(*visits.borrow(), vec![K_B]);
}
#[test]
fn ens11_repeated_memops_in_a_do_not_starve_an_idle_continuation_for_b() {
    let (mut m, visits) = ens11_layout_manager();
    let port = InvalidatePort::new(InvalidateRegs::from_usermode_base(0xBB_0000).unwrap());
    for ticket in 810..818 {
        m.queue_split(None, Some(K_A), ticket, port.trigger());
        if ticket == 810 {
            m.queue_split(None, Some(K_B), 899, port.trigger());
        }
        m.start_queued_splits(port.trigger());
        m.on_walk_ready(port.trigger());
        assert!(!m.take_splits().is_empty());
        assert!(m.retry_deferred_layout());
    }
    assert!(visits.borrow().iter().filter(|k| **k == K_B).count() >= 3);
}

#[test]
fn ens11_empty_diff_split_does_not_succeed_after_a_readiness_refusal() {
    let (mut m, _) = ens11_layout_manager();
    m.table.target(K_A).unwrap().refuse_refresh.set(true);
    let port = InvalidatePort::new(InvalidateRegs::from_usermode_base(0xBB_0000).unwrap());
    m.queue_split(None, Some(K_A), 910, port.trigger());
    m.start_queued_splits(port.trigger());
    m.on_walk_ready(port.trigger());
    let done = m.take_splits();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].0, 910);
    assert!(done[0].1.is_err());
}
#[test]
fn ens11_one_targets_readiness_refusal_does_not_forge_another_targets_verdict() {
    let (mut m, _) = ens11_layout_manager();
    m.table.target(K_A).unwrap().refuse_refresh.set(true);
    let port = InvalidatePort::new(InvalidateRegs::from_usermode_base(0xBB_0000).unwrap());
    m.queue_split(None, Some(K_A), 911, port.trigger());
    m.queue_split(None, Some(K_B), 912, port.trigger());
    m.start_queued_splits(port.trigger());
    m.on_walk_ready(port.trigger());
    let done: BTreeMap<_, _> = m.take_splits().into_iter().collect();
    assert!(done[&911].is_err());
    assert_eq!(done[&912], Ok(()));
}
