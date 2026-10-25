#![cfg(feature = "diag-intr")]

use kf_qemu::intrdiag::{Census, Marker, Source};
use kf_trap::cpuintr::{CpuIntr, Raise, Reg};

fn tree() -> CpuIntr {
    CpuIntr::new(kf_chip::Family::Ada, 0xbb0000).unwrap()
}

fn armed() -> CpuIntr {
    let tree = tree();
    tree.write(Reg::LeafEnSet(2), 0xff);
    tree.write(Reg::LeafEnSet(4), 0xff);
    tree.write(Reg::TopEnSet, 0xff);
    tree
}

fn count(diag: &Census, name: &str) -> u64 {
    diag.snapshot(1_000_000_000).get(name)
}

#[test]
fn markers_separate_kind_and_interrupt_payload() {
    let d = Census::new();
    for kind in [Marker::Gate, Marker::Batch, Marker::Staged] {
        d.marker(kind, true);
        d.marker(kind, false);
    }
    for kind in ["gate", "batch", "staged"] {
        assert_eq!(count(&d, &format!("marker_{kind}_nsi")), 1);
        assert_eq!(count(&d, &format!("marker_{kind}_plain")), 1);
    }
}

#[test]
fn marker_totals_are_window_local() {
    let d = Census::new();
    d.marker(Marker::Gate, true);
    let before = d.snapshot(10);
    d.marker(Marker::Gate, true);
    d.marker(Marker::Batch, false);
    let delta = d.snapshot(20).since(&before);
    assert_eq!(delta.get("marker_gate_nsi"), 1);
    assert_eq!(delta.get("marker_batch_plain"), 1);
}

#[test]
fn host_edges_keep_source_vector_and_unmapped_notifications() {
    let d = Census::new();
    d.host_edge(Source::Engine, Some(64));
    d.host_edge(Source::Fifo, Some(128));
    d.host_edge(Source::Engine, None);
    assert_eq!(count(&d, "edge_engine_v64"), 1);
    assert_eq!(count(&d, "edge_fifo_v128"), 1);
    assert_eq!(count(&d, "edge_engine_v512"), 1);
    assert_eq!(count(&d, "edge_engine"), 2);
    assert_eq!(count(&d, "edge_fifo"), 1);
}

#[test]
fn repeated_edges_are_counted_even_without_a_guest_raise() {
    let d = Census::new();
    for _ in 0..7 {
        d.host_edge(Source::Engine, Some(64));
        d.host_edge(Source::Fifo, None);
    }
    assert_eq!(count(&d, "edge_engine_v64"), 7);
    assert_eq!(count(&d, "edge_fifo_v512"), 7);
    assert_eq!(count(&d, "messages"), 0);
}

#[test]
fn guest_raises_keep_logical_vector_and_eventfd_outcome() {
    let d = Census::new();
    let t = armed();
    assert_eq!(d.latch(&t, 64, Source::Engine, 10), Raise::Message);
    assert_eq!(d.latch(&t, 128, Source::Fifo, 20), Raise::Message);
    d.delivery(true);
    d.delivery(false);
    assert_eq!(count(&d, "raise_v64"), 1);
    assert_eq!(count(&d, "raise_v128"), 1);
    assert_eq!(count(&d, "messages"), 2);
    assert_eq!(count(&d, "eventfd_v0"), 1);
    assert_eq!(count(&d, "eventfd_errors"), 1);
}

#[test]
fn masked_latches_and_tree_retriggers_remain_distinct() {
    let d = Census::new();
    let t = tree();
    assert_eq!(d.latch(&t, 64, Source::Engine, 10), Raise::None);
    d.write(&t, Reg::LeafEnSet(2), 1, 20);
    assert_eq!(d.write(&t, Reg::TopEnSet, 2, 30), Raise::Message);
    assert_eq!(count(&d, "held"), 1);
    assert_eq!(count(&d, "messages"), 1);
    assert_eq!(count(&d, "raise_v64"), 1);
    assert_eq!(count(&d, "source_tree"), 1);
}

