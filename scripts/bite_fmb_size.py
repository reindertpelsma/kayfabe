#!/usr/bin/env python3
"""Bite harness for the fault-method-buffer-size rung (`0x20802a08`).

★★★ This rung's whole value is a **number nobody could derive**, so the mutations are aimed
at the two ways a right number becomes worthless: the value stops being the measured one, or
the refusal that stands in for "no measurement" stops being a refusal.

⚠ There is a mutation this harness deliberately CANNOT plant: "the measurement was wrong".
No test in the tree can catch that, because no test has access to a GA106. What the suite can
catch is drift *away from* what was measured — which is what every row below probes. Stated
so nobody reads a green 12/12 as evidence that 20480 is correct.

★★ Files are restored with an explicit `os.utime` bump. `shutil.copy2`/`cp -a` preserve the
OLD mtime, so cargo serves a stale rlib and every later mutation reads as a non-biter —
measured twice on 2026-07-28.

Usage:  python3 scripts/bite_fmb_size.py [--cargo-target DIR]
"""

import argparse
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

FMBSIZE = "crates/kayfabe-abi/src/fmbsize.rs"
INITTABLES = "crates/kayfabe-device/src/inittables.rs"
GA10X = "crates/kayfabe-device/src/ga10x.rs"
DEVLIB = "crates/kayfabe-device/src/lib.rs"
SWEEP = "crates/kayfabe-device/src/sweep.rs"

FMB = "-p kayfabe-device --test ce_fault_method_buffer_size"
ABI = "-p kayfabe-abi --lib"
TRIAGE = "-p kayfabe-device --test sweep_triage"
INIT = "-p kayfabe-device --test init_tables"
CREC = "-p kayfabe-crec --test cap1b_differential"

# (name, file, old, new, cargo-args, expected-test)
MUTATIONS = [
    (
        "the measured size drifts to a plausible neighbour (16 KiB, not 20)",
        FMBSIZE,
        "pub const GA106_CE_FAULT_METHOD_BUFFER_SIZE: u32 = 20480;",
        "pub const GA106_CE_FAULT_METHOD_BUFFER_SIZE: u32 = 16384;",
        FMB,
        "the_reply_carries_the_bytes_the_real_ga106_put_on_the_wire",
    ),
    (
        "the measured size drifts, seen by the abi unit test's raw-byte pin",
        FMBSIZE,
        "pub const GA106_CE_FAULT_METHOD_BUFFER_SIZE: u32 = 20480;",
        "pub const GA106_CE_FAULT_METHOD_BUFFER_SIZE: u32 = 16384;",
        ABI,
        "the_reply_is_the_four_bytes_the_real_ga106_put_on_the_wire",
    ),
    (
        "the chip row stops carrying the measurement and hard-codes its own number",
        GA10X,
        "    ce_fault_method_buffer_size: kayfabe_abi::fmbsize::GA106_CE_FAULT_METHOD_BUFFER_SIZE,",
        "    ce_fault_method_buffer_size: 4096,",
        FMB,
        "the_answer_comes_from_the_chip_row_and_not_from_this_crate",
    ),
    (
        "the reply is emitted BIG-endian — the same number, unreadable by the guest",
        FMBSIZE,
        "    Ok(size.to_le_bytes().to_vec())",
        "    Ok(size.to_be_bytes().to_vec())",
        FMB,
        "the_reply_carries_the_bytes_the_real_ga106_put_on_the_wire",
    ),
    (
        "★★★ zero is ENCODED instead of refused — the wall, rebuilt behind an NV_OK",
        FMBSIZE,
        "    if size == 0 {\n        return Err(FaultMethodBufferSizeError::Zero);\n    }\n    Ok(size.to_le_bytes().to_vec())",
        "    Ok(size.to_le_bytes().to_vec())",
        FMB,
        "the_encoder_refuses_a_zero_even_if_a_chip_row_smuggles_one_past",
    ),
    (
        "zero encoded, seen by the abi test that states the rule independently",
        FMBSIZE,
        "    if size == 0 {\n        return Err(FaultMethodBufferSizeError::Zero);\n    }\n    Ok(size.to_le_bytes().to_vec())",
        "    Ok(size.to_le_bytes().to_vec())",
        ABI,
        "zero_is_refused_on_both_sides_because_zero_is_the_bug",
    ),
    (
        "a chip stating NO size realizes anyway — the operator-visible refusal removed",
        DEVLIB,
        "    if chip.ce_fault_method_buffer_size == 0 {\n        return Err(ChipError::NoFaultMethodBufferSize {\n            device_id: chip.pci_device_id,\n        });\n    }",
        "    if false {\n        return Err(ChipError::NoFaultMethodBufferSize {\n            device_id: chip.pci_device_id,\n        });\n    }",
        FMB,
        "a_chip_that_states_no_size_is_refused_at_realize_not_served_a_zero",
    ),
    (
        "a short reply decodes as a smaller number instead of being refused",
        FMBSIZE,
        "    let Some(w) = params.get(..CE_FAULT_METHOD_BUFFER_SIZE_PARAMS_SIZE) else {\n        return Err(FaultMethodBufferSizeError::Zero);\n    };",
        "    let mut padded = params.to_vec();\n    padded.resize(CE_FAULT_METHOD_BUFFER_SIZE_PARAMS_SIZE, 0);\n    let w = &padded[..];",
        ABI,
        "a_short_body_is_not_read_as_a_smaller_number",
    ),
    (
        "the policy answers four zeros for a chip with no size instead of refusing",
        INITTABLES,
        "                match kayfabe_abi::fmbsize::encode_fault_method_buffer_size(\n                    self.chip.ce_fault_method_buffer_size,\n                ) {\n                    Ok(p) => p,\n                    Err(_) => return refuse(),\n                }",
        "                kayfabe_abi::fmbsize::encode_fault_method_buffer_size(\n                    self.chip.ce_fault_method_buffer_size,\n                )\n                .unwrap_or_else(|_| vec![0u8; 4])",
        FMB,
        "the_encoder_refuses_a_zero_even_if_a_chip_row_smuggles_one_past",
    ),
    (
        "the control leaves the served universe every coverage gate quantifies over",
        INITTABLES,
        "        Self::CeFaultMethodBufferSize,\n        Self::GrCaps,",
        "        Self::DeviceInfo,\n        Self::GrCaps,",
        INIT,
        "every_variant_of_the_served_universe_round_trips_through_its_own_control_id",
    ),
    (
        "the control silently un-served — the ORACLE's replay stops exercising it",
        INITTABLES,
        "        Self::CeFaultMethodBufferSize,\n        Self::GrCaps,",
        "        Self::DeviceInfo,\n        Self::GrCaps,",
        CREC,
        "the_served_replies_are_the_ones_posted_and_each_carries_the_result_it_earned",
    ),
    (
        "★★ the triage row reverts to the reading a real GA106 falsified",
        SWEEP,
        "        cmd: 0x2080_2a08,\n        engine: \"KernelCE\",\n        disposition: SweepDisposition::AmputationUnsurvivable,",
        "        cmd: 0x2080_2a08,\n        engine: \"KernelCE\",\n        disposition: SweepDisposition::RefusalIsInvisible,",
        TRIAGE,
        "the_unsurvivable_class_still_names_the_measured_crashes",
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
