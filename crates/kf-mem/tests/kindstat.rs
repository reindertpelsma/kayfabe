//! Kind-pair accounting and map-path equivalence, without GPU access.
#![cfg(feature = "diag-kindstat")]

use kf_mem::apply::{ApplyCfg, DiffRun, apply_entry_observed as apply_entry};
use kf_mem::kindstat::Census;
use kf_mem::ledger::{Desired, MapTarget, Mapped};
use std::cell::RefCell;

#[derive(Default)]
struct Target {
    rows: RefCell<Vec<Desired>>,
    refuse: bool,
    held: bool,
    cpu: bool,
    batch: bool,
}

impl MapTarget for Target {
    fn map_batch(&self, rows: &[Desired], _: bool) -> Result<(), String> {
        if !self.batch {
            return Err(kf_mem::ledger::NOT_BATCHED.into());
        }
        self.rows.borrow_mut().extend_from_slice(rows);
        Ok(())
    }
    fn map(&self, row: &Desired, _: bool) -> Result<Mapped, String> {
        if self.refuse {
            return Err("test refusal".into());
        }
        if self.held {
            return Ok(Mapped::HeldByHost);
        }
        self.rows.borrow_mut().push(*row);
        Ok(Mapped::Placed)
    }
    fn unmap(&self, _: u64, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn invalidate(&self) -> Result<(), String> {
        Ok(())
    }
    fn gpu_space(&self) -> bool {
        !self.cpu
    }
}

fn config() -> ApplyCfg<'static> {
    ApplyCfg {
        store_bytes: 0x1000_0000,
        grain: 4096,
        ram_offset: &|gpa, _| Some(gpa),
        usermode: None,
        per_map_kind: true,
        carve: 0x1000_0000,
        carve_refuse: true,
    }
}

fn run(kind: u8, ram: bool, leaf: u64, len: u64) -> DiffRun {
    DiffRun {
        unmap: false,
        va: 0x2000_0000,
        len,
        at: 0x400000,
        ap: if ram { 2 } else { 0 },
        held: false,
        kind,
        perm: kf_host::MapPerm::READ_WRITE,
        privileged: false,
        leaf,
    }
}

#[test]
fn raw_host_aperture_and_guest_page_size_are_separate_bins() {
    let d = Census::new();
    for ram in [false, true] {
        for leaf in [4096, 65536, 2097152] {
            let t = Target::default();
            let a = apply_entry(&t, &[run(8, ram, leaf, leaf)], &config(), Some(&d));
            assert_eq!(a.mapped, 1);
            assert_eq!(d.snapshot(1).bytes(8, 6, ram, leaf), leaf);
            assert_eq!(d.snapshot(1).pages4k(8, 6, ram, leaf), leaf / 4096);
        }
    }
}

#[test]
fn unknown_and_depth_raw_kinds_survive_the_census() {
    let d = Census::new();
    for (raw, host) in [(0x0b, 1), (0x0e, 5), (0xfe, 0)] {
        let t = Target::default();
        apply_entry(&t, &[run(raw, false, 65536, 131072)], &config(), Some(&d));
        assert_eq!(d.snapshot(1).bytes(raw, host, false, 65536), 131072);
        assert_eq!(d.snapshot(1).pages4k(raw, host, false, 65536), 32);
    }
}

#[test]
fn cumulative_bytes_and_interval_rates_do_not_count_runs_as_pages() {
    let d = Census::new();
    let before = d.snapshot(1_000_000_000);
    d.record_mapping(8, 6, false, 2097152, 4194304);
    d.record_mapping(8, 6, false, 2097152, 2097152);
    let delta = d.snapshot(3_000_000_000).since(&before);
    assert_eq!(delta.bytes(8, 6, false, 2097152), 6291456);
    assert_eq!(delta.pages4k(8, 6, false, 2097152), 1536);
    assert_eq!(
        delta.bytes_per_second(8, 6, false, 2097152),
        Some(3145728.0)
    );
}

#[test]
fn byte_and_page_totals_are_exact_in_a_second_window() {
    let d = Census::new();
    d.record_mapping(9, 6, true, 4096, 4096);
    let before = d.snapshot(10);
    d.record_mapping(9, 6, true, 4096, 12288);
    let delta = d.snapshot(20).since(&before);
    assert_eq!(delta.bytes(9, 6, true, 4096), 12288);
    assert_eq!(delta.pages4k(9, 6, true, 4096), 3);
    assert_eq!(d.snapshot(20).bytes(9, 6, true, 4096), 16384);
}

