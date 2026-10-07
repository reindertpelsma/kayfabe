#!/usr/bin/env python3
"""Bite harness for the event-notification rung (`0x20800301`) and the bridge-refusal
instrument.

★★★ Every mutation here is chosen to **COMPILE**. A mutation that breaks compilation is
not a bite — the test never ran, so nothing was shown to be load-bearing. That mistake bit
three agents on 2026-07-31 and it is the reason this harness reports `COMPILE-FAIL` as a
distinct, *failing* outcome rather than folding it into "bit".

★★ Files are restored with an explicit `os.utime` bump. `shutil.copy2`/`cp -a` preserve the
OLD mtime, so cargo serves a stale rlib and every later mutation reads as a non-biter —
measured twice on 2026-07-28.

Usage:  python3 scripts/bite_event_notify.py [--cargo-target DIR]
"""

import argparse
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

INITTABLES = "crates/kayfabe-device/src/inittables.rs"
EVENTNOTIFY = "crates/kayfabe-abi/src/eventnotify.rs"
POLICY = "crates/kayfabe-rmrpc/src/policy.rs"

# (name, file, old, new, test-target, expected-test)
#
# `expected-test` is the test the mutation MUST kill. A mutation that turns some other
# test red is not evidence about the behaviour it was planted to probe, so the harness
# checks the name rather than merely the exit status.
MUTATIONS = [
    (
        "reply body emptied (the `inert` answer)",
        INITTABLES,
        "                eventnotify::encode_event_set_notification(&reg)",
        "                vec![0u8; eventnotify::EVENT_SET_NOTIFICATION_PARAMS_SIZE]",
        "event_set_notification",
        "the_reply_carries_the_registration_back_because_an_empty_body_would_rearm_notifier_zero",
    ),
    (
        "reply reflects the request instead of re-encoding it",
        INITTABLES,
        "                eventnotify::encode_event_set_notification(&reg)",
        "                cmd.payload[at..at + eventnotify::EVENT_SET_NOTIFICATION_PARAMS_SIZE].to_vec()",
        "event_set_notification",
        "the_reply_is_re_encoded_and_not_reflected_so_the_pad_runs_come_back_zero",
    ),
    (
        "SILENT_NOTIFIERS scope dropped (any legal notifier armed)",
        INITTABLES,
        "                if !eventnotify::is_silent_notifier(reg.event) {\n                    return refuse();\n                }",
        "                if false {\n                    return refuse();\n                }",
        "event_set_notification",
        "a_legal_notifier_this_device_cannot_promise_silence_for_is_refused",
    ),
    (
        "already-armed transition rule dropped",
        INITTABLES,
        "                if reg.action != eventnotify::ACTION_DISABLE\n                    && *slot != eventnotify::ACTION_DISABLE as u8\n                {\n                    return refuse();\n                }",
        "                if false {\n                    return refuse();\n                }",
        "event_set_notification",
        "arming_an_already_armed_notifier_is_refused_the_way_rm_refuses_it",
    ),
    (
        "the arming is never recorded",
        INITTABLES,
        "                *slot = u8::try_from(reg.action).unwrap_or(0);",
        "                let _ = &slot;",
        "event_set_notification",
        "arming_an_already_armed_notifier_is_refused_the_way_rm_refuses_it",
    ),
    (
        "DISABLE does not clear the slot",
        INITTABLES,
        "                *slot = u8::try_from(reg.action).unwrap_or(0);",
        "                if reg.action != eventnotify::ACTION_DISABLE {\n                    *slot = u8::try_from(reg.action).unwrap_or(0);\n                }",
        "event_set_notification",
        "disabling_is_always_legal_and_re_arms_the_notifier_for_a_later_registration",
    ),
    (
        "NV2080_NOTIFIERS_MAXCOUNT bound relaxed to `>`",
        EVENTNOTIFY,
        "    if event >= NV2080_NOTIFIERS_MAXCOUNT {",
        "    if event > NV2080_NOTIFIERS_MAXCOUNT {",
        "event_set_notification",
        "the_decoder_enforces_the_two_bounds_the_guests_own_handler_enforces",
    ),
    (
        "the TIMER notifier check dropped",
        EVENTNOTIFY,
        "    if event == NV2080_NOTIFIERS_TIMER {\n        return Err(EventNotifyError::TimerEvent);\n    }",
        "    if false {\n        return Err(EventNotifyError::TimerEvent);\n    }",
        "event_set_notification",
        "the_decoder_enforces_the_two_bounds_the_guests_own_handler_enforces",
    ),
    (
        "the action allowlist dropped",
        EVENTNOTIFY,
        "    if !matches!(action, ACTION_DISABLE | ACTION_SINGLE | ACTION_REPEAT) {",
        "    if false {",
        "event_set_notification",
        "an_action_outside_the_three_the_sdk_names_is_refused",
    ),
    (
        "short params read past instead of refused",
        EVENTNOTIFY,
        "    if params.len() < EVENT_SET_NOTIFICATION_PARAMS_SIZE {",
        "    if params.len() < 4 {",
        "event_set_notification",
        "a_short_params_struct_is_refused_rather_than_read_past",
    ),
    (
        "info32/info16/bNotifyState dropped from the reply",
        EVENTNOTIFY,
        "    params[NOTIFY_STATE_OFF] = u8::from(reg.notify_state);",
        "    params[NOTIFY_STATE_OFF] = 0;",
        "event_set_notification",
        "a_registrations_client_supplied_info_fields_survive_the_round_trip",
    ),
    (
        "the bridge refusal is never recorded (the instrument)",
        POLICY,
        "            Err(r) => self.census.record(r),",
        "            Err(_r) => {}",
        "gsp_rm_alloc",
        "the_refusal_census_is_readable_through_a_handle_after_the_policy_is_boxed",
    ),
]


