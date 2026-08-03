#!/usr/bin/env python3
"""Bite harness for the GPFIFO-schedule rung (`0xa06f0103`, task #177).

★★★ **Why this harness matters more than usual here.** Every field of
`NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS` is `[IN]`, so there is no reply to get right and a
test that merely asserted `NV_OK` would pass against a policy that performed *nothing*.
The mutation this file exists for is `M1`: it deletes the doorbell gate, which is the only
thing that makes serving this control a **performed transition** rather than a word. If M1
survives, the rung is a fabricated promise and the whole increment should be reverted.

★★ Every mutation is chosen to **COMPILE**. A mutation that breaks compilation is not a
bite — the test never ran. `COMPILE-FAIL` is reported as its own failing outcome.

★★ Files are restored with an explicit `os.utime` bump: `shutil.copy2`/`cp -a` preserve the
OLD mtime, cargo then serves a stale rlib, and every later mutation reads as a non-biter.

⚠ **One mutation in this file was INERT on its first run, and it is recorded rather than
quietly fixed.** M9's first form uppercased `on purpose` inside the triage row's prose. It
changed bytes and SURVIVED — correctly, because the test asserts that the row still
*contains* `hVASpace = NV01_NULL_OBJECT`, and uppercasing the words after that substring
leaves the asserted property true. An inert mutation is not evidence of a gap in the test;
it is evidence that the mutation did not touch the behaviour under test. M9 now deletes the
substring the test actually names.

Usage:  python3 scripts/bite_gpfifo_schedule.py [--cargo-target DIR]
"""

import argparse
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

FWD = "crates/kayfabe-fwd/src/lib.rs"
SUBMIT = "crates/kayfabe-abi/src/submit.rs"
POLICY = "crates/kayfabe-rmrpc/src/policy.rs"
CORE = "crates/kayfabe-core/src/gpu.rs"
SWEEP = "crates/kayfabe-device/src/sweep.rs"

SCHED = "-p kayfabe-tests --test gpfifo_schedule"

# (name, file, old, new, cargo-args, expected-test)
MUTATIONS = [
    (
        "★★★ M1 — THE GATE IS DELETED: a doorbell on a channel the guest never "
        "scheduled is planned anyway, so the control performs nothing",
        FWD,
        "    if !proc.exec.requested.contains(&cid) {\n"
        "        return Err(FwdFault::NotScheduled {\n"
        "            chan: cid,\n"
        "            vchid: route.vchid,\n"
        "        });\n"
        "    }\n",
        "",
        SCHED,
        "a_doorbell_is_refused_before_the_control_and_planned_after_it",
    ),
    (
        "M2 — the gate is INVERTED (only unscheduled channels ring), which is the "
        "shape a copy-paste of the old memo would produce",
        FWD,
        "    if !proc.exec.requested.contains(&cid) {",
        "    if proc.exec.requested.contains(&cid) {",
        SCHED,
        "a_doorbell_is_refused_before_the_control_and_planned_after_it",
    ),
    (
        "M3 — the enabled-vs-scheduled split is SILENTLY SERVED: bSkipSubmit / "
        "bSkipEnable are ignored instead of refused",
        SUBMIT,
        "        self.b_skip_submit == 0 && self.b_skip_enable == 0",
        "        true",
        SCHED,
        "the_enabled_versus_scheduled_split_is_refused_by_name",
    ),
    (
        "M4 — the reply is ZERO-FILLED instead of echoing the request, which the GSP "
        "transport would copy over the caller's own bEnable",
        SUBMIT,
        "    let mut out = vec![0u8; GpfifoScheduleParams::SIZE];\n"
        "    req.encode_into(&mut out)\n"
        '        .expect("SIZE bytes is exactly what encode_into needs");\n'
        "    out",
        "    vec![0u8; GpfifoScheduleParams::SIZE]",
        SCHED,
        "the_scrubbers_own_request_is_served_with_the_bytes_hardware_sends",
    ),
    (
        "M5 — a decided refusal reverts to NV_ERR_NOT_SUPPORTED, i.e. becomes "
        "indistinguishable from 'nobody claimed this command' in the guest's dmesg",
        SUBMIT,
        "pub const GPFIFO_SCHEDULE_REFUSED_STATUS: u32 = 0x40;",
        "pub const GPFIFO_SCHEDULE_REFUSED_STATUS: u32 = 0x56;",
        SCHED,
        "a_decided_refusal_is_never_the_unclaimed_signature",
    ),
    (
        "★★ M6 — the control claim WIDENS to every control id, which would silence the "
        "unserviced ledger permanently (PolicyChain is a find_map)",
        POLICY,
        "        if !OBJECT_CONTROLS.contains(&req.cmd) {\n            return None;\n        }",
        "        if false {\n            return None;\n        }",
        SCHED,
        "every_other_control_is_still_declined_so_the_ledger_lives",
    ),
    (
        "M7 — bEnable = NV_FALSE stops withdrawing the declaration, so a channel the "
        "guest stopped keeps running",
        CORE,
        "        proc.exec.requested.remove(&route.chan)",
        "        proc.exec.requested.contains(&route.chan)",
        SCHED,
        "withdrawing_the_declaration_refuses_the_doorbell_again",
    ),
    (
        "M8 — a malformed NvBool (neither 0 nor 1) is accepted instead of refused",
        SUBMIT,
        "        if value > 1 {",
        "        if false {",
        SCHED,
        "a_byte_that_is_not_an_nvbool_is_a_decode_failure",
    ),
    (
        "M9 — the triage row's CORRECTION is deleted, taking with it the record of "
        "what is still false (the scrubber has no VAS)",
        SWEEP,
        "hVASpace = NV01_NULL_OBJECT on purpose",
        "no VAS is declared for it",
        SCHED,
        "the_triage_row_survives_and_records_the_correction",
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

    targets = sorted({m[4] for m in MUTATIONS})
    for t in targets:
        r = run(f"cargo test --no-fail-fast {t}", env)
        if r.returncode != 0:
            print(f"★★★ BASELINE RED for `{t}` — the harness cannot measure anything.")
            print((r.stdout + r.stderr)[-4000:])
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
            r = run(f"cargo test --no-fail-fast {target}", env)
            out = r.stdout + r.stderr
            if "error[E" in out or "error: could not compile" in out:
                verdict, detail = "COMPILE-FAIL", "not a bite: the test never ran"
            elif r.returncode == 0:
                verdict, detail = "SURVIVED", "nothing red — the behaviour is NOT covered"
            elif f"{expected} ... FAILED" in out or f"---- {expected} " in out:
                verdict, detail = "BIT", expected
            else:
                failed = [
                    ln.strip()
                    for ln in out.splitlines()
                    if ln.strip().startswith("---- ") and " stdout" in ln
                ]
                verdict = "BIT-ELSEWHERE"
                detail = f"expected {expected}; red were {failed}"
        finally:
            with open(full, "w") as f:
                f.write(src)
            os.utime(full, (time.time(), time.time()))
        results.append((name, verdict, detail))
        print(f"  {verdict:<14} {name}\n                 {detail}")

    bad = [r for r in results if r[1] not in ("BIT",)]
    print(f"\n{len(results) - len(bad)}/{len(results)} mutations BIT the test they name")
    if bad:
        print("★ not a clean sweep:")
        for n, v, d in bad:
            print(f"   {v:<14} {n} — {d}")
        return 1
    print("ALL MUTATIONS BIT")
    return 0


if __name__ == "__main__":
    sys.exit(main())
