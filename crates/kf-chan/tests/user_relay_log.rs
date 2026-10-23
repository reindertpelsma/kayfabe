//! The relay observability lines (`docs/design/V3_RELAY_LIFECYCLE_LOG.md`): format, once-only, and bounded under a
//! flood of synthetic relays. GPU-free; the pump only prints what `kf_chan::user_log` returns.

use kf_chan::user_lifecycle::{Bound, Lifecycle, LifecycleStats, Mode, Notice, Obs, Plan};
use kf_chan::user_log::{At, RelayIdent, bound_line, lifecycle_line, scan_long_line};
use kf_chip::classes::Kind;

const SW: u8 = 1 << 6;

fn obs(abs: u64, epoch: u64, sw_mask: u8) -> Obs {
    Obs {
        raw: 0x1000 + abs,
        abs,
        epoch,
        put_abs: abs + 1,
        put_full: false,
        sw_mask,
        have_software: true,
        quiescent: true,
    }
}

/// Drive `scan` entries in Scan, then bind (no live registration), then `after` Pointer entries.
fn run(scan: u64, bind: bool, after: u64) -> Lifecycle {
    let mut l = Lifecycle::new();
    for i in 0..scan {
        assert!(matches!(l.entry(&obs(i, 0, 0), || 0), Plan::Scan { .. }));
        l.forwarded(i, 0x1000 + i);
    }
    if bind {
        assert_eq!(l.entry(&obs(scan, 0, SW), || 0), Plan::Forward);
        l.forwarded(scan, 0x1000 + scan);
        for i in 1..=after {
            assert_eq!(l.entry(&obs(scan + i, 0, SW), || 0), Plan::Forward);
            l.forwarded(scan + i, 0x1000 + scan + i);
        }
    }
    l
}

fn ident() -> RelayIdent {
    let i = RelayIdent::new(Some(0x3b8), Some(1));
    i.note_class(Kind::ThreeD);
    i.note_class(Kind::Compute);
    i
}

fn fields(line: &str) -> std::collections::HashMap<&str, &str> {
    line.split(' ').filter_map(|w| w.split_once('=')).collect()
}

#[test]
fn ident_fields_name_pid_classes_and_engine() {
    assert_eq!(
        ident().fields(),
        "pid=0x3b8 classes=Compute+ThreeD engine=0x1"
    );
    assert_eq!(
        RelayIdent::new(None, None).fields(),
        "pid=none classes=none engine=none"
    );
}

#[test]
fn ident_class_set_is_idempotent_and_bounded_by_the_kinds() {
    let i = RelayIdent::new(Some(1), Some(0));
    for _ in 0..1_000_000 {
        i.note_class(Kind::DmaCopy);
    }
    assert_eq!(i.fields(), "pid=0x1 classes=DmaCopy engine=0x0");
    for k in Kind::ALL {
        i.note_class(k);
    }
    let f = i.fields();
    let classes = f.split(' ').nth(1).unwrap();
    assert_eq!(classes.split('+').count(), Kind::ALL.len(), "{f}");
    assert!(f.len() < 200);
}

#[test]
fn a_never_bound_relay_says_so_with_its_scan_entries() {
    let l = run(5, false, 0);
    let line = lifecycle_line((0xc1d0_00bf, 5), At::Retire, &l, &ident()).unwrap();
    assert!(
        line.starts_with("kf3: USER_RING_LIFECYCLE (c1d000bf, 5) mode=Scan Counts {"),
        "the pre-existing prefix is kept: {line}"
    );
    let tail = line.rsplit_once('}').unwrap().1;
    let f = fields(tail);
    assert_eq!(f["bound"], "never", "{line}");
    assert_eq!(f["scan_entries"], "5");
    assert_eq!(f["at"], "retire");
    assert_eq!(f["pid"], "0x3b8");
    assert_eq!(f["classes"], "Compute+ThreeD");
    assert_eq!(f["engine"], "0x1");
    assert!(!line.contains('\n'));
}

#[test]
fn a_bound_relay_reports_the_scan_entries_before_the_bind_not_its_later_entries() {
    let l = run(3, true, 1000);
    let line = lifecycle_line((1, 2), At::Exit, &l, &ident()).unwrap();
    assert!(line.contains(" mode=Pointer Counts {"), "{line}");
    let f = fields(line.rsplit_once('}').unwrap().1);
    assert_eq!(
        (f["bound"], f["scan_entries"], f["at"]),
        ("yes", "3", "exit")
    );
    assert_eq!(
        l.bound(),
        Some(Bound {
            to: Mode::Pointer,
            scan_entries: 3
        })
    );
}

