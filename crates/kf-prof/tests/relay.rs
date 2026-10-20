//! Passive relay gauges: raw observations remain distinct from completion authority.
use kf_prof::relay::{Observation, RelayGauge, Wait, Wake};
#[test]
fn private_and_guest_progress_are_independent_and_missing_is_not_zero() {
    let mut g = RelayGauge::new(7, 0x11, 10);
    let w = g
        .observe(
            20,
            Observation {
                private_get: Some(4),
                reference: Some(9),
                guest_get: Some(2),
                ..Observation::default()
            },
        )
        .unwrap();
    assert_eq!((w[6], w[8], w[16]), (4, 9, 2));
    assert_eq!(w[5] & 2, 0);
    assert_ne!(w[5] & 1, 0);
}
#[test]
fn unchanged_private_get_keeps_its_change_time_despite_a_new_sample() {
    let mut g = RelayGauge::new(1, 1, 10);
    g.observe(
        20,
        Observation {
            private_get: Some(5),
            ..Observation::default()
        },
    )
    .unwrap();
    let w = g
        .observe(
            40,
            Observation {
                private_get: Some(5),
                reference: Some(6),
                ..Observation::default()
            },
        )
        .unwrap();
    assert_eq!(w[15], 20);
    assert_eq!(w[25], 40);
    assert_eq!(w[8], 6);
}
#[test]
fn wrapped_private_get_changes_its_age_without_changing_reference() {
    let mut g = RelayGauge::new(1, 1, 10);
    g.observe(
        20,
        Observation {
            private_get: Some(511),
            reference: Some(19),
            ..Observation::default()
        },
    )
    .unwrap();
    let w = g
        .observe(
            30,
            Observation {
                private_get: Some(0),
                ..Observation::default()
            },
        )
        .unwrap();
    assert_eq!(w[15], 30);
    assert_eq!(w[8], 19);
    assert_eq!(w[6], 0);
}
#[test]
fn unchanged_wait_retains_its_birth_and_new_target_starts_a_new_obligation() {
    let mut g = RelayGauge::new(1, 1, 10);
    g.observe(
        20,
        Observation {
            wait: Wait::Marker(9),
            ..Observation::default()
        },
    )
    .unwrap();
    let w = g
        .observe(
            30,
            Observation {
                wait: Wait::Marker(9),
                ..Observation::default()
            },
        )
        .unwrap();
    assert_eq!(w[27], 20);
    let w = g
        .observe(
            40,
            Observation {
                wait: Wait::Popped(10),
                ..Observation::default()
            },
        )
        .unwrap();
    assert_eq!((w[9], w[10], w[27]), (2, 10, 40));
}
#[test]
fn reference_does_not_close_a_wait_without_the_real_relay_transition() {
    let mut g = RelayGauge::new(1, 1, 10);
    let w = g
        .observe(
            20,
            Observation {
                reference: Some(99),
                wait: Wait::Marker(9),
                ..Observation::default()
            },
        )
        .unwrap();
    assert_eq!((w[9], w[10], w[27]), (1, 9, 20));
    let w = g
        .observe(
            30,
            Observation {
                wait: Wait::None,
                ..Observation::default()
            },
        )
        .unwrap();
    assert_eq!((w[9], w[27]), (0, 0));
}
#[test]
fn absent_reads_do_not_refresh_the_recorded_hardware_observation() {
    let mut g = RelayGauge::new(1, 1, 10);
    g.observe(
        20,
        Observation {
            private_get: Some(1),
            reference: Some(8),
            ..Observation::default()
        },
    )
    .unwrap();
    let w = g
        .observe(
            30,
            Observation {
                wake: Some(Wake::Event),
                ..Observation::default()
            },
        )
        .unwrap();
    assert_eq!(w[25], 20);
    assert_eq!(w[15], 20);
    assert_eq!((w[13], w[14]), (1, 30));
}
#[test]
fn doorbell_count_change_and_last_wake_are_separate_observations() {
    let mut g = RelayGauge::new(1, 1, 10);
    g.observe(
        20,
        Observation {
            doorbells: Some(4),
            wake: Some(Wake::Spin),
            ..Observation::default()
        },
    )
    .unwrap();
    let w = g
        .observe(
            30,
            Observation {
                doorbells: Some(4),
                wake: Some(Wake::Backstop),
                ..Observation::default()
            },
        )
        .unwrap();
    assert_eq!((w[11], w[12], w[13], w[14]), (4, 20, 3, 30));
}
#[test]
fn backward_clock_cannot_corrupt_a_published_relay_lifetime() {
    let mut g = RelayGauge::new(1, 1, 10);
    let old = g.observe(20, Observation::default()).unwrap();
    assert!(
        g.observe(
            19,
            Observation {
                private_get: Some(8),
                ..Observation::default()
            }
        )
        .is_err()
    );
    assert_eq!(g.observe(20, Observation::default()).unwrap(), old);
}
#[cfg(feature = "prof-inject")]
mod runtime {
    use super::*;
    use kf_prof::runtime::Session;
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn dir() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "kf-prof-relay-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn live_source_is_mapped_before_the_first_aggregator_second() {
        let p = dir();
        let s = Session::create_diagnostic(&p).unwrap();
        let mut r = s.register_relay(0x44).unwrap();
        r.observe(Observation {
            private_get: Some(4),
            private_put: Some(7),
            reference: Some(2),
            wait: Wait::Marker(9),
            ..Observation::default()
        });
        let b = std::fs::read(p.join("prof.mmap")).unwrap();
        let off = kf_prof::layout::LEDGER_BASE;
        let word =
            |i: usize| u64::from_le_bytes(b[off + i * 8..off + i * 8 + 8].try_into().unwrap());
        assert!(word(0) > 0 && word(0) % 2 == 0);
        assert_eq!(
            (word(3), word(7), word(8), word(9), word(10)),
            (0x44, 4, 7, 2, 1)
        );
        s.stop().unwrap();
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn relay_wait_is_an_overdue_ledger_source_and_real_idle_closes_it() {
        let p = dir();
        let s = Session::create_diagnostic(&p).unwrap();
        let mut r = s.register_relay(0x45).unwrap();
        r.observe(Observation {
            wait: Wait::Popped(8),
            ..Observation::default()
        });
        std::thread::sleep(Duration::from_millis(15));
        let row = s.snapshot();
        assert_eq!(row["relays"][0]["key"], 0x45);
        assert_eq!(row["ledger"]["oldest"][0]["kind"], 12);
        assert_eq!(row["ledger"]["oldest"][0]["overdue"], true);
        r.observe(Observation::default());
        assert_eq!(
            s.snapshot()["ledger"]["oldest"].as_array().unwrap().len(),
            0
        );
        s.stop().unwrap();
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn reused_guest_keys_keep_distinct_host_registration_generations() {
        let p = dir();
        let s = Session::create_diagnostic(&p).unwrap();
        let mut a = s.register_relay(7).unwrap();
        let mut b = s.register_relay(7).unwrap();
        a.observe(Observation {
            private_get: Some(1),
            ..Observation::default()
        });
        b.observe(Observation {
            private_get: Some(9),
            ..Observation::default()
        });
        let row = s.snapshot();
        assert_eq!(row["relays"].as_array().unwrap().len(), 2);
        assert_ne!(
            row["relays"][0]["generation"],
            row["relays"][1]["generation"]
        );
        assert_eq!(row["relays"][0]["private_get"], 1);
        assert_eq!(row["relays"][1]["private_get"], 9);
        s.stop().unwrap();
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn late_source_registration_generation_is_its_own_host_slot() {
        let p = dir();
        let s = Session::create_diagnostic(&p).unwrap();
        for key in 0..65 {
            let mut r = s.register_relay(key).unwrap();
            r.observe(Observation::default());
        }
        let row = s.snapshot();
        assert_eq!(row["relays"][64]["generation"], 65);
        s.stop().unwrap();
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn source_exhaustion_never_publishes_into_the_record_region() {
        let p = dir();
        let s = Session::create_diagnostic(&p).unwrap();
        for key in 0..4096 {
            let _ = s.register_relay(key).unwrap();
        }
        assert!(s.register_relay(8192).is_err());
        let b = std::fs::read(p.join("prof.mmap")).unwrap();
        let off = kf_prof::layout::RECORD_BASE;
        assert_ne!(
            u64::from_le_bytes(b[off + 8..off + 16].try_into().unwrap()),
            1,
            "a source tag must not overwrite a per-second record"
        );
        assert_eq!(s.snapshot()["relay_dropped"], 1);
        s.stop().unwrap();
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn every_relay_table_refusal_is_visible_and_never_overwrites() {
        let p = dir();
        let s = Session::create_diagnostic(&p).unwrap();
        for key in 0..4096 {
            assert!(s.register_relay(key).is_ok());
        }
        assert!(s.register_relay(9999).is_err());
        assert!(s.register_relay(10000).is_err());
        let row = s.snapshot();
        assert_eq!(row["relay_registered"], 4096);
        assert_eq!(row["relay_dropped"], 2);
        assert_eq!(row["relays"][0]["key"], 0);
        s.stop().unwrap();
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn duplicate_guest_keys_keep_separate_open_relay_obligations() {
        let p = dir();
        let s = Session::create_diagnostic(&p).unwrap();
        let mut a = s.register_relay(7).unwrap();
        let mut b = s.register_relay(7).unwrap();
        a.observe(Observation {
            wait: Wait::Marker(9),
            ..Observation::default()
        });
        b.observe(Observation {
            wait: Wait::Popped(11),
            ..Observation::default()
        });
        let row = s.snapshot();
        assert_eq!(row["ledger"]["counts"][12], 2);
        assert_eq!(row["ledger"]["oldest"].as_array().unwrap().len(), 2);
        s.stop().unwrap();
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn high_reference_does_not_override_the_relays_recorded_wait() {
        let p = dir();
        let s = Session::create_diagnostic(&p).unwrap();
        let mut a = s.register_relay(7).unwrap();
        a.observe(Observation {
            wait: Wait::Marker(9),
            reference: Some(99),
            private_get: Some(100),
            ..Observation::default()
        });
        assert_eq!(s.snapshot()["ledger"]["counts"][12], 1);
        s.stop().unwrap();
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn snapshot_retains_private_put_and_marks_stale_reads_with_their_original_stamp() {
        let p = dir();
        let s = Session::create_diagnostic(&p).unwrap();
        let mut a = s.register_relay(7).unwrap();
        a.observe(Observation {
            private_get: Some(1),
            private_put: Some(7),
            ..Observation::default()
        });
        let before = s.snapshot()["relays"][0]["private_observed_ns"].clone();
        a.observe(Observation::default());
        let row = s.snapshot();
        assert_eq!(row["relays"][0]["private_put"], 7);
        assert_eq!(row["relays"][0]["private_depth"], 6);
        assert_eq!(row["relays"][0]["private_observed_ns"], before);
        s.stop().unwrap();
        std::fs::remove_dir_all(p).unwrap();
    }

    #[test]
    fn private_depth_counts_a_wrap_at_the_real_storage_capacity() {
        let p = dir();
        let s = Session::create_diagnostic(&p).unwrap();
        let mut a = s.register_relay(7).unwrap();
        a.observe(Observation {
            private_get: Some(511),
            private_put: Some(3),
            ..Observation::default()
        });
        assert_eq!(s.snapshot()["relays"][0]["private_depth"], 4);
        s.stop().unwrap();
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn private_depth_uses_the_reported_capacity_for_a_different_wrap() {
        let p = dir();
        let s = Session::create_diagnostic(&p).unwrap();
        let mut a = s.register_relay(7).unwrap();
        a.observe(Observation {
            private_get: Some(7),
            private_put: Some(1),
            private_entries: 8,
            ..Observation::default()
        });
        assert_eq!(s.snapshot()["relays"][0]["private_depth"], 2);
        s.stop().unwrap();
        std::fs::remove_dir_all(p).unwrap();
    }
}

#[cfg(feature = "prof-inject")]
#[test]
fn oversize_relay_and_thread_record_is_bounded_with_explicit_omissions() {
    let mut row = serde_json::json!({"schema":1,"start_ns":1,"end_ns":2,"threads":vec![serde_json::json!({"states":vec![0;1024],"sites":vec![0;1024],"tid":7});64],"relays":vec![serde_json::json!({"key":7,"padding":vec![0;1024]});4096],"coverage":{}});
    let b = kf_prof::runtime::bounded_record(&mut row).unwrap();
    assert!(b.len() <= 65472);
    assert_eq!(row["coverage"]["payload_truncated"], true);
    assert!(row["coverage"]["relay_rows_omitted"].as_u64().unwrap() > 0);
}
#[cfg(feature = "prof-inject")]
#[test]
fn irreducible_record_overflow_is_an_error_before_any_publication() {
    let mut row = serde_json::json!({"schema":1,"start_ns":1,"end_ns":2,"coverage":{},"irreducible":"x".repeat(70000)});
    assert!(kf_prof::runtime::bounded_record(&mut row).is_err());
}
#[test]
fn private_get_ahead_of_marker_never_supplies_a_processing_receipt() {
    let mut g = RelayGauge::new(1, 1, 10);
    let w = g
        .observe(
            20,
            Observation {
                private_get: Some(100),
                reference: Some(1),
                wait: Wait::Marker(9),
                ..Observation::default()
            },
        )
        .unwrap();
    assert_eq!((w[9], w[10], w[27]), (1, 9, 20));
}

#[test]
fn changing_hardware_values_do_not_restart_an_unchanged_wait() {
    let mut g = RelayGauge::new(2, 7, 1);
    g.observe(
        10,
        Observation {
            wait: Wait::Marker(9),
            reference: Some(1),
            ..Observation::default()
        },
    )
    .unwrap();
    let w = g
        .observe(
            20,
            Observation {
                wait: Wait::Marker(9),
                reference: Some(3),
                private_get: Some(8),
                ..Observation::default()
            },
        )
        .unwrap();
    assert_eq!(w[27], 10);
}
#[test]
fn confirmed_private_put_is_retained_when_no_new_publication_occurs() {
    let mut g = RelayGauge::new(1, 1, 1);
    g.observe(
        10,
        Observation {
            private_put: Some(7),
            ..Observation::default()
        },
    )
    .unwrap();
    let w = g
        .observe(
            20,
            Observation {
                private_get: Some(2),
                ..Observation::default()
            },
        )
        .unwrap();
    assert_eq!(w[7], 7);
    assert_ne!(w[5] & 2, 0);
    assert_eq!(w[26], 10);
}
#[test]
fn a_backward_clock_cannot_relabel_an_outstanding_wait() {
    let mut g = RelayGauge::new(1, 1, 1);
    g.observe(
        10,
        Observation {
            wait: Wait::Marker(9),
            ..Observation::default()
        },
    )
    .unwrap();
    assert!(
        g.observe(
            9,
            Observation {
                wait: Wait::Popped(11),
                ..Observation::default()
            }
        )
        .is_err()
    );
    let w = g
        .observe(
            10,
            Observation {
                wait: Wait::Marker(9),
                ..Observation::default()
            },
        )
        .unwrap();
    assert_eq!((w[9], w[10], w[27]), (1, 9, 10));
}

#[test]
fn repeated_zero_after_get_wrap_does_not_reset_the_change_time() {
    let mut g = RelayGauge::new(1, 1, 1);
    g.observe(
        10,
        Observation {
            private_get: Some(511),
            ..Observation::default()
        },
    )
    .unwrap();
    g.observe(
        20,
        Observation {
            private_get: Some(0),
            ..Observation::default()
        },
    )
    .unwrap();
    let w = g
        .observe(
            30,
            Observation {
                private_get: Some(0),
                wake: Some(Wake::Event),
                ..Observation::default()
            },
        )
        .unwrap();
    assert_eq!(w[15], 20);
}

#[test]
fn unchanged_guest_get_retains_its_own_change_time() {
    let mut g = RelayGauge::new(1, 1, 10);
    g.observe(
        20,
        Observation {
            guest_get: Some(7),
            ..Observation::default()
        },
    )
    .unwrap();
    let w = g
        .observe(
            40,
            Observation {
                guest_get: Some(7),
                private_get: Some(3),
                ..Observation::default()
            },
        )
        .unwrap();
    assert_eq!(w[30], 20);
    assert_eq!(w[15], 40);
}
#[test]
fn guest_get_wrap_changes_once_and_no_fresh_read_keeps_its_age() {
    let mut g = RelayGauge::new(1, 1, 10);
    g.observe(
        20,
        Observation {
            guest_get: Some(7),
            ..Observation::default()
        },
    )
    .unwrap();
    assert_eq!(
        g.observe(
            30,
            Observation {
                guest_get: Some(0),
                ..Observation::default()
            }
        )
        .unwrap()[30],
        30
    );
    assert_eq!(
        g.observe(
            80,
            Observation {
                guest_get: Some(0),
                ..Observation::default()
            }
        )
        .unwrap()[30],
        30
    );
    assert_eq!(g.observe(90, Observation::default()).unwrap()[30], 30);
}

#[cfg(feature = "prof-inject")]
fn runtime_guest_geometry(get: u32, put: u32, entries: u32, private: u32, expected: u64) {
    let path =
        std::env::temp_dir().join(format!("kf-prof-k1-guest-{}-{entries}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let s = kf_prof::runtime::Session::create_diagnostic(&path).unwrap();
    let mut r = s.register_relay(1).unwrap();
    r.observe(Observation {
        guest_get: Some(get),
        guest_put: Some(put),
        guest_entries: entries,
        private_entries: private,
        ..Observation::default()
    });
    let row = s.snapshot();
    assert_eq!(row["relays"][0]["guest_depth"], expected);
    let changed = row["relays"][0]["guest_get_changed_ns"].as_u64().unwrap();
    assert!(changed > 0);
    r.observe(Observation {
        guest_entries: entries,
        ..Observation::default()
    });
    assert_eq!(s.snapshot()["relays"][0]["guest_get_changed_ns"], changed);
    s.stop().unwrap();
    std::fs::remove_dir_all(path).unwrap();
}
#[cfg(feature = "prof-inject")]
#[test]
fn runtime_guest_wrap_uses_eight_entries_not_the_private_geometry() {
    runtime_guest_geometry(7, 1, 8, 512, 2);
}
#[cfg(feature = "prof-inject")]
#[test]
fn runtime_guest_wrap_uses_512_entries_even_when_private_ring_is_smaller() {
    runtime_guest_geometry(511, 3, 512, 16, 4);
}

#[test]
fn retiring_a_producer_preserves_its_unanswered_processing_wait() {
    let mut g = RelayGauge::new(1, 4, 10);
    g.observe(
        20,
        Observation {
            private_get: Some(1),
            wait: Wait::Marker(9),
            ..Observation::default()
        },
    )
    .unwrap();
    let w = g.retire(40).unwrap();
    assert_eq!(w[23], 0);
    assert_eq!((w[9], w[10], w[27]), (1, 9, 20));
    assert_eq!(w[25], 20);
}
#[test]
fn retirement_has_its_own_observation_without_fabricating_descriptor_progress() {
    let mut g = RelayGauge::new(2, 7, 10);
    g.observe(
        20,
        Observation {
            private_get: Some(3),
            private_put: Some(6),
            reference: Some(1),
            wait: Wait::Popped(7),
            ..Observation::default()
        },
    )
    .unwrap();
    let w = g.retire(50).unwrap();
    assert_eq!((w[23], w[4]), (0, 50));
    assert_eq!(
        (w[6], w[7], w[8], w[15], w[25], w[27]),
        (3, 6, 1, 20, 20, 20)
    );
}
#[cfg(feature = "prof-inject")]
#[test]
fn dropped_relay_probe_is_retired_in_the_live_file() {
    let path = std::env::temp_dir().join(format!("kf-prof-k1-retired-live-{}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let s = kf_prof::runtime::Session::create_diagnostic(&path).unwrap();
    let mut r = s.register_relay(1).unwrap();
    r.observe(Observation {
        wait: Wait::Marker(9),
        ..Observation::default()
    });
    drop(r);
    let row = s.snapshot();
    assert_eq!(row["relays"][0]["active"], false);
    assert_eq!(row["ledger"]["counts"][12], 1);
    s.stop().unwrap();
    std::fs::remove_dir_all(path).unwrap();
}
#[cfg(feature = "prof-inject")]
#[test]
fn dropping_a_descriptor_wait_does_not_close_an_unanswered_obligation() {
    let path =
        std::env::temp_dir().join(format!("kf-prof-k1-retired-pending-{}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let s = kf_prof::runtime::Session::create_diagnostic(&path).unwrap();
    let mut r = s.register_relay(2).unwrap();
    r.observe(Observation {
        wait: Wait::Popped(17),
        ..Observation::default()
    });
    drop(r);
    let row = s.snapshot();
    assert_eq!(row["relays"][0]["active"], false);
    assert_eq!(row["ledger"]["oldest"][0]["kind"], 12);
    assert_eq!(row["ledger"]["oldest"][0]["source_active"], false);
    s.stop().unwrap();
    std::fs::remove_dir_all(path).unwrap();
}