#[test]
fn an_isr_cycle_ends_at_the_root_unmask() {
    let d = Census::new();
    let t = armed();
    d.latch(&t, 64, Source::Engine, 100);
    d.write(&t, Reg::TopEnClear, 0xff, 110);
    d.write(&t, Reg::Leaf(2), 1, 120);
    d.write(&t, Reg::TopEnSet, 0xff, 150);
    assert_eq!(count(&d, "isr_cycles"), 1);
    assert_eq!(count(&d, "isr_ns"), 50);
    assert_eq!(count(&d, "cycle_starts"), 1);
}

#[test]
fn repeated_messages_do_not_restart_the_cycle_clock() {
    let d = Census::new();
    let t = armed();
    d.latch(&t, 64, Source::Engine, 10);
    d.latch(&t, 128, Source::Fifo, 20);
    d.write(&t, Reg::TopEnClear, 0xff, 30);
    d.write(&t, Reg::Leaf(2), 1, 40);
    d.write(&t, Reg::Leaf(4), 1, 45);
    d.write(&t, Reg::TopEnSet, 0xff, 60);
    assert_eq!(count(&d, "isr_cycles"), 1);
    assert_eq!(count(&d, "isr_ns"), 50);
}

#[test]
fn a_second_message_before_a_tree_write_is_a_merge_candidate() {
    let d = Census::new();
    let t = armed();
    d.latch(&t, 64, Source::Engine, 10);
    d.latch(&t, 128, Source::Fifo, 20);
    assert_eq!(count(&d, "merge_candidates"), 1);
    d.write(&t, Reg::LeafEnClear(2), 1, 40);
    d.latch(&t, 128, Source::Fifo, 50);
    assert_eq!(count(&d, "merge_candidates"), 1);
    assert_eq!(count(&d, "first_tree_writes"), 1);
    assert_eq!(count(&d, "first_write_ns"), 30);
}

#[test]
fn reads_do_not_end_the_merge_window() {
    let d = Census::new();
    let t = armed();
    d.latch(&t, 64, Source::Engine, 1);
    d.read(Reg::Top);
    d.latch(&t, 64, Source::Engine, 2);
    d.latch(&t, 64, Source::Engine, 3);
    assert_eq!(count(&d, "merge_candidates"), 2);
    assert_eq!(count(&d, "was_asserted"), 2);
}

#[test]
fn trapped_tree_accesses_keep_the_register_classes() {
    let d = Census::new();
    let t = tree();
    for r in [
        Reg::LeafEnSet(2), Reg::LeafEnClear(2), Reg::TopEnSet, Reg::TopEnClear,
        Reg::Leaf(2), Reg::Trigger, Reg::Top,
    ] {
        d.write(&t, r, 0, 1);
        d.read(r);
    }
    for name in ["en_set", "en_clear", "top_set", "top_clear", "w1c", "trigger", "top"] {
        assert_eq!(count(&d, &format!("write_{name}")), 1);
        assert_eq!(count(&d, &format!("read_{name}")), 1);
    }
}

#[test]
fn repeated_register_accesses_and_window_deltas_are_exact() {
    let d = Census::new();
    let t = tree();
    let before = d.snapshot(0);
    for _ in 0..4 {
        d.write(&t, Reg::LeafEnSet(4), 0xff, 1);
        d.write(&t, Reg::LeafEnClear(4), 0xff, 2);
        d.write(&t, Reg::Leaf(4), 0xff, 3);
        d.read(Reg::Leaf(4));
    }
    let delta = d.snapshot(10).since(&before);
    for name in ["write_en_set", "write_en_clear", "write_w1c", "read_w1c"] {
        assert_eq!(delta.get(name), 4);
    }
}

#[test]
fn overdue_enabled_pending_is_loud_and_counted_once_per_flight() {
    let d = Census::new();
    let t = armed();
    d.latch(&t, 64, Source::Engine, 1);
    d.sample(&t, 1_000_001);
    assert_eq!(count(&d, "stranded"), 0, "strictly more than one millisecond");
    d.sample(&t, 1_000_002);
    d.sample(&t, 2_000_000);
    assert_eq!(count(&d, "stranded"), 1);
}

