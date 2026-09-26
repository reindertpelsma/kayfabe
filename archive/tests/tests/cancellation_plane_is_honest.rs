//! ★★★★★ **w306 — THE CANCELLATION PLANE'S DISCIPLINE, AS A GATE.**
//!
//! ## The defect this generalises
//!
//! `docs/audits/w301_cancellation_error_leaks.md` §1.4 found `NVA06C_CTRL_CMD_PREEMPT`
//! (`0xa06c0105`) answered `NV_OK` with the guest's own bytes echoed back, unconditionally,
//! on a channel group that may have had a live scheduled host twin executing on a real
//! GA106. Nothing was preempted. w303 fixed **that row** (`preempt_is_decided.rs`).
//!
//! ⊘ **But the finding was never that a row was wrong.** It was that membership in
//! `INPUT_ONLY_CONTROLS` carried **no reason**, so a row that had stopped being true looked
//! exactly like one that never was — and it passed review for two days. Fixing one row does
//! not close that; a table that a *new* cancellation verb can be added to does.
//!
//! ## What is gated here
//!
//! `kayfabe_abi::submit::CANCELLATION_VERBS` states, per verb, what a caller is entitled to
//! believe after an `NV_OK` ([`CancelPromise`]), where that promise is written down, who
//! performs the verb on GA106+GSP, the status honesty requires when we cannot perform it,
//! and the measured arrival census. This file asserts the table's own rules:
//!
//! | test | what breaking it would mean |
//! |---|---|
//! | ★ [`no_cancellation_verb_is_answered_by_an_input_only_echo`] | the w301 defect, re-made for another id |
//! | ★ [`no_cancellation_verb_can_be_unperformed_and_still_report_success`] | *"accepted and dropped"* expressible as data |
//! | [`every_row_cites_ogkm_and_carries_an_arrival_census`] | a promise somebody remembered |
//! | [`the_table_covers_every_verb_measured_arriving`] | a verb the guest asks for that our census cannot see |
//! | [`the_table_is_not_degenerate`] | the gates above passing because the table says nothing |
//!
//! ## ★ THE KNOWN-POSITIVE, WATCHED FAILING
//!
//! `0xa06c0105` was a row of `INPUT_ONLY_CONTROLS` at `91f8b34b`, which is exactly the state
//! [`no_cancellation_verb_is_answered_by_an_input_only_echo`] forbids. Re-inserting it
//! reproduces the failure by name, verified before this file was committed:
//!
//! ```text
//! ---- no_cancellation_verb_is_answered_by_an_input_only_echo stdout ----
//! thread 'no_cancellation_verb_is_answered_by_an_input_only_echo' panicked at
//! tests/tests/cancellation_plane_is_honest.rs:
//! ★★★★★ FORGED COMPLETION. 0xa06c0105 NVA06C_CTRL_CMD_PREEMPT is a cancellation verb
//! promising `NotRunning`, and it is in INPUT_ONLY_CONTROLS — the path that answers NV_OK
//! with the caller's own bytes, unconditionally, without looking at any state. That is the
//! w301 §1.4 defect exactly.
//! ```
//!
//! ⊘ *A gate nobody has seen fail is not a gate*, and this campaign has shipped several that
//! could not.
//!
//! ### ★★★ …and the SECOND injection is the one that shows this is not a duplicate
//!
//! `preempt_is_decided.rs::preempt_left_the_input_only_table_and_is_still_claimed` already
//! asserts the same thing **for `0xa06c0105`**. So the injection above proves the gate fires,
//! but not that it is worth having. The one that does is a **different** verb — inserting
//! `0xa06f0112` (`STOP_CHANNEL`) into `INPUT_ONLY_CONTROLS` instead:
//!
//! ```text
//! === w303's gate (id-specific — GREEN, i.e. BLIND to this):
//! test result: ok. 6 passed; 0 failed
//! === w306's gate (RED):
//! ★★★★★ FORGED COMPLETION. 0xa06f0112 NVA06F_CTRL_CMD_STOP_CHANNEL is a cancellation verb
//! promising `NotRunning`, and it is in INPUT_ONLY_CONTROLS — …
//! test result: FAILED. 4 passed; 1 failed
//! ```
//!
//! ⇒ **The existing suite is green while the new forgery is present.** That is the whole
//! argument for this file: w303 fixed a row; this quantifies over the class, including verbs
//! nobody has written an arm for yet.
//!
//! ## ⊘ What this file does NOT claim
//!
//! **Nothing here cancels anything, and no verb's answer changed in w306.** Every row whose
//! `unperformed_status` is not `PREEMPT_UNPERFORMED_STATUS` is, today, still refused by the
//! ledger's blanket `0x56` — `docs/design/cancellation_plane.md` §1 is the record of what
//! each id is actually answered with, and §8 says which increments must not ship without a
//! boot. This file gates the *specification*, which is the thing that decayed last time.
//!
//! [`CancelPromise`]: kayfabe_abi::submit::CancelPromise

