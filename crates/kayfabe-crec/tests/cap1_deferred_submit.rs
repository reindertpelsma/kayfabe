//! ★★★★★ **w395 — THE DEFERRAL IS OBSERVATIONALLY EQUIVALENT, measured over 359 062
//! records.**
//!
//! `cap1_coldboot_hermetic` replayed twice against the same GSP: once with the command ring
//! serviced INSIDE the `QUEUE_HEAD` store (`SubmitMode::Inline`, the control) and once with
//! the store VALIDATING AND RETURNING and the service run as a separate
//! `GspFsm::service_deferred` call immediately after (`SubmitMode::Deferred`, as the shell's
//! worker does). Every projected guest-RAM write, every command, the final phase and the
//! closure limit must be identical — the deferral changes **where** the service runs and
//! nothing the guest can see.
//!
//! ⊘ What this cannot say: anything about the worker RACING the guest. A capture is a total
//! order; only a live boot exercises the interleaving. It also cannot see the E8 stale-binding
//! refusal fire, because `cap1` never rings after its teardown — `tests/gsp_submit_schedule.rs`
//! in `kayfabe-qemu-raw` covers that at the register plane.

use kayfabe_crec::{CTrace, Fill, Replay, bench_abi, cap1_path, load_cap1};
use kayfabe_gsp::{BootPhase, Transition};

fn cap1() -> CTrace {
    match load_cap1() {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => panic!("cap1 at {:?} did not decode: {e:?}", cap1_path()),
        Err(e) => panic!("cap1 is missing at {:?} ({e})", cap1_path()),
    }
}

#[test]
fn deferring_the_submit_changes_nothing_the_guest_can_observe() {
    let t = cap1();
    let inline = Replay::new(&t, bench_abi()).run(Fill::Reconstructed);
    let deferred = Replay::new(&t, bench_abi())
        .with_deferred_submit()
        .run(Fill::Reconstructed);

    // ★ Non-vacuity: both arms must actually reach a serviced doorbell and the bind, or the
    // equality below is two empty runs agreeing.
    for want in [Transition::E6, Transition::E7, Transition::E12] {
        assert!(inline.transitions_seen.contains(&want), "control never fired {want:?}");
        assert!(
            deferred.transitions_seen.contains(&want),
            "deferred arm never fired {want:?}"
        );
    }
    assert!(
        !inline.commands.is_empty(),
        "the control answered no command; the comparison would be vacuous"
    );

    assert_eq!(
        inline.rust.events, deferred.rust.events,
        "the projected guest-RAM writes (and the IRQ announcements) must be byte-identical: \
         the deferral moves the service, it does not change it"
    );
    assert_eq!(
        inline.commands.len(),
        deferred.commands.len(),
        "the same number of commands must be answered"
    );
    for ((ti, ci), (td, cd)) in inline.commands.iter().zip(deferred.commands.iter()) {
        assert_eq!(ti, td, "a command was answered in a different transaction");
        assert_eq!(ci.function, cd.function, "a command's function id differs");
    }
    assert_eq!(inline.final_phase, deferred.final_phase);
    assert_eq!(inline.final_phase, BootPhase::Halted);
    assert_eq!(inline.closure_limit, deferred.closure_limit);
    assert_eq!(inline.txns.len(), deferred.txns.len());
}

/// The deferred arm must never leave a service OWED at the end of a transaction — the
/// replay services immediately, so a leftover owed flag would mean a store that produced
/// `service_owed` was not followed by a pass, i.e. the merge in `Replay::run_once` skipped it.
#[test]
fn the_deferred_arm_answers_every_command_the_control_did_in_the_same_transaction() {
    let t = cap1();
    let inline = Replay::new(&t, bench_abi()).run(Fill::Reconstructed);
    let deferred = Replay::new(&t, bench_abi())
        .with_deferred_submit()
        .run(Fill::Reconstructed);
    let by_txn = |r: &kayfabe_crec::ReplayResult| {
        r.commands
            .iter()
            .map(|(txn, c)| (*txn, c.function))
            .collect::<Vec<_>>()
    };
    assert_eq!(by_txn(&inline), by_txn(&deferred));
}