#[test]
fn refused_and_held_rows_are_not_successful_maps() {
    let d = Census::new();
    for t in [
        Target {
            refuse: true,
            ..Target::default()
        },
        Target {
            held: true,
            ..Target::default()
        },
    ] {
        apply_entry(&t, &[run(8, false, 4096, 4096)], &config(), Some(&d));
    }
    assert_eq!(d.snapshot(1).bytes(8, 6, false, 4096), 0);
}

#[test]
fn cpu_views_are_not_host_gpu_pte_mappings() {
    let d = Census::new();
    let t = Target {
        cpu: true,
        ..Target::default()
    };
    let a = apply_entry(&t, &[run(8, false, 4096, 4096)], &config(), Some(&d));
    assert_eq!(a.mapped, 1);
    assert_eq!(d.snapshot(1).bytes(8, 6, false, 4096), 0);
}

#[test]
fn runtime_opt_in_is_exact_and_defaults_off() {
    for s in [None, Some("0"), Some("true"), Some(" 1")] {
        assert!(Census::from_setting(s).is_none());
    }
    assert!(Census::from_setting(Some("1")).is_some());
}

#[test]
fn off_observer_changes_neither_verdicts_nor_host_requests() {
    let d = Census::new();
    assert!(Census::from_setting(None).is_none());
    for ram in [false, true] {
        for raw in [0, 6, 8, 9, 0xb, 0xfe] {
            let a = Target::default();
            let b = Target::default();
            let rows = [run(raw, ram, 65536, 131072)];
            assert_eq!(
                apply_entry(&a, &rows, &config(), Some(&d)),
                apply_entry(&b, &rows, &config(), None)
            );
            assert_eq!(*a.rows.borrow(), *b.rows.borrow());
        }
    }
}

#[test]
fn nonzero_prefix_is_removed_from_each_window_and_rate() {
    let d = Census::new();
    d.record_mapping(8, 6, false, 65536, 65536);
    let before = d.snapshot(1_000_000_000);
    d.record_mapping(8, 6, false, 65536, 131072);
    let delta = d.snapshot(2_000_000_000).since(&before);
    assert_eq!(delta.bytes(8, 6, false, 65536), 131072);
    assert_eq!(delta.bytes_per_second(8, 6, false, 65536), Some(131072.0));
}

#[test]
fn cpu_depth_views_are_excluded_even_when_the_mapping_succeeds() {
    let d = Census::new();
    let t = Target {
        cpu: true,
        ..Target::default()
    };
    let a = apply_entry(&t, &[run(0xb, false, 65536, 65536)], &config(), Some(&d));
    assert_eq!(a.mapped, 1);
    assert_eq!(d.snapshot(1).bytes(0xb, 1, false, 65536), 0);
}

#[test]
fn held_sysmem_is_not_our_host_pte() {
    let d = Census::new();
    let t = Target {
        held: true,
        ..Target::default()
    };
    let a = apply_entry(&t, &[run(9, true, 65536, 65536)], &config(), Some(&d));
    assert_eq!(a.held, 1);
    assert_eq!(d.snapshot(1).bytes(9, 6, true, 65536), 0);
}

#[test]
fn batched_and_fallback_sysmem_rows_have_identical_byte_counts() {
    for batch in [false, true] {
        let d = Census::new();
        let t = Target {
            batch,
            ..Target::default()
        };
        let a = run(8, true, 4096, 4096);
        let b = DiffRun {
            va: a.va + 4096,
            at: a.at + 4096,
            ..a
        };
        let result = apply_entry(&t, &[a, b], &config(), Some(&d));
        assert_eq!(result.mapped, 2);
        assert_eq!(d.snapshot(1).bytes(8, 6, true, 4096), 8192);
        assert_eq!(d.snapshot(1).pages4k(8, 6, true, 4096), 2);
    }
}

#[test]
fn status_contains_raw_host_pages_and_a_single_shutdown_summary() {
    let d = Census::new();
    d.record_mapping(8, 6, false, 4096, 8192);
    let report = d.report();
    assert!(report.contains("\"raw\":8,\"host\":6,\"ram\":false,\"page\":\"4K\""));
    assert!(report.contains("\"bytes\":8192,\"pages4k\":2"));
    assert!(report.contains("\"compressible_bytes\":8192,\"stripped_bytes\":8192"));
    assert!(d.shutdown().is_some());
    assert!(d.shutdown().is_none());
}

#[test]
fn concurrent_mapping_bytes_are_not_lost() {
    let d = Census::new();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let d = &d;
            scope.spawn(move || {
                for _ in 0..1000 {
                    d.record_mapping(8, 6, false, 4096, 4096);
                }
            });
        }
    });
    assert_eq!(d.snapshot(1).bytes(8, 6, false, 4096), 4096 * 4000);
    assert_eq!(d.snapshot(1).pages4k(8, 6, false, 4096), 4000);
}