#[test]
fn the_first_tree_write_checks_overdue_bits_before_closing_the_flight() {
    let d = Census::new();
    let t = armed();
    d.latch(&t, 128, Source::Fifo, 10);
    d.write(&t, Reg::TopEnClear, 0xff, 1_000_011);
    assert_eq!(count(&d, "stranded"), 1);
    d.sample(&t, 9_000_000);
    assert_eq!(count(&d, "stranded"), 1);
}

#[test]
fn masked_pending_and_already_serviced_flights_are_not_stranded() {
    let d = Census::new();
    let t = tree();
    d.latch(&t, 64, Source::Engine, 1);
    d.sample(&t, 3_000_000);
    assert_eq!(count(&d, "stranded"), 0);
    let t = armed();
    d.latch(&t, 64, Source::Engine, 4_000_000);
    d.write(&t, Reg::Leaf(2), 1, 4_000_001);
    d.sample(&t, 9_000_000);
    assert_eq!(count(&d, "stranded"), 0);
}

#[test]
fn only_the_exact_runtime_opt_in_allocates_an_observer() {
    for value in [None, Some("0"), Some("true"), Some(""), Some(" 1")] {
        assert!(Census::from_setting(value).is_none());
    }
    assert!(Census::from_setting(Some("1")).is_some());
}

#[test]
fn reset_closes_flights_without_erasing_cumulative_evidence() {
    let d = Census::new();
    let t = armed();
    d.latch(&t, 64, Source::Engine, 1);
    t.reset();
    d.reset();
    d.sample(&t, 3_000_000);
    assert_eq!(count(&d, "messages"), 1);
    assert_eq!(count(&d, "stranded"), 0);
    assert_eq!(count(&d, "resets"), 1);
}

#[test]
fn counters_normalize_over_the_same_time_and_frame_window() {
    let d = Census::new();
    let before = d.snapshot(1_000_000_000);
    for _ in 0..30 {
        d.marker(Marker::Batch, true);
    }
    let delta = d.snapshot(3_000_000_000).since(&before);
    assert_eq!(delta.per_second("marker_batch_nsi"), Some(15.0));
    assert_eq!(delta.per_frame("marker_batch_nsi", 10), Some(3.0));
    assert_eq!(delta.per_frame("marker_batch_nsi", 0), None);
    assert_eq!(before.since(&before).per_second("messages"), None);
}

#[test]
fn counters_are_atomic_across_producers() {
    let d = Census::new();
    std::thread::scope(|s| {
        for _ in 0..4 {
            let d = &d;
            s.spawn(move || {
                for _ in 0..1000 {
                    d.marker(Marker::Batch, true);
                    d.host_edge(Source::Engine, Some(64));
                    d.delivery(true);
                }
            });
        }
    });
    for key in ["marker_batch_nsi", "edge_engine_v64", "eventfd_v0"] {
        assert_eq!(count(&d, key), 4000);
    }
}

#[test]
fn observer_on_and_off_produce_identical_interrupt_and_shadow_sequences() {
    let a = tree();
    let b = tree();
    let d = Census::new();
    let mut seed = 7u64;
    for step in 1..20_000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let vector = ((seed >> 32) % 600) as u32;
        let r = match seed % 7 {
            0 => Reg::Leaf((seed as usize / 7) % 8),
            1 => Reg::LeafEnSet((seed as usize / 7) % 8),
            2 => Reg::LeafEnClear((seed as usize / 7) % 8),
            3 => Reg::TopEnSet,
            4 => Reg::TopEnClear,
            5 => Reg::Trigger,
            _ => Reg::Top,
        };
        let v = (seed >> 16) as u32;
        let (on, off) = if seed & 8 == 0 {
            (d.latch(&a, vector, Source::Engine, step), b.latch(vector))
        } else {
            (d.write(&a, r, v, step), b.write(r, v))
        };
        assert_eq!(on, off, "delivery at step {step}");
        let mut sa = Vec::new();
        let mut sb = Vec::new();
        a.shadow_all(|o, v| sa.push((o, v)));
        b.shadow_all(|o, v| sb.push((o, v)));
        assert_eq!(sa, sb, "shadow at step {step}");
    }
}
