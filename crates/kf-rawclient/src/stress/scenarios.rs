//! Scenario tests: the whole arm against the fake backend. Each test names the property it proves.

use super::backend::{BackendError, Probe};
use super::fake::{Costs, Fake, Fault, Frag};
use super::params::{Shape, StressParams};
use super::run::{StressEnv, StressReport, run_stress};
use crate::caps::{HostFacts, VaBand, test_facts};
use crate::ledger::{CheckState, VecSink};
use crate::mode::Mode;
use crate::outcome::{NotRunReason, Outcome};
use crate::registry::REFRESH_STRESS;
use crate::watchdog::{NullProgress, Progress};
use std::cell::RefCell;

const SEED: u64 = 0x5eed;

fn small() -> StressParams {
    let p = StressParams {
        batches: 4,
        pages_per_batch: 16,
        sample: 16,
        live: 3,
        refreshes: 3,
        seed: Some(SEED),
        ..StressParams::default()
    };
    p.validate().expect("the small shape is valid");
    p
}

fn go_with(
    p: &StressParams,
    fake: &mut Fake,
    progress: &dyn Progress,
) -> (StressReport, Vec<String>) {
    let clock = fake.clock.clone();
    let mut sink = VecSink::default();
    let mut env = StressEnv {
        clock: &clock,
        progress,
        sink: &mut sink,
        mode: Mode::Native,
        rev: Some("r1".into()),
        default_seed: 1,
    };
    let rep = run_stress(p, &REFRESH_STRESS, fake, &mut env);
    (rep, sink.lines)
}

fn go(p: &StressParams, fake: &mut Fake) -> (StressReport, Vec<String>) {
    go_with(p, fake, &NullProgress)
}

fn has(lines: &[String], prefix: &str) -> bool {
    lines.iter().any(|l| l.starts_with(prefix))
}

fn state(rep: &StressReport, name: &str) -> CheckState {
    rep.checks
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no check {name}"))
        .state
}

fn notrun(rep: &StressReport) -> (NotRunReason, String) {
    match &rep.outcome {
        Outcome::NotRun(r, d) => (*r, d.clone()),
        other => panic!("expected NOTRUN, got {other:?}"),
    }
}

#[test]
fn a_good_run_passes_with_the_documented_output() {
    let mut f = Fake::new();
    let (rep, lines) = go(&small(), &mut f);
    assert_eq!(rep.outcome, Outcome::Pass, "{lines:#?}");
    assert_eq!(rep.outcome.exit_code(), 0);
    assert_eq!(rep.refreshes.len(), 3);
    assert_eq!(rep.control, Some(true));
    assert!(
        rep.checks.iter().all(|c| c.state == CheckState::Pass),
        "{:?}",
        rep.checks
    );
    // The contract lines.
    assert!(has(&lines, "STRESS_PARAMS batches=4"));
    assert!(lines.iter().any(
        |l| l.starts_with("RAWCLIENT_CELL die=GA106 family=Ampere mmu=Ver2")
            && l.ends_with("mode=bare rev=r1")
    ));
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("STRESS_QUALIFY "))
            .count(),
        3,
        "qualified every refresh"
    );
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("STRESS_REFRESH n="))
            .count(),
        3
    );
    for op in ["pin", "map", "inval", "unmap", "copy"] {
        assert!(has(&lines, &format!("STRESS_HIST op={op} n=")), "{op}");
    }
    assert!(has(&lines, "CHECK oracle_decoy PASS"));
    assert!(has(&lines, "STRESS_SUMMARY completed=3 attempted=3"));
    assert!(has(&lines, "RUNGCTL_refresh_stress=PASS"));
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("RUNG_refresh_stress="))
            .count(),
        1
    );
    assert_eq!(
        lines.last().map(|l| l.starts_with("RAWCLIENT_JSON {")),
        Some(true)
    );
    let json = lines.last().unwrap();
    assert!(json.contains("\"outcome\":\"PASS\"") && json.contains("\"cell\":{\"die\":\"GA106\""));
    assert!(json.contains("\"completed\":3"));
    // Nothing printed is a multi-line value.
    assert!(lines.iter().all(|l| !l.contains('\n')));
}

