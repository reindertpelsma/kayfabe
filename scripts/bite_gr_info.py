#!/usr/bin/env python3
"""Bite harness for the GR-info rung (`0x20800a2a`) and the uncaptured-oracle-row refusal.

★★★ This rung has two halves and they fail in opposite directions, so the mutations do too:

  1. **The data half.** 3712 bytes measured on a real GA106. A wrong entry, a wrong index
     field, a wrong stride or a served zero all produce a *plausible* reply the guest
     believes — and `infoList[0x2c] = 0` specifically rebuilds `RmInitAdapter failed!
     (0x25:0x40:1249)` behind an `NV_OK`.
  2. **The epistemic half.** The rule that an EMPTY capture is evidence of nothing. Its
     failure mode is silence: a decoder that returns `psize` zero bytes instead of refusing
     looks identical to one that refuses, unless something inspects the raw result.

⚠ Mutations this harness deliberately CANNOT plant: "the measurement was wrong", and "the
port should not serve this control at all". No test in the tree can catch either — the first
needs a GA106, the second needs a boot. What the suite catches is drift *away from* what was
measured and *away from* what was decided. Stated so nobody reads a green N/N as evidence
that the 58 numbers are correct.

★★ Files are restored with an explicit `os.utime` bump. `shutil.copy2`/`cp -a` preserve the
OLD mtime, so cargo serves a stale rlib and every later mutation reads as a non-biter —
measured twice on 2026-07-28.

Usage:  python3 scripts/bite_gr_info.py [--cargo-target DIR]
"""

import argparse
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

GRINFO = "crates/kayfabe-abi/src/grinfo.rs"
ORACLE = "crates/kayfabe-abi/src/oracle.rs"
INITTABLES = "crates/kayfabe-device/src/inittables.rs"
GA10X = "crates/kayfabe-device/src/ga10x.rs"
SWEEP = "crates/kayfabe-device/src/sweep.rs"

DEV = "-p kayfabe-device --test gr_info"
ABI = "-p kayfabe-abi --lib"
TRACE = "-p kayfabe-abi --test real_ga106_bodies"
TRIAGE = "-p kayfabe-device --test sweep_triage"
CENSUS = "-p kayfabe-abi --test initctrl_census"
INIT = "-p kayfabe-device --test init_tables"
CREC = "-p kayfabe-crec --test cap1b_differential"

