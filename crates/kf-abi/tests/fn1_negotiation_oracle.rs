//! The fn-1 (`SET_GUEST_SYSTEM_INFO`) negotiation oracle: NVIDIA's own guest-side
//! `RmRpcSetGuestSystemInfo` (ogkm 595.84, compiled by `tests/oracle/build_fn1_oracle.sh`), asked what
//! the guest does for a grid of (guest pair, host pair, host behaviour).
//!
//! This file pins the ORACLE, not kayfabe: each assertion is a fact the real guest code has, and the
//! kayfabe comparison (`kf-rm/tests/fn1_negotiation_oracle.rs`) relies on them. If the oracle ever
//! stops saying one of these, the comparison is about a different protocol. No number below is typed
//! by hand: the minimum comes from `consts`, the pairs from the grid's own universe.
//!
//! Skips (with `FN1-ORACLE-GATE: SKIPPED ...`) when the ogkm tree is absent.

#[path = "support/fn1_oracle.rs"]
mod fn1_oracle;

use fn1_oracle::{BODY_LEN, GridRow, consts, grid, hexnum, oracle};

const NV_OK: u32 = 0;

fn model<'a>(g: &'a [GridRow], name: &'a str) -> impl Iterator<Item = &'a GridRow> {
    g.iter().filter(move |r| r.model == name)
}

fn min_supported(bin: &std::path::Path) -> (u32, u32) {
    let c = consts(bin);
    (
        hexnum(&c["min_supported_major"][0]),
        hexnum(&c["min_supported_minor"][0]),
    )
}

#[test]
fn the_oracle_states_the_tree_it_was_built_from() {
    let Some(bin) = oracle("the_oracle_states_the_tree_it_was_built_from") else {
        return;
    };
    let c = consts(&bin);
    assert_eq!(c["ogkm_version"], ["595.84"]);
    assert_eq!(c["body_len"][0].parse::<usize>().unwrap(), BODY_LEN);
    assert!(c["map"].len() > 20, "the tree's internal->external table");
}

#[test]
fn an_equal_pair_is_accepted_in_one_exchange() {
    let Some(bin) = oracle("an_equal_pair_is_accepted_in_one_exchange") else {
        return;
    };
    let g = grid(&bin);
    let mut n = 0;
    for r in model(&g, "steer-to-host").filter(|r| r.guest == r.host) {
        n += 1;
        assert!(r.done.accepted(), "{r:?}");
        assert_eq!(r.done.issued, 1, "{r:?}");
        assert_eq!(r.done.agreed, Some(r.host), "{r:?}");
        assert_eq!(r.done.ext_calls, 1, "the fn-64 tail call follows: {r:?}");
        assert!(!r.done.retry_log, "{r:?}");
    }
    assert!(n > 20, "the universe has many pairs");
}

#[test]
fn a_host_that_steers_is_followed_once_if_its_pair_is_at_least_the_minimum_in_either_direction() {
    let Some(bin) = oracle("a_host_that_steers_is_followed_once") else {
        return;
    };
    let min = min_supported(&bin);
    let g = grid(&bin);
    let (mut older_guest, mut newer_guest, mut too_old) = (0, 0, 0);
    for r in model(&g, "steer-to-host").filter(|r| r.guest != r.host) {
        if r.host >= min {
            // ★ The guest code never compares host with guest: any host pair >= the minimum is followed.
            assert!(r.done.accepted(), "{r:?}");
            assert_eq!(r.done.issued, 2, "retried exactly once: {r:?}");
            assert_eq!(r.done.agreed, Some(r.host), "agreed = the HOST's pair: {r:?}");
            assert!(r.done.retry_log && !r.done.too_old_log, "{r:?}");
            assert_eq!(r.done.ext_calls, 1, "{r:?}");
            if r.guest < r.host {
                older_guest += 1;
            } else {
                newer_guest += 1;
            }
        } else {
            too_old += 1;
            assert_ne!(r.done.status, NV_OK, "{r:?}");
            assert_eq!(r.done.issued, 1, "no retry below the minimum: {r:?}");
            assert!(r.done.too_old_log && !r.done.retry_log, "{r:?}");
            assert_eq!(r.done.agreed, None, "{r:?}");
            assert_eq!(r.done.ext_calls, 0, "{r:?}");
            // The status the guest ends with is the host's private status, verbatim.
            assert_eq!(
                r.done.status,
                hexnum(&consts(&bin)["result_guest_host_driver_mismatch"][0]),
                "{r:?}"
            );
        }
    }
    assert!(older_guest > 100 && newer_guest > 100 && too_old > 10);
}

#[test]
fn a_non_zero_rpc_result_is_a_hard_stop_not_a_negotiation() {
    let Some(bin) = oracle("a_non_zero_rpc_result_is_a_hard_stop") else {
        return;
    };
    let c = consts(&bin);
    let not_supported = hexnum(&c["result_not_supported"][0]);
    let g = grid(&bin);
    for r in model(&g, "hard-fail-mismatch") {
        if r.guest == r.host {
            assert!(r.done.accepted(), "{r:?}");
        } else {
            // This is what kayfabe does today for a pair mismatch: the guest gets the status verbatim,
            // does not retry, and never learns the host's pair.
            assert_eq!(r.done.status, not_supported, "{r:?}");
            assert_eq!(r.done.issued, 1, "{r:?}");
            assert_eq!(r.done.agreed, None, "{r:?}");
        }
    }
}

#[test]
fn a_private_status_with_the_guests_own_pair_in_the_body_is_not_retried() {
    let Some(bin) = oracle("a_private_status_with_the_guests_own_pair") else {
        return;
    };
    let g = grid(&bin);
    for r in model(&g, "steer-always-private").filter(|r| r.guest == r.host) {
        // The retry rule keys on the body's pair DIFFERING from the guest's own: a private error that
        // names the same pair is final.
        assert_ne!(r.done.status, NV_OK, "{r:?}");
        assert_eq!(r.done.issued, 1, "{r:?}");
        assert_eq!(r.done.agreed, None, "{r:?}");
    }
}

#[test]
fn a_host_that_accepts_the_guest_pair_leaves_the_guest_on_its_own_pair_whatever_it_is() {
    let Some(bin) = oracle("a_host_that_accepts_the_guest_pair") else {
        return;
    };
    let g = grid(&bin);
    for r in model(&g, "accept-guest") {
        // OK / OK with the body unchanged: the guest stores what the REPLY says, and the minimum is
        // never consulted (it is only a retry-rule input).
        assert!(r.done.accepted(), "{r:?}");
        assert_eq!(r.done.agreed, Some(r.guest), "{r:?}");
        assert_eq!(r.done.issued, 1, "{r:?}");
    }
}

#[test]
fn the_agreed_pair_is_what_selects_the_rpc_version_table() {
    let Some(bin) = oracle("the_agreed_pair_is_what_selects_the_rpc_version_table") else {
        return;
    };
    let g = grid(&bin);
    for r in model(&g, "steer-to-host").filter(|r| r.done.accepted()) {
        let (major, minor) = r.done.agreed.unwrap();
        // RPC_VERSION_FROM_VGX_VERSION: major in 31:24, minor in 23:16.
        assert_eq!(r.done.ip_version, Some(major << 24 | minor << 16), "{r:?}");
    }
}
