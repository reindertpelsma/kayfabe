use super::*;
fn rows_handoff_fixture() -> (
    PlacedRows,
    RowsLog,
    std::sync::Arc<crate::ptlock::HandoffHub>,
    std::sync::Arc<std::sync::atomic::AtomicU64>,
) {
    let rows = PlacedRows::default();
    let clone = rows.clone();
    let count = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let out = count.clone();
    let hub = std::sync::Arc::new(crate::ptlock::HandoffHub::new(move |_| {
        assert!(
            clone.try_write().is_ok(),
            "wake must follow real row-lock release"
        );
        out.fetch_add(1, Ordering::Relaxed);
    }));
    let log = RowsLog::default();
    log.bind_handoff(hub.clone());
    (rows, log, hub, count)
}
#[test]
fn ens11_row_scope_release_wakes_before_the_next_host_operation() {
    let (rows, log, hub, count) = rows_handoff_fixture();
    {
        let _g = write_rows_releasing(&rows, &log).unwrap();
        let seen = hub.prepare();
        assert!(!hub.lost(7, seen));
        assert_eq!(count.load(Ordering::Relaxed), 0);
    }
    assert_eq!(
        count.load(Ordering::Relaxed),
        1,
        "not at VA-loop end or a later RM return"
    );
}
#[test]
fn ens11_row_scope_error_return_still_releases_the_registered_loser() {
    let (rows, log, hub, count) = rows_handoff_fixture();
    fn fail_inside_scope(
        rows: &PlacedRows,
        log: &RowsLog,
        hub: &crate::ptlock::HandoffHub,
    ) -> Result<(), ()> {
        let _g = write_rows_releasing(rows, log).unwrap();
        let seen = hub.prepare();
        assert!(!hub.lost(8, seen));
        Err(())
    }
    let result = fail_inside_scope(&rows, &log, &hub);
    assert!(result.is_err());
    assert_eq!(count.load(Ordering::Relaxed), 1);
}