use kayfabe_abi::submit::{
    CANCELLATION_VERBS, CancelPromise, INPUT_ONLY_CONTROLS, PerformedBy, cancellation_verb,
    input_only_control,
};

/// ★★★★★ **THE GATE.** A verb that promises anything about host hardware state may never be
/// answered by the input-only echo.
///
/// `respond_input_only` performs a size check and returns `NV_OK` with the caller's own
/// bytes (`crates/kayfabe-rmrpc/src/policy.rs`), **without looking at any state at all**.
/// For a control whose whole content is a claim about a real GA106, that is a forged
/// completion in the strongest form §0 of `road_to_v1_after_cup2.md` names — *a completion
/// is sent only if the observed state after it is intended and safe in the guest*.
///
/// ★ Watched failing: see this file's module doc.
#[test]
fn no_cancellation_verb_is_answered_by_an_input_only_echo() {
    for v in CANCELLATION_VERBS {
        assert!(
            input_only_control(v.cmd).is_none(),
            "★★★★★ FORGED COMPLETION. {:#010x} {} is a cancellation verb promising {}, and \
             it is in INPUT_ONLY_CONTROLS — the path that answers NV_OK with the caller's \
             own bytes, unconditionally, without looking at any state. That is the w301 \
             §1.4 defect exactly. Either it forwards, or it decides, or it refuses by name; \
             it may not echo.",
            v.cmd,
            v.name,
            match v.promise {
                CancelPromise::NoNewWork(_) => "`NoNewWork`",
                CancelPromise::NotRunning(_) => "`NotRunning`",
            },
        );
    }

    // ⊘ NON-VACUITY, and it is required: the assertion above is a ZERO, and
    // `a_census_zero_needs_a_known_positive`. If `input_only_control` were broken, or the
    // table were empty, the loop would pass by looking at nothing.
    assert!(
        !CANCELLATION_VERBS.is_empty(),
        "the loop above quantified over an empty set"
    );
    assert!(
        input_only_control(0xa06c_0103).is_some(),
        "★ known-positive for the zero above — SET_TIMESLICE (0xa06c0103) is a genuine \
         input-only ack and a SIBLING SCHEDULER CONTROL of PREEMPT, so the lookup is live \
         and is being asked about the right region of the id space"
    );
    assert!(
        !INPUT_ONLY_CONTROLS.is_empty(),
        "…and the table it consults is populated"
    );
}

/// ★★★★ *"We accepted it and dropped it"* must not be expressible as data.
///
/// ⊘ `InputOnlyDisposition` closes this for the echo table by having exactly one variant.
/// The same discipline has to hold here, because a `unperformed_status` of `NV_OK` would be
/// precisely *"when we cannot do it, say we did"* — reviewable-looking, and wrong.
#[test]
fn no_cancellation_verb_can_be_unperformed_and_still_report_success() {
    for v in CANCELLATION_VERBS {
        assert_ne!(
            v.unperformed_status, 0,
            "{:#010x} {} declares NV_OK as its answer for NOT having performed the verb. \
             There is no such thing: an unperformed cancellation reported as success is the \
             lie w301 §1.4 found, written into the specification instead of the code.",
            v.cmd, v.name,
        );
    }
}

/// Every promise carries its citation, and every row carries a measured arrival count.
///
/// ★ Two different failures, deliberately in one test because they are the same rule: a
/// claim with no source is a claim somebody remembered (*citing the oracle is not the
/// oracle being right*), and a row with no census invites ranking by apparent severity
/// rather than *by what a guest can actually cause*.
#[test]
fn every_row_cites_ogkm_and_carries_an_arrival_census() {
    for v in CANCELLATION_VERBS {
        let promise = match v.promise {
            CancelPromise::NoNewWork(s) | CancelPromise::NotRunning(s) => s,
        };
        assert!(
            promise.len() > 80,
            "{:#010x} {} carries a promise string too short to be the contract's own words: \
             {promise:?}",
            v.cmd,
            v.name,
        );
        assert!(
            v.contract.contains("ogkm-580"),
            "{:#010x} {} has no ogkm-580 citation for its promise: {:?}. ⚠ The pinned host \
             driver is 580.159.04; a citation to any other tree is a citation to code this \
             port does not run against.",
            v.cmd,
            v.name,
            v.contract,
        );
        let hal = match v.performed_by {
            PerformedBy::GuestCpuRmThenUs(s) | PerformedBy::UsAlone(s) => s,
        };
        assert!(
            hal.contains("flags"),
            "{:#010x} {} states no export-table flags. ⚠ In ogkm the .c you read is often \
             not the code that runs — an unresolved HAL binding is the trap that killed two \
             prior leads on this exact plane. Citation was: {hal:?}",
            v.cmd,
            v.name,
        );
        assert!(
            v.arrivals.contains("measured") || v.arrivals.contains("crossing boot"),
            "{:#010x} {} has no measured arrival census: {:?}",
            v.cmd,
            v.name,
            v.arrivals,
        );
    }
}