#[test]
fn exactly_one_invalidate_per_refresh_and_in_the_designed_order() {
    let mut f = Fake::new();
    let (rep, _) = go(&small(), &mut f);
    assert_eq!(rep.outcome, Outcome::Pass);
    assert_eq!(
        f.invalidates_this_refresh,
        vec![1, 1, 1],
        "ONE invalidate after N batches"
    );
    assert_eq!(f.count("pin "), 12);
    assert_eq!(f.count("map "), 12);
    assert_eq!(f.count("unmap "), 12);
    assert_eq!(f.count("free "), 12);
    assert_eq!(f.teardown_calls, 1);
    // Within a refresh: pins, maps, invalidate, copies, unmaps, probes, frees; no copy before the
    // invalidate and none after the first unmap (the fake would have refused it).
    let pos = |needle: &str, from: usize| {
        f.log[from..]
            .iter()
            .position(|l| l.starts_with(needle))
            .map(|p| p + from)
    };
    let a = pos("alloc_workload", 0).unwrap();
    let pins: Vec<usize> = (0..4)
        .map(|i| pos(&format!("pin {i}"), a).unwrap())
        .collect();
    let maps: Vec<usize> = (0..4)
        .map(|i| pos(&format!("map {i}"), a).unwrap())
        .collect();
    let inval = pos("invalidate", a).unwrap();
    let first_copy = pos("copy ", a).unwrap();
    let first_unmap = pos("unmap ", a).unwrap();
    assert!(
        pins.iter().max() < maps.iter().min(),
        "all pins before the first map"
    );
    assert!(
        maps.iter().max().unwrap() < &inval,
        "every map before the invalidate"
    );
    assert!(inval < first_copy && first_copy < first_unmap);
    // The pressure region exists before the workload and is released before the first pin.
    let pb = pos("pressure_begin", 0).unwrap();
    let pe = pos("pressure_end", pb).unwrap();
    assert!(pb < a && pe < pins[0]);
}

#[test]
fn the_default_shape_is_24_batches_of_4096_pages() {
    let p = StressParams {
        seed: Some(SEED),
        ..StressParams::default()
    };
    let mut f = Fake::new();
    let (rep, lines) = go(&p, &mut f);
    assert_eq!(rep.outcome, Outcome::Pass, "{:?}", rep.checks);
    assert_eq!(f.count("alloc_workload 98304"), 3);
    assert_eq!(f.count("pin "), 72);
    assert_eq!(f.invalidates_this_refresh, vec![1, 1, 1]);
    // 1,536 sampled pages and 32 live pages per refresh.
    assert_eq!(f.count("copy "), 3 * (1536 + 32));
    assert!(
        has(&lines, "STRESS_HIST op=map n=72 "),
        "24 map calls x 3 refreshes"
    );
    assert!(has(
        &lines,
        "STRESS_PARAMS batches=24 pages_per_batch=4096 leaf=4096 refreshes=3 sample=1536 live=32"
    ));
}

#[test]
fn the_same_seed_reproduces_the_run_and_a_new_seed_changes_the_sample() {
    let copies = |seed: u64| {
        let mut f = Fake::new();
        let p = StressParams {
            seed: Some(seed),
            ..small()
        };
        let (rep, _) = go(&p, &mut f);
        assert_eq!(rep.outcome, Outcome::Pass);
        f.log
            .into_iter()
            .filter(|l| l.starts_with("copy "))
            .collect::<Vec<_>>()
    };
    assert_eq!(copies(7), copies(7));
    assert_ne!(copies(7), copies(8));
    // Without --stress-seed the caller's default seed is used and printed.
    let mut f = Fake::new();
    let p = StressParams {
        seed: None,
        ..small()
    };
    let (rep, lines) = go(&p, &mut f);
    assert_eq!(rep.seed, 1);
    assert!(has(&lines, "STRESS_PARAMS") && lines[0].ends_with("seed=0x1"));
}

#[test]
fn an_unqualified_workload_decides_nothing_and_pins_nothing() {
    let mut f = Fake::new();
    f.frag = Frag::Contiguous;
    let (rep, lines) = go(&small(), &mut f);
    let (r, d) = notrun(&rep);
    assert_eq!(r, NotRunReason::NotQualified);
    assert!(d.starts_with("run-ratio"), "{d}");
    assert_eq!(rep.outcome.exit_code(), 3);
    assert_eq!(
        f.count("pin "),
        0,
        "nothing is pinned before the census passes"
    );
    assert_eq!(f.count("map "), 0);
    assert_eq!(f.count("invalidate"), 0);
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("STRESS_QUALIFY ") && l.ends_with("result=notqualified"))
    );
    assert!(has(
        &lines,
        "RUNG_refresh_stress=NOTRUN reason=notqualified"
    ));
    assert!(
        !has(&lines, "RUNGCTL_"),
        "a control that never ran is not printed"
    );
    assert_eq!(
        f.teardown_calls, 1,
        "resources are released on a not-run too"
    );
    assert!(rep.checks.is_empty());
}

