#!/usr/bin/env python3
"""Bite harness for increment **E4** — the GA10x USERD model and pushbuffer codec.

Plant a defect in `Ga10xUserd` / `Ga10xPushbuffer` or in the `kayfabe_abi::submit`
decoders they are built on, and require the guards to go RED. `suspect_the_instrument_first`
is the reason this exists at all: **a gate nobody has watched fail is not a gate**, and on
2026-08-02 every one of the twenty new E4 tests passed on its first run, which is exactly
the situation that memory is about.

★★★ **Four arms, because the point is which instrument catches what.**

  - `oracle`  — `tests/pushbuffer_abi_oracle.rs`: NVIDIA's own `DRF_NUM`/`DRF_VAL` over
                `clc56f.h`/`clc7b5.h`, plus the driver's own `kfifoGetUserdSizeAlign` HAL.
                This is the only arm that can catch a **wrong bit position**, because it is
                the only one that knows what the bits are.
  - `hostile` — `tests/pushbuffer_ga10x_hostile.rs`: E4's stated *control*. Garbage must
                fault or refuse. This is the arm that catches a **loosened refusal**.
  - `abi`     — `cargo test -p kayfabe-abi`: the unit tests beside the decoders, including
                the round trips that are explicitly labelled as proving nothing on their
                own.
  - `port`    — `tests/gsp_rm_alloc.rs`: the tripwire on the **shipped** `Arch`, so a
                generation swapped underneath the port is caught somewhere.

⊘ **The `abi` arm is here to be measured, not trusted.** `submit.rs`'s round-trip tests
compare our encoder against our decoder; the whole argument of `pushbuffer_abi_oracle.rs` is
that such a pair agrees with itself when both halves are wrong. Any bite that `abi` misses
and `oracle` catches is that argument, quantified — and a bite that only `abi` catches is
worth reading, because it means the oracle has a blind spot.

Usage:
    scripts/bite_pushbuffer_codec.py [--only N] [--list]

Every bite is applied to the working tree and restored afterwards, and the file's mtime is
touched after restoring — `shutil`/`cp -a` preserve mtime, cargo then serves a stale rlib,
and the next bite is measured against the previous one's binary
(`bite_harness_must_touch_after_restore`).
"""

import argparse
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ABI = os.path.join(ROOT, "crates/kayfabe-abi/src/submit.rs")
CHIPS = os.path.join(ROOT, "crates/kayfabe-chips/src/ga10x.rs")

ARMS = (
    ("oracle", ["cargo", "test", "-q", "--test", "pushbuffer_abi_oracle"]),
    ("hostile", ["cargo", "test", "-q", "--test", "pushbuffer_ga10x_hostile"]),
    ("abi", ["cargo", "test", "-q", "-p", "kayfabe-abi"]),
    ("port", ["cargo", "test", "-q", "--test", "gsp_rm_alloc"]),
)

RED = "RED"
#: An **equivalent mutant**: a rewrite that cannot change behaviour on any reachable input.
#: No test can catch it, and the harness REQUIRES it to stay green — a red one means the
#: equivalence argument in its own name is wrong, which is a finding rather than a pass.
EQUIVALENT = "EQUIVALENT"

