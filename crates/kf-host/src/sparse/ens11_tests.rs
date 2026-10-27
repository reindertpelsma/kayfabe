// Split-critical layout deferral uses the same real reservations model as ENS6.
#[test]
fn ens11_split_critical_sparse_admission_defers_rm_and_resumes_intent() {
    let (mut t, m, base) = ens6::roomy(2000);
    let budget = slice::Slice::default();
    {
        let _critical = slice::critical_scope(true);
        ens6::refresh(&mut t, &m, &budget, |t, ops| {
            assert_eq!(t.admit(ops, base, 4 * SPARSE_GRAIN), Ok(false));
        });
        assert_eq!(m.verbs(), 0, "no optional geometry before split release");
    }
    for _ in 0..10 {
        ens6::refresh(&mut t, &m, &budget, |t, ops| t.retry(ops).unwrap());
        if m.sparse_tiles().len() == 4 {
            break;
        }
    }
    assert_eq!(m.sparse_tiles().len(), 4, "deferred intent is not lost");
}
#[test]
fn ens11_plain_mapping_in_a_split_keeps_coarse_geometry_without_rm_refinement() {
    let (mut t, m, base) = ens6::roomy(2000);
    let budget = slice::Slice::default();
    let _critical = slice::critical_scope(true);
    ens6::refresh(&mut t, &m, &budget, |t, ops| {
        assert_eq!(
            t.prepare_map(ops, 99, base + 7 * NORMAL_GRAIN, PAGE, true)
                .unwrap()
                .len(),
            1
        );
    });
    assert_eq!(
        m.verbs(),
        0,
        "the backed map can use its existing coarse reservation"
    );
    assert_eq!(t.blocks.len(), 1);
}
#[test]
fn ens11_split_critical_retired_sparse_geometry_stays_covered_until_background_retry() {
    let (mut t, m, base) = ens6::roomy(2000);
    assert_eq!(t.admit(&m, base, SPARSE_GRAIN), Ok(true));
    let budget = slice::Slice::default();
    let before = m.verbs();
    {
        let _critical = slice::critical_scope(true);
        t.retire(&m, base, SPARSE_GRAIN, false).unwrap();
        ens6::refresh(&mut t, &m, &budget, |t, ops| t.retry(ops).unwrap());
        assert_eq!(
            m.verbs(),
            before,
            "no cleanup churn on the split's critical path"
        );
        assert_eq!(m.sparse_tiles(), vec![base]);
    }
    ens6::refresh(&mut t, &m, &budget, |t, ops| t.retry(ops).unwrap());
    assert!(m.sparse_tiles().is_empty());
}
#[test]
fn ens11_new_split_interrupts_background_layout_without_losing_sparse_intent() {
    use std::sync::atomic::Ordering::Relaxed;
    let (mut t, m, base) = ens6::roomy(2000);
    let budget = slice::Slice::default();
    let stop = budget.interrupt_handle();
    stop.store(true, Relaxed);
    ens6::refresh(&mut t, &m, &budget, |t, ops| {
        assert_eq!(t.admit(ops, base, SPARSE_GRAIN), Ok(false))
    });
    assert_eq!(m.verbs(), 0);
    stop.store(false, Relaxed);
    ens6::refresh(&mut t, &m, &budget, |t, ops| t.retry(ops).unwrap());
    assert_eq!(m.sparse_tiles(), vec![base]);
}
#[test]
fn ens11_split_interrupt_during_a_verb_stops_children_and_repairs_the_owner() {
    use std::sync::atomic::Ordering::Relaxed;
    let (mut t, m, base) = ens6::roomy(2000);
    let budget = slice::Slice::default();
    let stop = budget.interrupt_handle();
    let arm = stop.clone();
    m.refuse_when(move |_, _, _| {
        arm.store(true, Relaxed);
        false
    });
    ens6::refresh(&mut t, &m, &budget, |t, ops| {
        assert_eq!(
            t.admit(ops, base + 7 * NORMAL_GRAIN, SPARSE_GRAIN),
            Ok(false)
        );
    });
    assert!(m.sparse_tiles().is_empty());
    assert_eq!(m.blocks.borrow().len(), 1);
    assert!(
        m.verbs() <= 4,
        "one child then mandatory repair, not more layout"
    );
    assert!(!t.poisoned);
}
#[test]
fn ens11_nested_noncritical_scope_cannot_reenable_layout_inside_a_split() {
    assert!(!slice::in_split());
    {
        let _outer = slice::critical_scope(true);
        assert!(slice::in_split());
        {
            let _inner = slice::critical_scope(false);
            assert!(slice::in_split());
        }
        assert!(slice::in_split());
    }
    assert!(!slice::in_split());
}

#[test]
fn ens11_empty_namespace_cleanup_finishes_after_the_layout_slice_is_spent() {
    let (mut t, m, base) = ens6::roomy(2000);
    t.admit(&m, base, SPARSE_GRAIN).unwrap();
    let budget = slice::Slice::default();
    ens6::refresh(&mut t, &m, &budget, |t, ops| {
        budget.spend(slice::SLICE_NS);
        assert_eq!(t.clear(ops), Ok(()));
    });
    assert!(m.sparse_tiles().is_empty());
}
#[test]
fn ens11_empty_namespace_cleanup_does_not_depend_on_optional_allowance() {
    let (mut t, m, base) = ens6::roomy(2000);
    t.admit(&m, base + SPARSE_GRAIN, 2 * SPARSE_GRAIN).unwrap();
    let budget = slice::Slice::default();
    ens6::refresh(&mut t, &m, &budget, |t, ops| {
        budget.spend(slice::SLICE_NS * 2);
        assert_eq!(t.clear(ops), Ok(()));
    });
    assert!(m.sparse_tiles().is_empty());
}