#[test]
fn fragmentation_is_a_requested_floor_not_a_constant() {
    // Scattered frames give ratio 1.0; a floor above 1.0 cannot be requested, and a modest floor passes.
    let mut f = Fake::new();
    let p = StressParams {
        min_run_permille: 1000,
        ..small()
    };
    assert_eq!(go(&p, &mut f).0.outcome, Outcome::Pass);
}

#[test]
fn hidden_frames_thp_and_missing_measurements_are_each_named() {
    let mut f = Fake::new();
    f.frag = Frag::Hidden;
    assert_eq!(
        notrun(&go(&small(), &mut f).0).0,
        NotRunReason::PagemapPfnHidden
    );

    let mut f = Fake::new();
    f.thp = Some(2 << 20);
    let (r, d) = notrun(&go(&small(), &mut f).0);
    assert_eq!(
        (r, d.as_str()),
        (NotRunReason::NotQualified, "thp-bytes=2097152")
    );

    let mut f = Fake::new();
    f.thp = None;
    assert_eq!(
        notrun(&go(&small(), &mut f).0).0,
        NotRunReason::ThpUnmeasurable
    );
}

#[test]
fn a_host_that_lacks_a_capability_is_notrun_before_any_setup() {
    let cases: Vec<(HostFacts, NotRunReason)> = vec![
        (
            HostFacts {
                ce_class: None,
                ..test_facts()
            },
            NotRunReason::NoCe,
        ),
        (
            HostFacts {
                host_page_bytes: 65536,
                ..test_facts()
            },
            NotRunReason::GrainMismatch,
        ),
        (
            HostFacts {
                va_band: None,
                ..test_facts()
            },
            NotRunReason::NoGuestVaBand,
        ),
        (
            HostFacts {
                va_band: Some(VaBand { base: 0, len: 4096 }),
                ..test_facts()
            },
            NotRunReason::NoGuestVaBand,
        ),
        (
            HostFacts {
                pagemap_pfn_visible: Some(false),
                ..test_facts()
            },
            NotRunReason::PagemapPfnHidden,
        ),
    ];
    for (facts, want) in cases {
        let mut f = Fake::new();
        f.facts = facts;
        let (rep, lines) = go(&small(), &mut f);
        assert_eq!(notrun(&rep).0, want);
        assert_eq!(f.count("setup"), 0);
        assert_eq!(rep.outcome.exit_code(), 3);
        assert!(
            has(&lines, "RAWCLIENT_CELL"),
            "the cell is printed even when the run is not"
        );
    }
}

#[test]
fn no_device_and_too_little_memory_are_notrun_never_a_crash() {
    let mut f = Fake::new();
    f.fail_facts = Some(BackendError::Unsupported(
        NotRunReason::NoRmDevice,
        "open nvidiactl: ENOENT".into(),
    ));
    let (rep, lines) = go(&small(), &mut f);
    assert_eq!(notrun(&rep).0, NotRunReason::NoRmDevice);
    assert!(!has(&lines, "RAWCLIENT_CELL"), "no facts, no cell");
    assert!(has(
        &lines,
        "RUNG_refresh_stress=NOTRUN reason=no-rm-device"
    ));

    let mut f = Fake::new();
    f.avail = 64 << 20;
    let (rep, _) = go(&StressParams::default(), &mut f);
    assert_eq!(notrun(&rep).0, NotRunReason::TooLittleMemory);
    assert_eq!(
        f.count("alloc_workload"),
        0,
        "no allocation when the plan does not fit"
    );
}

#[test]
fn a_map_that_points_at_the_wrong_page_fails_the_oracle() {
    let mut f = Fake::new();
    f.fault = Fault::WrongPage;
    let (rep, lines) = go(&small(), &mut f);
    assert!(matches!(rep.outcome, Outcome::Fail(_)), "{:?}", rep.outcome);
    assert_eq!(rep.outcome.exit_code(), 1);
    assert_eq!(state(&rep, "oracle_pages"), CheckState::Fail);
    assert_eq!(
        state(&rep, "oracle_decoy"),
        CheckState::Pass,
        "the control still works: the instrument is sound"
    );
    assert!(has(&lines, "CHECK oracle_pages FAIL checked="));
    assert!(
        has(&lines, "RUNG_refresh_stress=FAIL why=")
            && lines.iter().any(|l| l.contains("oracle_pages"))
    );
    assert!(has(&lines, "RUNGCTL_refresh_stress=PASS"));
}