# (name, file, old, new, expect) — `old` must appear EXACTLY ONCE in the file.
BITES = [
    # ---------------------------------------------------------------- GPFIFO entries ---
    (
        "a control entry (LENGTH == 0) is decoded as a range — its low byte is OPCODE, "
        "not GET_HI, so this fabricates a pointer into guest memory",
        ABI,
        "    let dwords = (entry1 >> 10) & 0x1F_FFFF;\n    if dwords == 0 {\n        return None;\n    }",
        "    let dwords = (entry1 >> 10) & 0x1F_FFFF;",
        RED,
    ),
    (
        "GET_HI is read as 16 bits instead of 8 — a pushbuffer above 2^40 gets an address "
        "the entry cannot have named",
        ABI,
        "    let hi = entry1 & 0xFF;",
        "    let hi = entry1 & 0xFFFF;",
        RED,
    ),
    (
        "GP_ENTRY0's FETCH bit is read as part of the address (31:0 instead of 31:2)",
        ABI,
        "    let lo = entry0 & 0xFFFF_FFFC;",
        "    let lo = entry0;",
        RED,
    ),
    (
        "LENGTH is read one bit too wide (31:10 instead of 30:10) — it overlaps SYNC, so "
        "a sync-wait entry doubles its own length",
        ABI,
        "    let dwords = (entry1 >> 10) & 0x1F_FFFF;",
        "    let dwords = (entry1 >> 10) & 0x3F_FFFF;",
        RED,
    ),
    (
        "the LEVEL bit is read at 8 instead of 9",
        ABI,
        "        subroutine: (entry1 >> 9) & 1 == 1,",
        "        subroutine: (entry1 >> 8) & 1 == 1,",
        RED,
    ),
    # ---------------------------------------------------------------- method framing ---
    (
        "the IMMEDIATE form is given `count` argument words — the datum is IN the header, "
        "so this swallows the next method and desynchronises the parser",
        ABI,
        "        sec_op::IMMD_DATA_METHOD => (MethodForm::Immediate, method, 0),",
        "        sec_op::IMMD_DATA_METHOD => (MethodForm::Immediate, method, count),",
        RED,
    ),
    (
        "RESERVED6 and the undefined GRP2 TERT_OPs are SIZED instead of refused — the "
        "class header defines no format for them, so the count is invented",
        ABI,
        "        // GRP2 with an unenumerated TERT_OP, and RESERVED6. No format, no size.\n        _ => return None,",
        "        _ => (MethodForm::Legacy, method, count),",
        RED,
    ),
    (
        "METHOD_COUNT is read as 12 bits instead of 13 — every run of 4096+ words is cut "
        "in half and the parser lands in the middle of its own arguments",
        ABI,
        "    let count = ((header >> 16) & 0x1FFF) as usize;",
        "    let count = ((header >> 16) & 0xFFF) as usize;",
        RED,
    ),
    (
        "the method address is reported as a DWORD INDEX rather than a byte offset — "
        "`LAUNCH_DMA` becomes 0xC0, which on a copy engine is a different register and "
        "does not fault",
        ABI,
        "    let method = (header & 0xFFF) * 4;",
        "    let method = header & 0xFFF;",
        RED,
    ),
    (
        "the legacy COUNT_OLD is read at the modern extent (28:16 instead of 28:18)",
        ABI,
        "    let old_count = ((header >> 18) & 0x7FF) as usize;",
        "    let old_count = ((header >> 16) & 0x1FFF) as usize;",
        RED,
    ),
    (
        "the legacy ADDRESS_OLD is read at the modern extent (11:0 instead of 12:2)",
        ABI,
        "    let old_method = ((header >> 2) & 0x7FF) * 4;",
        "    let old_method = (header & 0xFFF) * 4;",
        RED,
    ),
    # ------------------------------------------------------------------------ USERD ---
    (
        "USERD is sized at 4096 — one channel's model then covers eight channels' USERD",
        ABI,
        "pub const USERD_SIZE: u64 = 512;",
        "pub const USERD_SIZE: u64 = 4096;",
        RED,
    ),
    (
        "GP_GET is read one dword early (33 instead of 34) — the consume cursor becomes "
        "GET_HI's neighbour and never moves",
        ABI,
        "pub const USERD_GP_GET: u64 = 34 * 4;",
        "pub const USERD_GP_GET: u64 = 33 * 4;",
        RED,
    ),
    (
        "GP_PUT is reported at GP_GET's offset — every submission overwrites the cursor "
        "hardware writes",
        CHIPS,
        "    fn gp_put_offset(&self) -> u64 {\n        submit::USERD_GP_PUT\n    }",
        "    fn gp_put_offset(&self) -> u64 {\n        submit::USERD_GP_GET\n    }",
        RED,
    ),
    # ----------------------------------------------------------------- the decodes ----
    (
        "`SET_OBJECT` takes the whole word as the class — bits 20:16 are ENGINE, so every "
        "bind on a non-zero engine reports a class nothing names",
        ABI,
        "pub const SET_OBJECT_NVCLASS_MASK: u32 = 0xFFFF;",
        "pub const SET_OBJECT_NVCLASS_MASK: u32 = 0xFFFF_FFFF;",
        RED,
    ),
    (
        "SEM_EXECUTE's OPERATION is masked to one bit — ACQ_STRICT_GEQ (2) and REDUCTION "
        "(6) then read as ACQUIRE, and ACQ_CIRC_GEQ (3) reads as a RELEASE",
        ABI,
        "    pub const SEM_EXECUTE_OPERATION_MASK: u32 = 0x7;",
        "    pub const SEM_EXECUTE_OPERATION_MASK: u32 = 0x1;",
        RED,
    ),
    (
        "the PDB address' low field is masked to 31:20 instead of 31:12",
        ABI,
        "    pub const MEM_OP_C_PDB_ADDR_LO_MASK: u32 = 0xFFFF_F000;",
        "    pub const MEM_OP_C_PDB_ADDR_LO_MASK: u32 = 0xFFF0_0000;",
        RED,
    ),
    (
        "the exact-argument-count check is dropped — a run the range cut in half decodes "
        "with the missing words read as zero",
        CHIPS,
        "        if h.form != submit::MethodForm::Incrementing || args.len() != h.arg_words {",
        "        if h.form != submit::MethodForm::Incrementing {",
        RED,
    ),
    (
        "the incrementing-framing check is dropped — a NON_INC run writes five words to "
        "ONE register and is a different operation entirely",
        CHIPS,
        "        if h.form != submit::MethodForm::Incrementing || args.len() != h.arg_words {",
        "        if args.len() != h.arg_words {",
        RED,
    ),
    (
        "an ACQUIRE is decoded as a release — this announces a completion the guest is "
        "still waiting for",
        CHIPS,
        "        if execute & submit::fifo::SEM_EXECUTE_OPERATION_MASK\n            != submit::fifo::SEM_EXECUTE_OPERATION_RELEASE\n        {\n            // Six of the eight operations are acquires, and REDUCTION is neither.\n            return None;\n        }",
        "        let _ = submit::fifo::SEM_EXECUTE_OPERATION_RELEASE;",
        RED,
    ),
    (
        "the 32-bit payload branch is dropped — SEM_PAYLOAD_HI is read even when the "
        "engine writes four bytes, inventing the top half of a fence",
        CHIPS,
        "        let payload = if execute & submit::fifo::SEM_EXECUTE_PAYLOAD_SIZE_64BIT != 0 {\n            u64::from(payload_lo) | (u64::from(payload_hi) << 32)\n        } else {\n            u64::from(payload_lo)\n        };",
        "        let payload = u64::from(payload_lo) | (u64::from(payload_hi) << 32);",
        RED,
    ),
    (
        "`PDB_ALL` is decoded as a PDB-targeted invalidate — there is no address in it",
        CHIPS,
        "        if c & submit::fifo::MEM_OP_C_PDB_ALL != 0 {\n            // `PDB_ALL` names no page directory; there is no `pdb` to report.\n            return None;\n        }",
        "",
        RED,
    ),
    (
        "every MEM_OP is decoded as a TLB invalidate — a MEMBAR and an L2 flush share the "
        "same four-word run",
        CHIPS,
        "        if op != submit::fifo::MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE\n            && op != submit::fifo::MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE_TARGETED\n        {\n            // MEMBAR, the L2 operations and ACCESS_COUNTER_CLR share this method run.\n            return None;\n        }",
        "        let _ = op;",
        RED,
    ),
    (
        "the ring's whole-entries check is dropped — a ring whose framing we cannot vouch "
        "for is decoded on a best-effort prefix",
        CHIPS,
        "        if ring.is_empty() || !ring.len().is_multiple_of(submit::GP_ENTRY_SIZE as usize) {\n            return Vec::new();\n        }\n",
        "",
        RED,
    ),
    # --------------------------------------------------------- ★★★ THE ONE THAT MATTERS
    (
        "★★★ `LAUNCH_DMA` fabricates a `CeLaunchDma` out of its flags word alone — the "
        "exact regression E4 refuses, because the operands were written by EARLIER runs "
        "and a per-method seam cannot see them",
        CHIPS,
        "            (submit::fifo::MEM_OP_A, 4) => Self::tlb_invalidate(args),",
        "            (submit::fifo::MEM_OP_A, 4) => Self::tlb_invalidate(args),\n"
        "            (submit::ce::LAUNCH_DMA, 1) => Some(PushMethod::CeLaunchDma {\n"
        "                dst: GpuVa(0),\n"
        "                src: GpuVa(0),\n"
        "                len: 0,\n"
        "                dst_is_virtual: true,\n"
        "                src_is_virtual: true,\n"
        "                work: kayfabe_arch::CeWork::Copy,\n"
        "            }),",
        RED,
    ),
    # ------------------------------------------------------------- equivalent mutants --
    (
        "the `entry & 0xFFFF_FFFF` mask on entry0 is dropped — EQUIVALENT: `entry0` is "
        "immediately masked to 0xFFFF_FFFC, which already drops every bit above 31",
        ABI,
        "    let entry0 = entry & 0xFFFF_FFFF;",
        "    let entry0 = entry;",
        EQUIVALENT,
    ),
]


