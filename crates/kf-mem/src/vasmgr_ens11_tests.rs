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
    key: VasKey,
    visits: Rc<RefCell<Vec<VasKey>>>,
}
impl MapTarget for Ens11LayoutTarget {
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