#[test]
fn a_stale_translation_from_the_previous_refresh_is_named_by_the_epoch_check() {
    let mut f = Fake::new();
    f.fault = Fault::Stale;
    let (rep, _) = go(&small(), &mut f);
    assert!(matches!(rep.outcome, Outcome::Fail(_)));
    assert_eq!(state(&rep, "oracle_epoch"), CheckState::Fail);
    // The first refresh is clean: the staleness exists only from the second on.
    assert_eq!(rep.refreshes.len(), 3);
}

#[test]
fn a_pte_that_misses_the_live_page_fails_only_the_live_check() {
    let mut f = Fake::new();
    f.fault = Fault::IgnoreRewrite;
    let (rep, _) = go(&small(), &mut f);
    assert!(matches!(rep.outcome, Outcome::Fail(_)));
    assert_eq!(state(&rep, "oracle_live"), CheckState::Fail);
    assert_eq!(state(&rep, "oracle_pages"), CheckState::Pass);
}

#[test]
fn a_gpu_that_returns_zeros_fails() {
    let mut f = Fake::new();
    f.fault = Fault::Zeros;
    let (rep, _) = go(&small(), &mut f);
    assert!(matches!(rep.outcome, Outcome::Fail(_)));
    assert_eq!(state(&rep, "oracle_pages"), CheckState::Fail);
}