def run(cmd, env):
    return subprocess.run(
        cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", type=int, default=None)
    ap.add_argument("--list", action="store_true")
    args = ap.parse_args()

    if args.list:
        for i, (name, path, _, _, expect) in enumerate(BITES):
            print(f"{i:2d}  [{expect:10s}] {os.path.basename(path):12s} {name}")
        return 0

    env = dict(os.environ)

    originals = {p: open(p, encoding="utf-8").read() for p in (ABI, CHIPS)}

    # ★ The clean tree must be GREEN in every arm, or a red bite proves nothing. And the
    # ORACLE arm must actually have RUN: on a box with no vendored ogkm tree it announces
    # `PUSHBUFFER-ORACLE-GATE: SKIPPED` and exits 0, so every bite would read as "missed
    # by the oracle" and this harness would print a confidently wrong table.
    print("== baseline ==", flush=True)
    base = {k: run(c, env) for k, c in ARMS}
    if any(r.returncode != 0 for r in base.values()):
        print("BASELINE IS NOT GREEN — every bite below would be meaningless.")
        for k, r in base.items():
            if r.returncode != 0:
                print(f"--- {k} ---\n{r.stdout.decode()[-3000:]}")
        return 2
    if b"PUSHBUFFER-ORACLE-GATE: RAN" not in base["oracle"].stdout:
        print(
            "THE ORACLE ARM DID NOT RUN. This box has no vendored open-kernel-modules "
            "tree, so `pushbuffer_abi_oracle.rs` SKIPPED and asserted nothing. Every bite "
            "would be reported as 'the oracle missed it', which is false. Set "
            "KAYFABE_OGKM_580 and re-run."
        )
        return 2
    print("baseline: all four arms GREEN, and the oracle RAN\n", flush=True)

    results = []
    todo = range(len(BITES)) if args.only is None else [args.only]
    try:
        for i in todo:
            name, path, old, new, expect = BITES[i]
            original = originals[path]
            n = original.count(old)
            if n != 1:
                print(f"{i:2d}  {name}\n    ★ ANCHOR MATCHED {n} TIMES — bite not applied")
                results.append((i, name, None, expect))
                continue
            if new == old:
                print(
                    f"{i:2d}  {name}\n    ★ BITE IS A NO-OP — the replacement is the "
                    f"original, so this row would report a false GREEN"
                )
                results.append((i, name, None, expect))
                continue
            # ★★★ RESTORE **EVERY** FILE FIRST, not just the one this bite touches.
            #
            # This harness bites two files. Its first version wrote
            # `originals[path].replace(...)` to `path` and left the OTHER file holding the
            # PREVIOUS bite — so every row whose predecessor bit a different file was
            # measured against two defects at once. It was caught only because one
            # EQUIVALENT row went red: bite 25 (a provably behaviour-preserving rewrite in
            # `submit.rs`) followed bite 24 (a fabricated `CeLaunchDma` in `ga10x.rs`),
            # and the oracle was reporting bite 24's failure under bite 25's name.
            #
            # ⊘ A single-file harness cannot exhibit this, which is why the pattern this
            # was copied from does not guard against it. `suspect_the_instrument_first`.
            for q, text in originals.items():
                with open(q, "w", encoding="utf-8") as f:
                    f.write(text)
                os.utime(q, None)
            with open(path, "w", encoding="utf-8") as f:
                f.write(original.replace(old, new))
            os.utime(path, None)
            time.sleep(0.01)
            red = {}
            for k, c in ARMS:
                r = run(c, env)
                # ★ A bite that does not COMPILE is not a measurement: cargo exits
                # non-zero and every arm reads RED for a reason that has nothing to do
                # with the guard. Reported as its own outcome rather than as a catch.
                red[k] = (
                    "BUILD"
                    if b"error[" in r.stdout or b"error: could not compile" in r.stdout
                    else r.returncode != 0
                )
            if any(v == "BUILD" for v in red.values()):
                print(f"{i:2d}  ★ BITE DID NOT COMPILE — not a measurement: {name}")
                results.append((i, name, None, expect))
                continue
            caught_by = [k for k, _ in ARMS if red[k]]
            note = ""
            if expect == EQUIVALENT:
                note = (
                    "  [equivalent — GREEN is correct]"
                    if not caught_by
                    else "  ★★★ AN EQUIVALENT MUTANT WENT RED — the argument in this "
                    "row's name is WRONG"
                )
            elif not caught_by:
                note = "  ★★ MISSED BY EVERYTHING"
            elif caught_by == ["oracle"]:
                note = "  <== ORACLE ALONE"
            elif caught_by == ["hostile"]:
                note = "  <== HOSTILE ALONE"
            elif caught_by == ["abi"]:
                note = "  <== ABI ALONE (the oracle has a blind spot here)"
            print(
                "{:2d}  {}  {}{}".format(
                    i,
                    " ".join(
                        f"{k}={'RED ' if red[k] else 'GREEN'}" for k, _ in ARMS
                    ),
                    name,
                    note,
                ),
                flush=True,
            )
            results.append((i, name, red, expect))
    finally:
        for p, text in originals.items():
            with open(p, "w", encoding="utf-8") as f:
                f.write(text)
            os.utime(p, None)

    applied = [r for r in results if r[2] is not None]
    live = [r for r in applied if r[3] == RED]
    equiv = [r for r in applied if r[3] == EQUIVALENT]
    caught = [r for r in live if any(r[2].values())]
    missed = [r for r in live if not any(r[2].values())]
    broken_equiv = [r for r in equiv if any(r[2].values())]
    per_arm = {
        k: sum(1 for r in live if r[2][k]) for k, _ in ARMS
    }
    oracle_only = [r for r in live if r[2]["oracle"] and not r[2]["hostile"]]
    hostile_only = [r for r in live if r[2]["hostile"] and not r[2]["oracle"]]
    print(
        f"\n{len(caught)}/{len(live)} live bites caught. Per arm: "
        + ", ".join(f"{k}={per_arm[k]}" for k, _ in ARMS)
        + f".\n{len(oracle_only)} caught by the ORACLE and not the control; "
        f"{len(hostile_only)} by the CONTROL and not the oracle — the two are guarding "
        f"different things and neither substitutes for the other.\n"
        f"{len(equiv)} rows are EQUIVALENT MUTANTS (required to stay green; "
        f"{len(broken_equiv)} did not)."
    )
    for r in missed:
        print(f"  MISSED: {r[0]:2d} {r[1]}")
    for r in broken_equiv:
        print(f"  EQUIVALENCE CLAIM FALSIFIED: {r[0]:2d} {r[1]}")
    if len(applied) != len(results):
        print("  ★ SOME BITES WERE NOT APPLIED — see the lines above.")
        return 2
    return 1 if (missed or broken_equiv) else 0


if __name__ == "__main__":
    sys.exit(main())
