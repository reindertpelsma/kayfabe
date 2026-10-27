// ENS10: slow calls and admission rollback must not start an unbounded cleanup tail.
fn ens10_refusal_tail(count: u64, refuse_at: u64) {
    let (mut t, m, gap) = ens6::gapped();
    let slice = slice::Slice::default();
    let seen = std::rc::Rc::new(Cell::new(0));
    let refused = seen.clone();
    m.refuse_when(move |_, _, sparse| {
        if sparse {
            refused.set(refused.get() + 1);
        }
        sparse && refused.get() == refuse_at
    });
    // Exhaust the remaining slice in the refused verb, after several successful tiles.
    let clock = || m.now();
    struct SlowFailure<'a>(&'a ens6::Metered);
    impl Ops for SlowFailure<'_> {
        fn reserve(&self, lo: u64, len: u64, sparse: bool) -> Result<u32, RmError> {
            let out = self.0.reserve(lo, len, sparse);
            if out.is_err() {
                self.0.clock.set(self.0.now() + slice::SLICE_NS);
            }
            out
        }
        fn release(&self, handle: u32) -> Result<(), RmError> {
            self.0.release(handle)
        }
    }
    let slow = SlowFailure(&m);
    let ops = slice::Sliced {
        inner: &slow,
        slice: &slice,
        clock: &clock,
    };
    assert_eq!(t.admit(&ops, gap, count * SPARSE_GRAIN), Ok(false));
    assert!(
        m.now() <= slice::SLICE_NS + count * ens6::VERB_NS,
        "cleanup exceeded exhausted budget: {}",
        m.now()
    );
    assert_eq!(
        m.releases.get(),
        0,
        "no optional conversion after the refusal"
    );
    assert_eq!(m.sparse_tiles().len() as u64, refuse_at - 1);
    assert_eq!(
        t.cleanup.len() as u64,
        refuse_at - 1,
        "deferred cleanup is retained"
    );
    assert!(t.blocks.values().all(|b| b.wanted.iter().all(|&v| v == 0)));
    t.tick = t.backoff_until;
    ens6::refresh(&mut t, &m, &slice, |t, ops| t.retry(ops).unwrap());
    assert!(m.sparse_tiles().is_empty(), "cleanup resumes after backoff");
    assert!(!t.poisoned);
}
#[test]
fn ens10_failed_eight_tile_admission_defers_cleanup_after_slice() {
    ens10_refusal_tail(8, 8);
}
#[test]
fn ens10_failed_twenty_tile_admission_keeps_bounded_cleanup_and_resumes() {
    ens10_refusal_tail(20, 16);
}

fn ens10_slow_replace(offset: u64) {
    let (mut t, m, base) = ens6::roomy(1000);
    let slice = slice::Slice::default();
    m.verb_ns.set(120_000_000);
    ens6::refresh(&mut t, &m, &slice, |t, ops| {
        assert_eq!(
            t.admit(ops, base + offset * NORMAL_GRAIN, SPARSE_GRAIN),
            Ok(false)
        );
    });
    assert_eq!(
        m.verbs(),
        2,
        "slow release must restore, not start more children"
    );
    assert_eq!(
        m.now(),
        240_000_000,
        "one slow release plus mandatory restore"
    );
    assert_eq!(t.backoff_step, 0, "a slice deferral is not an RM refusal");
    assert_eq!(t.stats.rm_declined, 0);
    assert_eq!(m.blocks.borrow().len(), 1);
    assert!(!t.poisoned);
}
#[test]
fn ens10_slow_release_defers_first_child_and_restores_original() {
    ens10_slow_replace(0);
}
#[test]
fn ens10_slow_release_does_not_start_three_child_replacement() {
    ens10_slow_replace(7);
}