#[test]
fn a_relay_that_forwarded_nothing_prints_no_final_line() {
    let l = Lifecycle::new();
    assert!(lifecycle_line((1, 2), At::Retire, &l, &ident()).is_none());
    assert!(lifecycle_line((1, 2), At::Exit, &l, &ident()).is_none());
}

#[test]
fn the_bind_is_noticed_exactly_once_per_relay() {
    let mut l = run(3, true, 50);
    let bound: Vec<Notice> = l
        .take_notices()
        .into_iter()
        .filter(|n| matches!(n, Notice::Bound { .. }))
        .collect();
    assert_eq!(
        bound,
        vec![Notice::Bound {
            to: Mode::Pointer,
            scan_entries: 3
        }]
    );
    // a later epoch change with a live registration (Pointer -> Window), and more entries: no second bind
    let p = l.entry(&obs(100, 1, SW), || 1);
    assert!(matches!(p, Plan::Scan { .. }));
    for i in 101..200 {
        l.entry(&obs(i, 1, SW), || 1);
        l.forwarded(i, 0);
    }
    assert!(
        l.take_notices()
            .iter()
            .all(|n| !matches!(n, Notice::Bound { .. }))
    );
}

#[test]
fn a_bind_with_a_live_registration_goes_to_window() {
    let mut l = Lifecycle::new();
    l.forwarded(0, 1);
    l.forwarded(1, 1);
    l.entry(&obs(2, 3, SW), || 2);
    let n: Vec<Notice> = l.take_notices();
    assert!(
        n.contains(&Notice::Bound {
            to: Mode::Window,
            scan_entries: 2
        }),
        "{n:?}"
    );
}

#[test]
fn a_relay_that_never_binds_queues_no_bind_notice() {
    let mut l = run(10_000, false, 0);
    assert!(
        l.take_notices()
            .iter()
            .all(|n| !matches!(n, Notice::Bound { .. }))
    );
    assert_eq!(l.bound(), None);
}

#[test]
fn the_bound_line_is_one_key_value_line() {
    let line = bound_line(
        (0xc1d0_00bf, 5),
        &Bound {
            to: Mode::Pointer,
            scan_entries: 3,
        },
        &ident(),
    );
    assert_eq!(
        line,
        "kf3: USER_RING_BOUND (c1d000bf, 5) to=Pointer scan_entries=3 pid=0x3b8 \
         classes=Compute+ThreeD engine=0x1"
    );
}

#[test]
fn the_first_switch_is_handed_out_once() {
    let mut l = run(1, true, 3);
    assert!(!l.take_first_switch(), "bound but not switched");
    l.entry(&obs(10, 1, SW), || 1); // Pointer -> Window (fresh)
    assert!(l.counts().to_window_fresh >= 1);
    assert!(l.take_first_switch());
    assert!(!l.take_first_switch(), "once per relay");
}

fn kv(summary: &str) -> std::collections::HashMap<String, u64> {
    summary
        .split(' ')
        .filter_map(|w| w.split_once('='))
        .filter_map(|(k, v)| Some((k.to_owned(), v.parse().ok()?)))
        .collect()
}

#[test]
fn the_summary_carries_the_relay_totals_and_the_entries_by_mode() {
    let st = LifecycleStats::default();
    for _ in 0..5 {
        st.relay_born();
    }
    st.relay_bound();
    st.relay_bound();
    st.relay_switching();
    st.relay_final(&run(7, false, 0));
    st.relay_final(&run(2, true, 9));
    st.exit_reported();
    st.exit_busy();
    let s = st.summary();
    assert!(s.starts_with("entries_scan=0 "), "old keys first: {s}");
    let m = kv(&s);
    assert_eq!(m["relays_born"], 5);
    assert_eq!(m["relays_bound"], 2);
    assert_eq!(m["relays_never_bound"], 3);
    assert_eq!(m["relays_switching"], 1);
    assert_eq!(m["relays_never_bound_with_entries"], 1);
    assert_eq!(m["entries_scan_never_bound"], 7);
    assert_eq!(m["relays_exit_reported"], 1);
    assert_eq!(m["relays_exit_busy"], 1);
    for k in ["entries_scan", "entries_pointer", "entries_window"] {
        assert!(m.contains_key(k), "{k}");
    }
    assert!(!s.contains('\n'));
}

