// ENS6 refused-window lifetime and bounds (second mutation ledger).

#[test]
fn ens6_a_window_is_remembered_until_just_before_its_ttl() {
    let (mut t, m, gap) = ens6::gapped();
    m.refuse_when(|_, _, sparse| sparse);
    t.admit(&m, gap, SPARSE_GRAIN).unwrap();
    let verbs = m.verbs();
    t.tick = WINDOW_TTL - 1;
    assert_eq!(t.admit(&m, gap, SPARSE_GRAIN), Ok(false));
    assert_eq!(m.verbs(), verbs);
    assert_eq!(t.stats.window_hits, 1);
}

#[test]
fn ens6_an_expired_window_is_asked_again_and_remembered_again() {
    let (mut t, m, gap) = ens6::gapped();
    m.refuse_when(|_, _, sparse| sparse);
    t.admit(&m, gap, SPARSE_GRAIN).unwrap();
    let verbs = m.verbs();
    t.tick = WINDOW_TTL + t.backoff_until;
    assert_eq!(t.admit(&m, gap, SPARSE_GRAIN), Ok(false));
    assert!(m.verbs() > verbs, "RM was asked once more");
    assert_eq!(t.stats.window_hits, 0);
    assert_eq!(t.refused[&gap].1, t.tick, "re-recorded at the new tick");
}

#[test]
fn ens6_window_eviction_drops_the_oldest_first() {
    let (mut t, _, gap) = ens6::gapped();
    for i in 0..100u64 {
        let lo = gap + i * 4 * SPARSE_GRAIN;
        t.remember_refused(lo, lo + SPARSE_GRAIN);
        assert!(t.refused.len() <= WINDOWS, "bounded after every insert");
    }
    assert!(!t.window_hit(gap), "the oldest window was evicted");
    assert!(t.window_hit(gap + 99 * 4 * SPARSE_GRAIN), "the newest is kept");
    assert!(t.window_hit(gap + 36 * 4 * SPARSE_GRAIN), "the 64th newest is kept");
    assert!(!t.window_hit(gap + 35 * 4 * SPARSE_GRAIN), "the 65th newest is gone");
}
