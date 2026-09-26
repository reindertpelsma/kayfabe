#!/usr/bin/env python3
"""Bite harness for the L2-invalidate/evict rung (`0x20800a6c`).

★★★ Every mutation here is chosen to **COMPILE**. A mutation that breaks compilation is
not a bite — the test never ran, so nothing was shown to be load-bearing. The harness
reports `COMPILE-FAIL` as a distinct, *failing* outcome rather than folding it into "bit".

★★ Files are restored with an explicit `os.utime` bump. `shutil.copy2`/`cp -a` preserve the
OLD mtime, so cargo serves a stale rlib and every later mutation reads as a non-biter —
measured twice on 2026-07-28.

⚠ Unlike `bite_event_notify.py` this harness carries a full **cargo argument string** per
mutation rather than a bare `--test NAME`. Two of this rung's load-bearing properties live
in `kayfabe-abi`'s *lib* unit tests — the flag allowlist and the four-zero reply — and a
`--test` target can never reach them, so a harness that could only name integration targets
would have reported them uncovered when they are not.

Usage:  python3 scripts/bite_l2_evict.py [--cargo-target DIR]
"""

import argparse
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

INITTABLES = "crates/kayfabe-device/src/inittables.rs"
L2EVICT = "crates/kayfabe-abi/src/l2evict.rs"
SWEEP = "crates/kayfabe-device/src/sweep.rs"

DEVICE = "-p kayfabe-device --test l2_invalidate_evict"
ABI = "-p kayfabe-abi --lib"
TRIAGE = "-p kayfabe-device --test sweep_triage"

# (name, file, old, new, cargo-args, expected-test)
#
# `expected-test` is the test the mutation MUST kill. A mutation that turns some other test
# red is not evidence about the behaviour it was planted to probe, so the harness checks
# the name rather than merely the exit status.
MUTATIONS = [
    (
        "the reply ECHOES the request's flags instead of answering four zeros",
        L2EVICT,
        "    vec![0u8; L2_INVALIDATE_EVICT_PARAMS_SIZE]\n}",
        "    _req.flags.to_le_bytes().to_vec()\n}",
        DEVICE,
        "the_reply_is_four_zeros_which_is_the_opposite_of_the_event_control",
    ),
    (
        "the reply ECHOES the request's flags (the abi round trip)",
        L2EVICT,
        "    vec![0u8; L2_INVALIDATE_EVICT_PARAMS_SIZE]\n}",
        "    _req.flags.to_le_bytes().to_vec()\n}",
        ABI,
        "the_reply_is_four_zeros_and_carries_no_byte_of_the_request",
    ),
    (
        "the reply body is EMPTIED (the `inert` answer, which is wrong here for a "
        "different reason than for 0x20800301)",
        L2EVICT,
        "    vec![0u8; L2_INVALIDATE_EVICT_PARAMS_SIZE]\n}",
        "    Vec::new()\n}",
        DEVICE,
        "the_evict_kbusverifybar2_actually_sends_is_served",
    ),
    (
        "the unknown-flag refusal dropped — a generic NV_OK for any bit pattern",
        L2EVICT,
        "    if unknown != 0 {\n        return Err(L2EvictError::UnknownFlags { flags, unknown });\n    }",
        "    if false {\n        return Err(L2EvictError::UnknownFlags { flags, unknown });\n    }",
        DEVICE,
        "a_flag_bit_the_sdk_does_not_name_is_refused_rather_than_blanket_accepted",
    ),
    (
        "the named-flag set widened by one bit the SDK does not define",
        L2EVICT,
        "    FLAGS_ALL | FLAGS_FIRST | FLAGS_LAST | FLAGS_NORMAL | FLAGS_CLEAN | FLAGS_WAIT_FB_PULL;",
        "    FLAGS_ALL | FLAGS_FIRST | FLAGS_LAST | FLAGS_NORMAL | FLAGS_CLEAN | FLAGS_WAIT_FB_PULL | 0x40;",
        DEVICE,
        "a_flag_bit_the_sdk_does_not_name_is_refused_rather_than_blanket_accepted",
    ),
    (
        "WAIT_FB_PULL dropped from the value a GA106 actually sends (the "
        "`bL2CleanFbPull` HAL field misread)",
        L2EVICT,
        "pub const FLAGS_CLEAN_VERIFY_BAR2: u32 = FLAGS_ALL | FLAGS_CLEAN | FLAGS_WAIT_FB_PULL;",
        "pub const FLAGS_CLEAN_VERIFY_BAR2: u32 = FLAGS_ALL | FLAGS_CLEAN;",
        ABI,
        "the_value_kbusverifybar2_sends_on_a_ga106_decodes",
    ),
    (
        "short params read past instead of refused",
        L2EVICT,
        "    if params.len() < L2_INVALIDATE_EVICT_PARAMS_SIZE {",
        "    if params.len() < 1 {",
        ABI,
        "short_params_are_refused_rather_than_padded",
    ),
    (
        "the policy serves an undecodable request instead of refusing it",
        INITTABLES,
        "                let Ok(evict) = l2evict::decode_l2_invalidate_evict(\n"
        "                    &cmd.payload[at..at + l2evict::L2_INVALIDATE_EVICT_PARAMS_SIZE],\n"
        "                ) else {\n"
        "                    return refuse();\n"
        "                };",
        "                let evict = l2evict::decode_l2_invalidate_evict(\n"
        "                    &cmd.payload[at..at + l2evict::L2_INVALIDATE_EVICT_PARAMS_SIZE],\n"
        "                )\n"
        "                .unwrap_or(l2evict::L2InvalidateEvict { flags: 0 });",
        DEVICE,
        "the_decoder_and_the_policy_refuse_the_same_requests",
    ),
    (
        "the control leaves the served universe every coverage gate quantifies over",
        INITTABLES,
        "        Self::MemsysL2InvalidateEvict,\n    ];",
        "        Self::DeviceInfo,\n    ];",
        DEVICE,
        "the_control_is_in_the_served_universe_every_gate_quantifies_over",
    ),
    (
        "the sysmembar's CORRECTED disposition reverted to the false one",
        SWEEP,
        "        cmd: 0x2080_0a70,\n        engine: \"KernelBus\",\n        disposition: SweepDisposition::RefusalIsInvisible,",
        "        cmd: 0x2080_0a70,\n        engine: \"KernelBus\",\n        disposition: SweepDisposition::RefusalHalts,",
        DEVICE,
        "the_sysmembar_beside_it_was_decided_separately_and_is_still_refused",
    ),
    (
        "the sysmembar's reverted disposition, seen by the triage gate too",
        SWEEP,
        "        cmd: 0x2080_0a70,\n        engine: \"KernelBus\",\n        disposition: SweepDisposition::RefusalIsInvisible,",
        "        cmd: 0x2080_0a70,\n        engine: \"KernelBus\",\n        disposition: SweepDisposition::RefusalHalts,",
        TRIAGE,
        "a_control_whose_refusal_is_invisible_must_cite_the_oracles_own_reply",
    ),
    (
        "the L2 evict silently un-served, leaving its triage row claiming otherwise",
        INITTABLES,
        "        Self::MemsysL2InvalidateEvict,\n    ];",
        "        Self::DeviceInfo,\n    ];",
        TRIAGE,
        "a_halting_refusal_may_be_served_or_not_and_the_table_says_which",
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

    # Baseline: every target must be GREEN before any mutation, or a red result below says
    # nothing at all.
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
