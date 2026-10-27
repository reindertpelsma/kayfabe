//! CPU simulations of the production IRQSTORM scheduler and real atomic interrupt tree.
use super::*;
use crate::reset::Gate;
use kf_trap::cpuintr::{CpuIntr, Raise, Reg};
use std::sync::atomic::AtomicUsize;
const MS: u64 = 1_000_000;
fn storm(ms: u16) -> Storm {
    Storm::new(Config { period_ms: ms })
}
fn tree(vectors: &[u32]) -> CpuIntr {
    let t = CpuIntr::new(kf_chip::Family::Ampere, 0xbb0000).unwrap();
    t.write(Reg::TopEnSet, 0xff);
    for &v in vectors {
        t.write(Reg::LeafEnSet((v / 32) as usize), 1 << (v % 32));
    }
    t
}
fn image(t: &CpuIntr) -> Vec<(u64, u32)> {
    let mut out = Vec::new();
    t.shadow_all(|a, b| out.push((a, b)));
    out
}
#[test]
fn unset_and_zero_are_off_even_with_a_mode() {
    for p in [None, Some("0"), Some(" 0 ")] {
        for m in [None, Some("all"), Some("owed")] {
            assert_eq!(Config::parse(p, m), Ok(None));
        }
    }
}
#[test]
fn valid_host_periods_select_all_and_keep_the_exact_period() {
    for ms in [1u16, 7, 1000] {
        assert_eq!(
            Config::parse(Some(&ms.to_string()), None),
            Ok(Some(Config { period_ms: ms }))
        );
    }
}
#[test]
fn explicit_all_accepts_a_valid_interval_without_becoming_guest_controlled() {
    assert_eq!(
        Config::parse(Some("10"), Some("all")),
        Ok(Some(Config { period_ms: 10 }))
    );
}
#[test]
fn out_of_range_or_malformed_periods_are_rejected() {
    for p in ["-1", "1001", "18446744073709551615", "", "junk"] {
        assert_eq!(Config::parse(Some(p), None), Err(Invalid::Period));
    }
}
#[test]
fn unknown_modes_are_rejected() {
    assert_eq!(Config::parse(Some("1"), Some("ALL")), Err(Invalid::Mode));
    assert_eq!(Config::parse(Some("1"), Some("junk")), Err(Invalid::Mode));
}
#[test]
fn owed_never_silently_falls_back_to_all() {
    assert_eq!(
        Config::parse(Some("1"), Some("owed")),
        Err(Invalid::OwedUnavailable)
    );
}
#[test]
fn only_one_shared_transport_edge_is_sent_for_all_enabled_completion_vectors() {
    let s = storm(1);
    let t = tree(&[0, 1, 2]);
    let g = Gate::default();
    let calls = AtomicUsize::new(0);
    s.tick(
        || 0,
        &g,
        &t,
        [0, 0, 1, 2],
        || {
            calls.fetch_add(1, Ordering::Relaxed);
            true
        },
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(s.raises.load(Ordering::Relaxed), 1);
    assert_eq!(
        s.summary(),
        " irq_storm[mode=all period_ms=1 storm_raises=1 storm_skipped_masked=0]"
    );
}
#[test]
fn nsi_vectors_outside_v0_v1_v2_use_the_same_existing_transport() {
    let s = storm(7);
    let t = tree(&[5, 70]);
    let calls = AtomicUsize::new(0);
    s.tick(
        || 0,
        &Gate::default(),
        &t,
        [5, 70],
        || {
            calls.fetch_add(1, Ordering::Relaxed);
            true
        },
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        s.summary(),
        " irq_storm[mode=all period_ms=7 storm_raises=1 storm_skipped_masked=0]"
    );
}
#[test]
fn before_leaf_arming_no_message_is_emitted() {
    let s = storm(1);
    let t = tree(&[]);
    s.tick(|| 0, &Gate::default(), &t, [0, 1], || panic!("unarmed"));
    assert_eq!(s.raises.load(Ordering::Relaxed), 0);
    assert_eq!(s.skipped_masked.load(Ordering::Relaxed), 2);
}
#[test]
fn before_top_arming_no_message_is_emitted() {
    let s = storm(1);
    let t = tree(&[0, 70]);
    t.write(Reg::TopEnClear, 0xff);
    s.tick(
        || 0,
        &Gate::default(),
        &t,
        [0, 70],
        || panic!("top disabled"),
    );
    assert_eq!(s.skipped_masked.load(Ordering::Relaxed), 2);
}
#[test]
fn leaf_masks_skip_distinct_vectors_and_never_get_changed_by_the_storm() {
    let s = storm(1);
    let t = tree(&[0, 1]);
    t.write(Reg::LeafEnClear(0), 3);
    let before = image(&t);
    s.tick(|| 0, &Gate::default(), &t, [0, 0, 1], || panic!("masked"));
    assert_eq!(image(&t), before);
    assert_eq!(
        s.summary(),
        " irq_storm[mode=all period_ms=1 storm_raises=0 storm_skipped_masked=2]"
    );
}
#[test]
fn top_masks_skip_vectors_even_with_a_real_pending_leaf() {
    let s = storm(1);
    let t = tree(&[70]);
    t.latch(70);
    t.write(Reg::TopEnClear, 2);
    let before = image(&t);
    s.tick(|| 0, &Gate::default(), &t, [70], || panic!("top masked"));
    assert_eq!(image(&t), before);
    assert_eq!(s.skipped_masked.load(Ordering::Relaxed), 1);
}
#[test]
fn an_empty_enabled_tree_gets_only_a_spurious_message_and_no_status() {
    let s = storm(1);
    let t = tree(&[3]);
    let before = image(&t);
    s.tick(|| 0, &Gate::default(), &t, [3], || true);
    assert_eq!(s.raises.load(Ordering::Relaxed), 1);
    assert_eq!(image(&t), before);
    assert_eq!(t.read(Reg::Top), 0);
}
#[test]
fn guest_w1c_is_never_undone_and_unrelated_real_status_is_preserved() {
    let s = storm(1);
    let t = tree(&[3, 70]);
    t.latch(3);
    t.latch(70);
    t.write(Reg::Leaf(0), 1 << 3);
    let before = image(&t);
    s.tick(|| 0, &Gate::default(), &t, [3], || true);
    assert_eq!(s.raises.load(Ordering::Relaxed), 1);
    assert_eq!(image(&t), before);
    assert_eq!(t.read(Reg::Leaf(0)), 0);
    assert_eq!(t.read(Reg::Leaf(2)), 1 << (70 % 32));
}
fn cadence(ms: u16) {
    let s = storm(ms);
    let t = tree(&[0]);
    let g = Gate::default();
    let calls = AtomicUsize::new(0);
    for now in [
        0,
        0,
        u64::from(ms) * MS - 1,
        u64::from(ms) * MS,
        100 * u64::from(ms) * MS,
    ] {
        s.tick(
            || now,
            &g,
            &t,
            [0],
            || {
                calls.fetch_add(1, Ordering::Relaxed);
                true
            },
        );
    }
    assert_eq!(
        calls.load(Ordering::Relaxed),
        3,
        "no early sends or catch-up burst"
    );
}
#[test]
fn one_ms_period_cannot_spin_on_one_timestamp() {
    cadence(1);
}
#[test]
fn a_longer_period_is_not_shortened_and_a_delayed_tick_does_not_catch_up() {
    cadence(17);
}
fn ceiling(vectors: Vec<u32>) {
    let s = storm(1);
    let t = tree(&vectors);
    let g = Gate::default();
    let calls = AtomicUsize::new(0);
    for us in 0..1_000_000 {
        s.tick(
            || us * 1000,
            &g,
            &t,
            vectors.iter().copied(),
            || {
                calls.fetch_add(1, Ordering::Relaxed);
                true
            },
        );
    }
    assert_eq!(calls.load(Ordering::Relaxed), 1000);
    assert_eq!(s.raises.load(Ordering::Relaxed), 1000);
}
#[test]
fn three_vectors_do_not_turn_one_khz_into_three_khz() {
    ceiling(vec![0, 1, 2]);
}
#[test]
fn many_nsi_vectors_and_aliases_share_one_thousand_messages_per_second() {
    ceiling((0..32).chain(0..32).collect());
}
#[test]
fn reset_gate_blocks_messages_and_reopening_does_not_invent_status() {
    let s = storm(1);
    let t = tree(&[0]);
    let g = Gate::default();
    g.close();
    s.tick(|| 0, &g, &t, [0], || panic!("reset"));
    assert_eq!(s.raises.load(Ordering::Relaxed), 0);
    g.open();
    s.tick(|| 0, &g, &t, [0], || true);
    assert_eq!(s.raises.load(Ordering::Relaxed), 1);
    assert_eq!(t.read(Reg::Top), 0);
}
#[test]
fn tree_reset_requires_new_guest_enables_before_renotification() {
    let s = storm(1);
    let t = tree(&[70]);
    let g = Gate::default();
    t.latch(70);
    t.reset();
    s.tick(|| 0, &g, &t, [70], || panic!("new life unarmed"));
    assert_eq!(s.skipped_masked.load(Ordering::Relaxed), 1);
    t.write(Reg::TopEnSet, 2);
    t.write(Reg::LeafEnSet(2), 1 << (70 % 32));
    s.tick(|| MS, &g, &t, [70], || true);
    assert_eq!(s.raises.load(Ordering::Relaxed), 1);
    assert_eq!(t.read(Reg::Leaf(2)), 0);
}
fn near_overflow(now: u64) {
    let s = storm(1);
    let t = tree(&[0]);
    let g = Gate::default();
    s.tick(|| 0, &g, &t, [0], || true);
    for _ in 0..100 {
        s.tick(|| now, &g, &t, [0], || panic!("clock overflow"));
    }
    assert_eq!(s.raises.load(Ordering::Relaxed), 1);
}
#[test]
fn saturated_clock_cannot_mint_unlimited_credits() {
    near_overflow(u64::MAX);
}
#[test]
fn a_deadline_past_the_clock_domain_fails_without_a_message() {
    near_overflow(u64::MAX - 1);
}
fn contention(v: u32) {
    let s = storm(1);
    let t = tree(&[v]);
    let g = Gate::default();
    let calls = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let (s, t, g, calls) = (&s, &t, &g, &calls);
            scope.spawn(move || {
                for ms in 0..1000 {
                    s.tick(
                        || ms * MS,
                        g,
                        t,
                        [v],
                        || {
                            calls.fetch_add(1, Ordering::Relaxed);
                            true
                        },
                    );
                }
            });
        }
    });
    let n = calls.load(Ordering::Relaxed);
    assert!(n > 0 && n <= 1000, "{n}");
    assert_eq!(s.raises.load(Ordering::Relaxed), n as u64);
}
#[test]
fn competing_ticks_for_gr_share_a_single_global_rate_limit() {
    contention(0);
}
#[test]
fn competing_ticks_for_ce_share_a_single_global_rate_limit() {
    contention(70);
}
fn blocked_sender(v: u32) {
    let s = storm(1);
    let t = tree(&[v]);
    let g = Gate::default();
    let (begun, wait) = std::sync::mpsc::channel();
    let (release, done) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let (s, t, g) = (&s, &t, &g);
        scope.spawn(move || {
            s.tick(
                || 0,
                g,
                t,
                [v],
                || {
                    assert!(!g.quiet(), "the sender holds reset quiescence");
                    begun.send(()).unwrap();
                    done.recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                    true
                },
            )
        });
        wait.recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        t.write(Reg::LeafEnClear((v / 32) as usize), 1 << (v % 32));
        t.write(Reg::Leaf((v / 32) as usize), 1 << (v % 32));
        assert_eq!(t.read(Reg::Leaf((v / 32) as usize)), 0);
        release.send(()).unwrap();
    });
    assert!(g.quiet());
}
#[test]
fn a_blocked_worker_sender_cannot_block_gr_trap_writes() {
    blocked_sender(0);
}
#[test]
fn a_blocked_worker_sender_cannot_block_ce_trap_writes() {
    blocked_sender(70);
}
#[test]
fn no_host_completion_consumption_history_is_claimed_by_the_pacer() {
    assert_eq!(
        Config::parse(Some("7"), Some("owed")),
        Err(Invalid::OwedUnavailable)
    );
}
#[test]
fn hostile_vector_ids_cannot_index_the_tree_or_steal_real_status() {
    let s = storm(1);
    let t = tree(&[0]);
    let before = image(&t);
    s.tick(|| 0, &Gate::default(), &t, [u32::MAX, 256, 512, 0], || true);
    assert_eq!(s.raises.load(Ordering::Relaxed), 1);
    assert_eq!(image(&t), before);
}