/// ★★★ Every id measured arriving at us is in the table.
///
/// ⊘ The failure this closes is `our_census_counts_intent_the_driver_counts_attempts`: a
/// verb the guest actually asks for, that our own vocabulary has no name for, is invisible
/// to every later census — which is how `0x2080012c` (`GPU_EVICT_CTX`) stayed out of w301's
/// table while arriving in 115 boots.
#[test]
fn the_table_covers_every_verb_measured_arriving() {
    // `[measured 2026-08-14, w306]` over the 195 committed boot logs under
    // `traces/{boots,guest_boots,w294_cudalimit,w297_cup3,w298_ablation,w299_multiproc}`
    // that print an unserviced ledger, plus the two crossing boots' served-control census.
    for (cmd, ledgers, name) in [
        (0xa06f_0112u32, "115/195", "NVA06F_CTRL_CMD_STOP_CHANNEL"),
        (0x2080_012c, "115/195", "NV2080_CTRL_CMD_GPU_EVICT_CTX"),
        (
            0x906f_0102,
            "1/195 (w299 multiproc)",
            "NV906F_CTRL_CMD_RESET_CHANNEL",
        ),
        (
            0xa06f_0103,
            "both crossing boots",
            "NVA06F_CTRL_CMD_GPFIFO_SCHEDULE",
        ),
        (
            0xa06c_0101,
            "both crossing boots",
            "NVA06C_CTRL_CMD_GPFIFO_SCHEDULE",
        ),
        (
            0xa06c_0105,
            "native GA106, 1/run",
            "NVA06C_CTRL_CMD_PREEMPT",
        ),
    ] {
        assert!(
            cancellation_verb(cmd).is_some(),
            "{cmd:#010x} {name} is measured arriving ({ledgers}) and is NOT in \
             CANCELLATION_VERBS. A verb a guest asks for that our vocabulary cannot name is \
             invisible to every census built on that vocabulary — which is how \
             GPU_EVICT_CTX stayed out of w301's table while arriving in 115 boots.",
        );
    }

    // ⊘ Known-positive for the lookup itself: an id that is NOT a cancellation verb must
    // miss, or the test above would pass for any input.
    assert!(
        cancellation_verb(0x2080_012b).is_none(),
        "★ GPU_PROMOTE_CTX is EVICT_CTX's mirror and is not a cancellation verb; if the \
         lookup answered Some for it, the coverage assertions above would be vacuous"
    );
}

/// The table says something, in both directions.
///
/// ⊘ A table of all-`NotRunning` rows would make the first gate look strong while asserting
/// nothing about the distinction that matters, and a table with no served verb in it would
/// be a list of things we refuse rather than a description of the plane.
#[test]
fn the_table_is_not_degenerate() {
    let no_new_work = CANCELLATION_VERBS
        .iter()
        .filter(|v| matches!(v.promise, CancelPromise::NoNewWork(_)))
        .count();
    let not_running = CANCELLATION_VERBS
        .iter()
        .filter(|v| matches!(v.promise, CancelPromise::NotRunning(_)))
        .count();
    assert!(
        no_new_work >= 2 && not_running >= 2,
        "the promise split is degenerate: {no_new_work} NoNewWork, {not_running} \
         NotRunning. The whole point of the axis is that it CUTS the set — \
         cancellation_plane.md §2 ranks tier D above tier X on exactly this distinction"
    );

    let ours_alone = CANCELLATION_VERBS
        .iter()
        .filter(|v| matches!(v.performed_by, PerformedBy::UsAlone(_)))
        .count();
    assert!(
        ours_alone >= 2,
        "★ at least two verbs must be resolved as compiled-out-to-GSP. If none were, either \
         the HAL resolution was skipped or it was got wrong in the direction that makes the \
         plane look like somebody else's problem — and it is not: we ARE the GSP"
    );

    // ★ And the one row already answered by a DECISION rather than a refusal is still
    // there, still claimed, and still not an echo — the composition with w303.
    let preempt = cancellation_verb(0xa06c_0105).expect("PREEMPT is in the table");
    assert_eq!(
        preempt.unperformed_status,
        kayfabe_abi::submit::PREEMPT_UNPERFORMED_STATUS,
        "the table's declared refusal status for PREEMPT must be the one respond_preempt \
         actually answers, or the specification and the code have drifted apart while both \
         look reviewed"
    );
}
