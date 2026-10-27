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