#[test]
fn a_refused_map_fails_and_everything_pinned_is_released() {
    let mut f = Fake::new();
    f.fault = Fault::MapRefused(2);
    let (rep, lines) = go(&small(), &mut f);
    match &rep.outcome {
        Outcome::Fail(w) => assert!(w.starts_with("map:"), "{w}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(f.count("copy "), 0, "no oracle on a failed map");
    assert_eq!(f.count("free "), 4, "all four descriptors freed");
    assert_eq!(f.count("unmap "), 2, "the two mapped batches unmapped");
    assert_eq!(f.teardown_calls, 1);
    assert!(has(&lines, "RUNG_refresh_stress=FAIL why=map:"));
}

#[test]
fn the_unmap_postcondition_is_checked_when_the_backend_can_and_skipped_when_it_cannot() {
    let mut f = Fake::new();
    f.fault = Fault::LeakMapping;
    let (rep, _) = go(&small(), &mut f);
    assert_eq!(state(&rep, "unmap_postcondition"), CheckState::Fail);
    assert!(matches!(rep.outcome, Outcome::Fail(_)));

    let mut f = Fake::new();
    f.probe = Probe::Unsupported;
    let (rep, lines) = go(&small(), &mut f);
    assert_eq!(
        rep.outcome,
        Outcome::Pass,
        "a missing verb does not fail the run"
    );
    assert_eq!(state(&rep, "unmap_postcondition"), CheckState::Skip);
    assert!(
        has(
            &lines,
            "CHECK unmap_postcondition SKIP probe_va-verb-missing"
        ),
        "{lines:#?}"
    );
    assert!(
        has(&lines, "STRESS_SUMMARY")
            && lines.iter().any(|l| l.contains("postcondition=unchecked"))
    );
}

#[test]
fn the_soft_deadline_stops_new_refreshes_and_two_completed_still_pass() {
    // Each refresh costs ~20 s of simulated time (4 maps of 5 s); the deadline is 50 s. Refresh 0 and 1
    // complete (about 40 s), refresh 2 starts and is cut at a batch boundary once 50 s have passed.
    let mut f = Fake::new();
    f.costs = Costs {
        map: 5_000_000_000,
        ..Costs::default()
    };
    let (rep, _) = go(&small(), &mut f);
    assert_eq!(rep.refreshes.len(), 2, "{:?}", rep.refreshes.len());
    assert_eq!(rep.outcome, Outcome::Pass, "{:?}", rep.checks);
    assert_eq!(state(&rep, "refreshes_completed"), CheckState::Pass);
    assert_eq!(f.teardown_calls, 1);
    assert_eq!(
        f.count("free "),
        12,
        "the cut refresh still released its descriptors"
    );
    assert_eq!(
        f.count("unmap "),
        10,
        "two completed refreshes, plus the two batches mapped when the cut came"
    );
}

#[test]
fn fewer_than_two_completed_refreshes_is_a_slow_failure() {
    // Maps cost 20 s each: refresh 0 is cut after two batches.
    let mut f = Fake::new();
    f.costs = Costs {
        map: 20_000_000_000,
        ..Costs::default()
    };
    let (rep, _) = go(&small(), &mut f);
    assert_eq!(rep.refreshes.len(), 0);
    assert!(matches!(rep.outcome, Outcome::Fail(_)));
    assert_eq!(state(&rep, "refreshes_completed"), CheckState::Fail);
    assert_eq!(f.count("free "), 4, "released after the cut");
    assert_eq!(
        f.count("unmap "),
        3,
        "the three batches mapped before the cut"
    );
}

#[test]
fn a_single_call_over_the_phase_limit_is_a_stall_and_over_two_seconds_is_only_counted() {
    let mut f = Fake::new();
    f.costs = Costs {
        inval: 11_000_000_000,
        ..Costs::default()
    };
    let p = StressParams {
        deadline_s: 600,
        hard_cap_s: 900,
        ..small()
    };
    let (rep, _) = go(&p, &mut f);
    assert_eq!(state(&rep, "no_stall"), CheckState::Fail);
    assert!(rep.stall_over_2s >= 3);
    // 3 s calls: over the 2 s TDR reference (counted) but under the 10 s stall limit (not a failure).
    let mut f = Fake::new();
    f.costs = Costs {
        inval: 3_000_000_000,
        ..Costs::default()
    };
    let (rep, lines) = go(&p, &mut f);
    assert_eq!(rep.outcome, Outcome::Pass);
    assert_eq!(rep.stall_over_2s, 3);
    assert!(lines.iter().any(|l| l.contains("stall_over_2s=3")));
}

#[test]
fn the_sliced_shape_records_every_page_call_in_a_bounded_histogram() {
    let mut f = Fake::new();
    let p = StressParams {
        shape: Shape::Sliced,
        ..small()
    };
    let (rep, lines) = go(&p, &mut f);
    assert_eq!(rep.outcome, Outcome::Pass);
    // 4 batches x 16 pages x 3 refreshes.
    assert!(
        has(&lines, "STRESS_HIST op=map n=192 "),
        "{:?}",
        lines.iter().find(|l| l.contains("op=map"))
    );
    assert!(
        lines.len() < 100,
        "output stays bounded ({} lines)",
        lines.len()
    );
}

#[derive(Default)]
struct Recorder(RefCell<Vec<(&'static str, u32, u32)>>);

impl Progress for Recorder {
    fn enter(&self, phase: &'static str, refresh: u32, batch: u32) {
        self.0.borrow_mut().push((phase, refresh, batch));
    }
}

#[test]
fn the_arm_reports_every_phase_so_the_watchdog_can_name_where_it_hung() {
    let mut f = Fake::new();
    let rec = Recorder::default();
    let (rep, _) = go_with(&small(), &mut f, &rec);
    assert_eq!(rep.outcome, Outcome::Pass);
    let seen = rec.0.borrow();
    for phase in [
        "facts",
        "plan",
        "setup",
        "pressure",
        "alloc",
        "touch",
        "census",
        "pin",
        "map",
        "invalidate",
        "oracle",
        "oracle-live",
        "unmap",
        "postcondition",
        "free",
    ] {
        assert!(
            seen.iter().any(|s| s.0 == phase),
            "phase {phase} never reported"
        );
    }
    assert!(
        seen.contains(&("map", 2, 3)),
        "refresh and batch indices are carried"
    );
}

#[test]
fn memory_is_released_between_refreshes_so_the_peak_does_not_accumulate() {
    let mut f = Fake::new();
    let (rep, _) = go(&small(), &mut f);
    assert_eq!(rep.outcome, Outcome::Pass);
    // alloc and release alternate: never two workloads alive at once.
    let mut live = 0i32;
    for l in &f.log {
        if l.starts_with("alloc_workload") {
            live += 1;
            assert!(live <= 1, "two workloads alive");
        }
        if l == "release_workload" {
            live -= 1;
        }
    }
    assert_eq!(live, 0);
}