# (name, file, old, new, cargo-args, expected-test)
MUTATIONS = [
    # ── the data half ──────────────────────────────────────────────────────────────────
    (
        "★★★ the entry that ended fmb1 drifts to a plausible neighbour (32, not 64)",
        GRINFO,
        "        64,    // 0x2c MAX_SUBCONTEXT_COUNT     ★★★ the entry run fmb1 died without",
        "        32,    // 0x2c MAX_SUBCONTEXT_COUNT     ★★★ the entry run fmb1 died without",
        TRACE,
        "the_gr_info_reply_is_byte_identical_to_the_real_ga106",
    ),
    (
        "the same drift, seen at the wire by the device-level raw-byte pin",
        GRINFO,
        "        64,    // 0x2c MAX_SUBCONTEXT_COUNT     ★★★ the entry run fmb1 died without",
        "        32,    // 0x2c MAX_SUBCONTEXT_COUNT     ★★★ the entry run fmb1 died without",
        DEV,
        "the_reply_carries_the_bytes_the_real_ga106_put_on_the_wire",
    ),
    (
        "a LITTER constant nothing cross-checks drifts — only the trace can see it",
        GRINFO,
        "        8,     // 0x19 LITTER_NUM_MXBAR_FBP_PORTS",
        "        4,     // 0x19 LITTER_NUM_MXBAR_FBP_PORTS",
        TRACE,
        "the_gr_info_reply_is_byte_identical_to_the_real_ga106",
    ),
    (
        "★★ the entry stops carrying its own position — gpu.c:6279 searches on that field",
        GRINFO,
        "            out[at..at + 4].copy_from_slice(&index.to_le_bytes());",
        "            out[at..at + 4].copy_from_slice(&0u32.to_le_bytes());",
        ABI,
        "every_entry_carries_its_own_position_as_its_index",
    ),
    (
        "the reply is emitted BIG-endian — the same 58 numbers, unreadable by the guest",
        GRINFO,
        "            out[at + 4..at + 8].copy_from_slice(&data.to_le_bytes());",
        "            out[at + 4..at + 8].copy_from_slice(&data.to_be_bytes());",
        TRACE,
        "the_gr_info_reply_is_byte_identical_to_the_real_ga106",
    ),
    (
        "★★★ a zero max-subcontext count is ENCODED instead of refused — the wall, rebuilt",
        GRINFO,
        "        if self.data[IDX_MAX_SUBCONTEXT_COUNT] == 0 {\n            return Err(GrInfoError::MaxSubcontextCountZero);\n        }",
        "        if false {\n            return Err(GrInfoError::MaxSubcontextCountZero);\n        }",
        DEV,
        "a_zero_max_subcontext_count_is_refused_rather_than_served",
    ),
    (
        "a zero GPC bound is encoded — every gpcId < maxNumGpcs check silently fails",
        GRINFO,
        "        if self.data[IDX_LITTER_NUM_GPCS] == 0 {\n            return Err(GrInfoError::LitterNumGpcsZero);\n        }",
        "        if false {\n            return Err(GrInfoError::LitterNumGpcsZero);\n        }",
        DEV,
        "the_other_two_load_bearing_zeros_are_refused_on_the_wire_too",
    ),
    (
        "★★ the two GR descriptions may disagree — one chip, two answers, nothing red",
        GRINFO,
        "            if self.data[index] != derived {",
        "            if false {",
        DEV,
        "a_chip_whose_two_gr_descriptions_disagree_is_refused_on_the_wire",
    ),
    (
        "the serve site stops cross-checking, so the encoder's agreement is never asked for",
        INITTABLES,
        "                if self\n                    .chip\n                    .gr_info\n                    .validate_against(&self.chip.gr_static)\n                    .is_err()\n                {\n                    return refuse();\n                }",
        "                if false {\n                    return refuse();\n                }",
        DEV,
        "a_chip_whose_two_gr_descriptions_disagree_is_refused_on_the_wire",
    ),
    (
        "the policy answers 3712 zeros for a bad row instead of refusing",
        INITTABLES,
        "                match self.chip.gr_info.encode() {\n                    Ok(p) => p,\n                    Err(_) => return refuse(),\n                }",
        "                self.chip\n                    .gr_info\n                    .encode()\n                    .unwrap_or_else(|_| vec![0u8; 3712])",
        DEV,
        "a_zero_max_subcontext_count_is_refused_rather_than_served",
    ),
    (
        "the chip row stops carrying the measurement and states its own table",
        GA10X,
        "    gr_info: kayfabe_abi::grinfo::GA106_GR_INFO,",
        "    gr_info: kayfabe_abi::grinfo::GrInfoProfile {\n        data: {\n            let mut d = kayfabe_abi::grinfo::GA106_GR_INFO.data;\n            d[0x25] = 11;\n            d\n        },\n    },",
        DEV,
        "the_answer_comes_from_the_chip_row_and_not_from_this_crate",
    ),
    (
        "the control leaves the served universe every coverage gate quantifies over",
        INITTABLES,
        "        Self::GrCaps,\n        Self::GrInfo,",
        "        Self::GrCaps,\n        Self::DeviceInfo,",
        INIT,
        "every_variant_of_the_served_universe_round_trips_through_its_own_control_id",
    ),
    (
        "the control silently un-served — the ORACLE's replay stops exercising it at seq 50",
        INITTABLES,
        "        Self::GrCaps,\n        Self::GrInfo,",
        "        Self::GrCaps,\n        Self::DeviceInfo,",
        CREC,
        "the_served_replies_are_the_ones_posted_and_each_carries_the_result_it_earned",
    ),
    (
        "★★ the triage row reverts to the reading boot fmb1 falsified",
        SWEEP,
        "        cmd: 0x2080_0a2a,\n        engine: \"KernelGraphics\",",
        "        cmd: 0x2080_0a2b,\n        engine: \"KernelGraphics\",",
        TRIAGE,
        "the_unsurvivable_class_still_names_the_measured_crashes",
    ),
    # ── the epistemic half ─────────────────────────────────────────────────────────────
    (
        "★★★ an EMPTY capture is decoded as psize zero bytes instead of refused",
        ORACLE,
        "    if dlen == 0 {\n        return Err(OracleRowError::BodyNeverCaptured { cmd, psize });\n    }",
        "    if dlen == 0 {\n        return Ok(CapturedEvidence::Complete { psize });\n    }",
        ABI,
        "every_empty_capture_row_is_refused_by_name_unless_no_body_exists",
    ),
    (
        "★★ the refusal is deleted outright — the rule stops existing",
        ORACLE,
        "    if dlen == 0 {\n        return Err(OracleRowError::BodyNeverCaptured { cmd, psize });\n    }",
        "    if dlen == 0 && psize == usize::MAX {\n        return Err(OracleRowError::BodyNeverCaptured { cmd, psize });\n    }",
        ABI,
        "every_empty_capture_row_is_refused_by_name_unless_no_body_exists",
    ),
    # ⊘ The bite that stood here — "a TRUNCATED row starts being refused too — the rule
    # over-reaches into a real body" — asserted the OPPOSITE of what the rule now says, and
    # its guard test asserted `Ok` for `0x20800a22`. Both were wrong: sixteen of the 56 rows
    # are truncated, more than the eleven empty ones, and a truncated row decoded cleanly
    # while zero-filling its uncaptured tail. It is replaced by its inverse rather than
    # deleted, because the *live* defect is now the one in the other direction.
    (
        "★★★ a TRUNCATED row decodes as a complete body — the uncaptured tail becomes zeros",
        ORACLE,
        "    if dlen < psize {\n        return Err(OracleRowError::BodyTruncated {\n"
        "            cmd,\n            kept: dlen,\n            psize,\n        });\n    }",
        "    if dlen < psize && psize == usize::MAX {\n        return Err(OracleRowError::BodyTruncated {\n"
        "            cmd,\n            kept: dlen,\n            psize,\n        });\n    }",
        ABI,
        "all_sixteen_truncated_rows_are_refused_and_none_is_a_zero_trim",
    ),
    (
        "★★ a truncated row leaves the sixteen — the class silently shortens, and the row "
        "that goes is the one already SERVED at 47%",
        ORACLE,
        "    TruncatedRow { cmd: 0x2080_0a22, psize: 34592, kept: 16376, c_line: 6221, trailing_zeros_kept: 12053 },\n",
        "",
        CENSUS,
        "every_truncated_row_in_the_c_header_is_in_truncated_rows_byte_for_byte",
    ),
    (
        "★★★ the zero-trim refutation is inverted — a row is claimed to end in a non-zero "
        "byte, which would make refusing it wrong",
        ORACLE,
        "    TruncatedRow { cmd: 0x2080_0a40, psize: 24580, kept: 16384, c_line: 6252, trailing_zeros_kept: 15833 },",
        "    TruncatedRow { cmd: 0x2080_0a40, psize: 24580, kept: 16384, c_line: 6252, trailing_zeros_kept: 0 },",
        CENSUS,
        "every_truncated_row_in_the_c_header_is_in_truncated_rows_byte_for_byte",
    ),
    (
        "★★ the per-FIELD escape hatch stops bounding — a read past the kept prefix is "
        "called captured, which is the zero-fill wearing a predicate's name",
        ORACLE,
        "        Some(end) => end <= dlen,",
        "        Some(end) => end <= dlen || end <= usize::MAX,",
        ABI,
        "a_field_inside_the_kept_prefix_is_captured_and_one_past_it_is_not",
    ),
    (
        "★★★ a claimed hardware body drifts from the committed trace",
        ORACLE,
        "        real: &[0x00, 0x00, 0x01, 0x04],",
        "        real: &[0x00, 0x00, 0x01, 0x05],",
        TRACE,
        "every_claimed_hardware_body_is_the_body_in_the_trace",
    ),
    (
        "the ECHOING row is flattened to one value — the two-caller finding erased",
        ORACLE,
        "        real: &[0x31, 0x00, 0x00, 0x00],",
        "        real: &[0x11, 0x00, 0x00, 0x11],",
        TRACE,
        "every_claimed_hardware_body_is_the_body_in_the_trace",
    ),
    (
        "★★ a row leaves the enumerated eleven — the universe silently shortens",
        ORACLE,
        "    EmptyCaptureRow {\n        cmd: 0x2080_0aac,\n        psize: 4,\n        c_line: 6258,\n        real: &[0x00, 0x00, 0x01, 0x00],\n        coincides: false,\n    },\n",
        "",
        ABI,
        "the_eleven_are_all_here_and_the_share_is_stated",
    ),
    (
        "a coincidence is asserted rather than derived from the measured body",
        ORACLE,
        "        cmd: 0x2080_0a4b,\n        psize: 4,\n        c_line: 6257,\n        real: &[0x00, 0x00, 0x01, 0x04],\n        coincides: false,",
        "        cmd: 0x2080_0a4b,\n        psize: 4,\n        c_line: 6257,\n        real: &[0x00, 0x00, 0x01, 0x04],\n        coincides: true,",
        ABI,
        "the_refusal_is_not_the_only_witness_that_zeros_would_have_been_wrong",
    ),
    (
        "★★★ a triage row keeps the PHRASE 'a real GA106' but loses the committed trace — "
        "the exact shape 0x20800a6c's refuted four-zero claim had",
        SWEEP,
        "              words kbusVerifyBar2_GM107's call sites pass ([measured] 2026-08-01, \\\n"
        "              traces/real_ga106/rpc_bodies_real_ga106.txt; kayfabe_abi::oracle). ",
        "              words kbusVerifyBar2_GM107's call sites pass (a real GA106, asked). ",
        TRIAGE,
        "a_citation_of_an_uncaptured_oracle_row_must_carry_what_hardware_said",
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