fn late_sender(v: u32) {
    let s = storm(1);
    let t = tree(&[v]);
    let g = Gate::default();
    let clock = AtomicU64::new(0);
    s.tick(
        || clock.load(Ordering::Relaxed),
        &g,
        &t,
        [v],
        || {
            clock.store(10 * MS, Ordering::Relaxed);
            true
        },
    );
    assert_eq!(s.raises.load(Ordering::Relaxed), 1);
    s.tick(|| clock.load(Ordering::Relaxed), &g, &t, [v], || true);
    assert_eq!(
        s.raises.load(Ordering::Relaxed),
        1,
        "cooldown follows actual send completion"
    );
    clock.store(11 * MS, Ordering::Relaxed);
    s.tick(|| clock.load(Ordering::Relaxed), &g, &t, [v], || true);
    assert_eq!(s.raises.load(Ordering::Relaxed), 2);
}
#[test]
fn a_delayed_gr_sender_does_not_create_a_catch_up_message() {
    late_sender(0);
}
#[test]
fn a_delayed_ce_sender_does_not_create_a_catch_up_message() {
    late_sender(70);
}
fn overlap(v: u32) {
    let s = storm(1);
    let t = tree(&[v]);
    let g = Gate::default();
    let clock = AtomicU64::new(0);
    let nested = AtomicUsize::new(0);
    s.tick(
        || clock.load(Ordering::Relaxed),
        &g,
        &t,
        [v],
        || {
            clock.store(10 * MS, Ordering::Relaxed);
            s.tick(
                || clock.load(Ordering::Relaxed),
                &g,
                &t,
                [v],
                || {
                    nested.fetch_add(1, Ordering::Relaxed);
                    true
                },
            );
            true
        },
    );
    assert_eq!(
        nested.load(Ordering::Relaxed),
        0,
        "a sender already owns the emission slot"
    );
    assert_eq!(s.raises.load(Ordering::Relaxed), 1);
}
#[test]
fn an_in_progress_gr_message_excludes_another_timer_sender() {
    overlap(0);
}
#[test]
fn an_in_progress_ce_message_excludes_another_timer_sender() {
    overlap(70);
}
fn failed_send(v: u32) {
    let s = storm(1);
    let t = tree(&[v]);
    let g = Gate::default();
    s.tick(|| 0, &g, &t, [v], || false);
    assert_eq!(s.raises.load(Ordering::Relaxed), 0);
    s.tick(|| 0, &g, &t, [v], || panic!("failed writes cannot spin"));
    s.tick(|| MS, &g, &t, [v], || true);
    assert_eq!(s.raises.load(Ordering::Relaxed), 1);
}
#[test]
fn failed_gr_eventfd_writes_are_not_counted_as_raises() {
    failed_send(0);
}
#[test]
fn failed_ce_eventfd_writes_are_not_counted_as_raises() {
    failed_send(70);
}
fn off_sequence(period: Option<&str>) {
    let s = Config::parse(period, Some("all")).unwrap().map(Storm::new);
    assert!(timer_interval(false, s.as_ref()).is_none());
    let t = tree(&[0]);
    let g = Gate::default();
    let mut actual = Vec::new();
    if t.latch(0) == Raise::Message {
        actual.push("real");
    }
    t.write(Reg::Leaf(0), 1);
    let before = image(&t);
    for now in 0..100 {
        if let Some(s) = &s {
            s.tick(
                || now * MS,
                &g,
                &t,
                [0],
                || {
                    actual.push("storm");
                    true
                },
            );
        }
    }
    assert_eq!(actual, ["real"]);
    assert_eq!(image(&t), before);
    assert_eq!(s.as_ref().map_or_else(String::new, Storm::summary), "");
}
#[test]
fn unset_preserves_the_real_interrupt_event_sequence() {
    off_sequence(None);
}
#[test]
fn explicit_zero_preserves_the_real_interrupt_event_sequence() {
    off_sequence(Some("0"));
}
#[test]
fn maximum_period_is_checked_even_with_explicit_all_mode() {
    assert_eq!(
        Config::parse(Some("1001"), Some("all")),
        Err(Invalid::Period)
    );
}
#[test]
fn a_valid_long_period_does_not_authorize_an_unknown_mode() {
    assert_eq!(
        Config::parse(Some("1000"), Some("random")),
        Err(Invalid::Mode)
    );
}
#[test]
fn normal_probe_timer_cadence_is_unchanged_with_no_storm() {
    assert_eq!(
        timer_interval(true, None),
        Some(std::time::Duration::from_millis(100))
    );
    assert_eq!(timer_interval(false, None), None);
}
#[test]
fn storm_timer_shares_the_probe_thread_without_enabling_channel_probes() {
    let s = storm(17);
    assert_eq!(
        timer_interval(false, Some(&s)),
        Some(std::time::Duration::from_millis(17))
    );
    let s = storm(1000);
    assert_eq!(
        timer_interval(false, Some(&s)),
        Some(std::time::Duration::from_millis(1000))
    );
    assert_eq!(
        timer_interval(true, Some(&s)),
        Some(std::time::Duration::from_millis(100))
    );
}