def run(cmd, env, cwd=ROOT):
    return subprocess.run(
        cmd, shell=True, cwd=cwd, env=env, capture_output=True, text=True
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cargo-target", default=os.environ.get("CARGO_TARGET_DIR"))
    args = ap.parse_args()

    env = dict(os.environ)
    if args.cargo_target:
        env["CARGO_TARGET_DIR"] = args.cargo_target

    # Baseline: every expected test must be GREEN before any mutation, or a red result
    # below says nothing.
    targets = sorted({m[4] for m in MUTATIONS})
    for t in targets:
        r = run(f"cargo test --no-fail-fast --test {t}", env)
        if r.returncode != 0:
            print(f"★★★ BASELINE RED for {t} — the harness cannot measure anything.")
            print(r.stdout[-4000:])
            return 2
    print(f"baseline green for {len(targets)} target(s): {', '.join(targets)}\n")

    results = []
    for name, path, old, new, target, expected in MUTATIONS:
        full = os.path.join(ROOT, path)
        with open(full) as f:
            src = f.read()
        n = src.count(old)
        if n != 1:
            results.append((name, f"ANCHOR x{n}", ""))
            print(f"  ⊘ {name}: anchor matched {n} times, expected 1")
            continue
        try:
            with open(full, "w") as f:
                f.write(src.replace(old, new, 1))
            os.utime(full, (time.time(), time.time()))
            r = run(f"cargo test --no-fail-fast --test {target}", env)
            out = r.stdout + r.stderr
            if "error[E" in out or "error: could not compile" in out:
                verdict = "COMPILE-FAIL"
                detail = "not a bite: the test never ran"
            elif r.returncode == 0:
                verdict = "SURVIVED"
                detail = "nothing red — the behaviour is NOT covered"
            elif f"{expected} ... FAILED" in out or f"---- {expected} " in out:
                verdict = "BIT"
                detail = expected
            else:
                verdict = "BIT-ELSEWHERE"
                detail = "red, but not the test this probes"
            results.append((name, verdict, detail))
            print(f"  {verdict:14s} {name}" + (f"  [{detail}]" if detail else ""))
        finally:
            with open(full, "w") as f:
                f.write(src)
            os.utime(full, (time.time(), time.time()))

    print("\n" + "=" * 78)
    bit = sum(1 for _, v, _ in results if v == "BIT")
    print(f"BITES: {bit}/{len(MUTATIONS)}")
    bad = [(n, v) for n, v, _ in results if v != "BIT"]
    for n, v in bad:
        print(f"  ⊘ {v}: {n}")
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