/// The flood: the number of lines depends on the number of relays and their events, never on the entries.
#[test]
fn a_flood_of_relays_and_entries_prints_a_bounded_number_of_lines() {
    fn lines(relays: u64, entries: u64) -> (u64, usize) {
        let st = LifecycleStats::default();
        let (mut n, mut longest) = (0u64, 0usize);
        for r in 0..relays {
            st.relay_born();
            // half of the relays bind after 4 scan entries, the other half never do
            let l = run(
                if r % 2 == 0 { 4 } else { entries },
                r % 2 == 0,
                if r % 2 == 0 { entries } else { 0 },
            );
            let mut l = l;
            for notice in l.take_notices() {
                if let Notice::Bound { to, scan_entries } = notice {
                    st.relay_bound();
                    let line = bound_line((r as u32, 1), &Bound { to, scan_entries }, &ident());
                    n += 1;
                    longest = longest.max(line.len());
                }
            }
            if let Some(line) = lifecycle_line((r as u32, 1), At::Retire, &l, &ident()) {
                st.relay_final(&l);
                n += 1;
                longest = longest.max(line.len());
            }
        }
        let s = st.summary();
        longest = longest.max(s.len());
        assert_eq!(kv(&s)["relays_born"], relays);
        (n, longest)
    }
    let small = lines(2_000, 20);
    let large = lines(2_000, 20_000);
    assert_eq!(
        small.0, large.0,
        "line count is independent of the entries a relay forwards"
    );
    // one BOUND per bound relay + one final line per relay
    assert_eq!(large.0, 1_000 + 2_000);
    assert!(large.1 < 2_048, "every line is bounded: {} bytes", large.1);
}

#[test]
fn the_exit_pass_runs_once_per_device() {
    let st = LifecycleStats::default();
    assert!(st.begin_exit());
    assert!(!st.begin_exit());
    assert!(!st.begin_exit());
}

fn scan_long_notices(l: &mut Lifecycle) -> Vec<(u64, u64)> {
    l.take_notices()
        .into_iter()
        .filter_map(|n| match n {
            Notice::ScanLong {
                threshold,
                scan_entries,
            } => Some((threshold, scan_entries)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_scan_long_line_is_one_key_value_line() {
    assert_eq!(
        scan_long_line((0xc1d0_00bf, 5), 100_000, 100_000, &ident()),
        "kf3: USER_RING_SCAN_LONG (c1d000bf, 5) threshold=100000 scan_entries=100000 pid=0x3b8 \
         classes=Compute+ThreeD engine=0x1"
    );
}

#[test]
fn a_relay_still_in_scan_says_so_once_at_each_of_three_thresholds() {
    let mut l = Lifecycle::new();
    let mut seen = Vec::new();
    for i in 0..10_500_000u64 {
        l.forwarded(i, 0x1000);
        if i % 1_000_000 == 0 || i == 99_999 || i == 9_999_999 {
            seen.extend(scan_long_notices(&mut l));
        }
    }
    seen.extend(scan_long_notices(&mut l));
    assert_eq!(
        seen,
        vec![
            (100_000, 100_000),
            (1_000_000, 1_000_000),
            (10_000_000, 10_000_000)
        ],
        "one notice per threshold, none per entry"
    );
}

#[test]
fn a_relay_that_binds_before_a_threshold_never_says_scan_long() {
    let mut l = run(50, true, 300_000);
    for i in 0..300_000u64 {
        l.forwarded(1_000_000 + i, 0);
    }
    assert!(scan_long_notices(&mut l).is_empty());
}

/// The coordinator's flood: 2,000 relays at 20 and at 20,000 entries print an exact, equal number of lines, and the
/// relays that do cross the thresholds print exactly their thresholds.
#[test]
fn the_scan_long_flood_prints_an_exact_bounded_number_of_lines() {
    fn lines(relays: u64, entries: u64) -> u64 {
        let mut n = 0u64;
        for r in 0..relays {
            let mut l = Lifecycle::new();
            for i in 0..entries {
                l.forwarded(i, 0);
            }
            for (t, e) in scan_long_notices(&mut l) {
                let line = scan_long_line((r as u32, 1), t, e, &ident());
                assert!(!line.is_empty() && line.len() < 512 && !line.contains('\n'));
                n += 1;
            }
        }
        n
    }
    assert_eq!(lines(2_000, 20), 0);
    assert_eq!(
        lines(2_000, 20_000),
        0,
        "below the first threshold: nothing, whatever the count"
    );
    assert_eq!(
        lines(100, 1_000_000),
        200,
        "two thresholds per relay, exactly"
    );
    assert_eq!(
        lines(50, 1_000_001),
        100,
        "the entry after a threshold adds nothing"
    );
}
