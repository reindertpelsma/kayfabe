//! The gates of the refresh stress as a **pure function of the evidence**.
//!
//! `gates(evidence)` returns the named checks; the verdict is whether any failed and any passed
//! (`Ledger::passed`). Nothing here reads a clock, a file or a device, so every rule is unit-tested with
//! hand-made evidence, including evidence that a real run could only produce on a broken host.
//!
//! Gate numbering follows V3_RAW_CLIENT.md §4.7: G0 (qualified) is decided before any evidence exists
//! and returns NOTRUN; G1 is the oracle family; G2 is the stall check; G3 (the guest/native ratio) is the
//! compare step's, because one process cannot know the other run.

use super::calibration::MIN_COMPLETED_REFRESHES;
use super::oracle::OracleTally;
use crate::ledger::{Check, CheckState};

/// What the unmap postcondition found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostState {
    /// Every probed VA was reported free.
    Free {
        /// VAs probed.
        probed: u64,
    },
    /// At least one probed VA was still mapped.
    StillMapped {
        /// How many.
        count: u64,
    },
    /// The backend cannot probe; nothing was checked.
    Unchecked(&'static str),
}

/// Everything the gates need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    /// Refreshes that ran to the end.
    pub completed: u32,
    /// The oracle's counts over the whole run.
    pub tally: OracleTally,
    /// The unmap postcondition.
    pub post: PostState,
    /// The slowest single RM call or fence wait, ns.
    pub max_call_ns: u64,
    /// The stall limit, ns.
    pub stall_limit_ns: u64,
    /// Errors met while tearing down (unmap/free failures after the timed part).
    pub teardown_errors: u32,
}

fn check(name: &'static str, state: CheckState, why: String) -> Check {
    Check { name, state, why }
}

fn pf(pass: bool) -> CheckState {
    if pass {
        CheckState::Pass
    } else {
        CheckState::Fail
    }
}

/// The named checks, in the order they are printed.
#[must_use]
pub fn gates(e: &Evidence) -> Vec<Check> {
    let t = &e.tally;
    let mut v = Vec::with_capacity(8);
    v.push(check(
        "refreshes_completed",
        pf(e.completed >= MIN_COMPLETED_REFRESHES),
        format!(
            "completed={} need={MIN_COMPLETED_REFRESHES}{}",
            e.completed,
            if e.completed < MIN_COMPLETED_REFRESHES {
                " slow"
            } else {
                ""
            }
        ),
    ));
    v.push(check(
        "oracle_pages",
        pf(t.checked > 0 && t.mismatches == 0),
        match t.first {
            Some(m) => format!(
                "checked={} mismatches={} first_page={} first_word={} got={:#x} want={:#x}",
                t.checked, t.mismatches, m.page, m.word, m.got, m.want
            ),
            None => format!("checked={} mismatches=0", t.checked),
        },
    ));
    // The control: a comparison that cannot say "different" proves nothing.
    v.push(check(
        "oracle_decoy",
        pf(t.decoy_checked > 0 && t.decoy_tripped == t.decoy_checked),
        format!("tripped={}/{}", t.decoy_tripped, t.decoy_checked),
    ));
    v.push(if e.completed < 2 {
        check(
            "oracle_epoch",
            CheckState::Skip,
            "needs two completed refreshes".into(),
        )
    } else {
        check(
            "oracle_epoch",
            pf(t.stale == 0),
            format!("stale_pages={}", t.stale),
        )
    });
    v.push(check(
        "oracle_live",
        pf(t.live_checked > 0 && t.live_mismatches == 0),
        format!(
            "checked={} mismatches={}",
            t.live_checked, t.live_mismatches
        ),
    ));
    v.push(match &e.post {
        PostState::Free { probed } => check(
            "unmap_postcondition",
            CheckState::Pass,
            format!("free={probed}"),
        ),
        PostState::StillMapped { count } => check(
            "unmap_postcondition",
            CheckState::Fail,
            format!("still_mapped={count}"),
        ),
        PostState::Unchecked(why) => check("unmap_postcondition", CheckState::Skip, (*why).into()),
    });
    v.push(check(
        "no_stall",
        pf(e.max_call_ns <= e.stall_limit_ns),
        format!(
            "max_call_ms={} limit_ms={}",
            e.max_call_ns / 1_000_000,
            e.stall_limit_ns / 1_000_000
        ),
    ));
    v.push(check(
        "teardown_clean",
        pf(e.teardown_errors == 0),
        format!("errors={}", e.teardown_errors),
    ));
    v
}

/// The positive control for `RUNGCTL_`: the decoy comparison tripped every time.
#[must_use]
pub fn control_passed(checks: &[Check]) -> bool {
    checks
        .iter()
        .any(|c| c.name == "oracle_decoy" && c.state == CheckState::Pass)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> Evidence {
        Evidence {
            completed: 3,
            tally: OracleTally {
                checked: 4608,
                decoy_checked: 4608,
                decoy_tripped: 4608,
                live_checked: 96,
                ..OracleTally::default()
            },
            post: PostState::Free { probed: 72 },
            max_call_ns: 40_000_000,
            stall_limit_ns: 10_000_000_000,
            teardown_errors: 0,
        }
    }

    fn state(e: &Evidence, name: &str) -> CheckState {
        gates(e)
            .into_iter()
            .find(|c| c.name == name)
            .expect(name)
            .state
    }

    fn all_pass(e: &Evidence) -> bool {
        let g = gates(e);
        g.iter().all(|c| c.state != CheckState::Fail)
            && g.iter().any(|c| c.state == CheckState::Pass)
    }

    #[test]
    fn good_evidence_passes_every_gate() {
        let g = gates(&good());
        assert_eq!(g.len(), 8);
        assert!(g.iter().all(|c| c.state == CheckState::Pass), "{g:?}");
        assert!(control_passed(&g));
    }

    #[test]
    fn each_gate_fails_for_its_own_reason_only() {
        let cases: Vec<(&str, Evidence)> = vec![
            (
                "refreshes_completed",
                Evidence {
                    completed: 1,
                    ..good()
                },
            ),
            (
                "oracle_pages",
                Evidence {
                    tally: OracleTally {
                        mismatches: 1,
                        ..good().tally
                    },
                    ..good()
                },
            ),
            (
                "oracle_pages",
                Evidence {
                    tally: OracleTally {
                        checked: 0,
                        decoy_checked: 0,
                        ..good().tally
                    },
                    ..good()
                },
            ),
            (
                "oracle_decoy",
                Evidence {
                    tally: OracleTally {
                        decoy_tripped: 4607,
                        ..good().tally
                    },
                    ..good()
                },
            ),
            (
                "oracle_epoch",
                Evidence {
                    tally: OracleTally {
                        stale: 2,
                        mismatches: 2,
                        ..good().tally
                    },
                    ..good()
                },
            ),
            (
                "oracle_live",
                Evidence {
                    tally: OracleTally {
                        live_mismatches: 1,
                        ..good().tally
                    },
                    ..good()
                },
            ),
            (
                "oracle_live",
                Evidence {
                    tally: OracleTally {
                        live_checked: 0,
                        ..good().tally
                    },
                    ..good()
                },
            ),
            (
                "unmap_postcondition",
                Evidence {
                    post: PostState::StillMapped { count: 3 },
                    ..good()
                },
            ),
            (
                "no_stall",
                Evidence {
                    max_call_ns: 10_000_000_001,
                    ..good()
                },
            ),
            (
                "teardown_clean",
                Evidence {
                    teardown_errors: 1,
                    ..good()
                },
            ),
        ];
        for (name, e) in cases {
            assert_eq!(state(&e, name), CheckState::Fail, "{name}");
            assert!(!all_pass(&e), "{name} must make the verdict fail");
        }
    }

    #[test]
    fn the_stall_edge_is_inclusive_of_the_limit() {
        let e = Evidence {
            max_call_ns: 10_000_000_000,
            ..good()
        };
        assert_eq!(state(&e, "no_stall"), CheckState::Pass);
    }

    #[test]
    fn a_missing_probe_is_a_skip_never_a_pass() {
        let e = Evidence {
            post: PostState::Unchecked("probe_va-verb-missing"),
            ..good()
        };
        assert_eq!(state(&e, "unmap_postcondition"), CheckState::Skip);
        assert!(
            all_pass(&e),
            "a skip does not fail the run, and is visible in the ledger"
        );
    }

    #[test]
    fn one_refresh_cannot_make_the_epoch_check() {
        let e = Evidence {
            completed: 1,
            ..good()
        };
        assert_eq!(state(&e, "oracle_epoch"), CheckState::Skip);
        assert_eq!(state(&e, "refreshes_completed"), CheckState::Fail);
    }

    #[test]
    fn a_decoy_that_never_ran_fails_the_control() {
        let e = Evidence {
            tally: OracleTally {
                decoy_checked: 0,
                decoy_tripped: 0,
                ..good().tally
            },
            ..good()
        };
        assert!(!control_passed(&gates(&e)));
    }

    #[test]
    fn the_first_mismatch_is_reported_with_its_coordinates() {
        use crate::stress::oracle::Mismatch;
        let e = Evidence {
            tally: OracleTally {
                mismatches: 1,
                first: Some(Mismatch {
                    page: 77,
                    word: 3,
                    got: 0xAA,
                    want: 0xBB,
                }),
                ..good().tally
            },
            ..good()
        };
        let why = gates(&e)
            .into_iter()
            .find(|c| c.name == "oracle_pages")
            .unwrap()
            .why;
        assert!(
            why.contains("first_page=77 first_word=3 got=0xaa want=0xbb"),
            "{why}"
        );
    }
}
