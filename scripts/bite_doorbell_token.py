#!/usr/bin/env python3
"""Bite harness for increment **E3** — the GA10x doorbell token decode.

Plant a defect in `Ga10xArch::decode_doorbell` and require the guards to go RED. A guard
nobody has watched fail is not a guard, and this is the one seam in the port whose wrong
answer is otherwise **silent**: a mis-decoded work-submit token routes a guest's ring to
another channel, and on the Mode-2 path we are the GSP, so nothing downstream notices
(`docs/design/execution_plane_increments.md` §2.1).

★★★ **Three arms, not one, because the point is which instrument catches what.**

  - `oracle`   — `tests/worksubmit_token_oracle.rs`: RM's own encoder, compiled and swept.
  - `hardware` — `tests/doorbell_token.rs`: tokens a real GA106 handed real channels,
                 against chids read out of RM's channel-ID manager.
  - `mock`     — `cargo test -p kayfabe-mocks`: the pre-existing suite, whose
                 `MockArch::token_for` is the **inverse of the mock's own decode**.

The `mock` arm exists to be **measured, not trusted**. `execution_plane_increments.md`
claims it is structurally blind to this class; a harness that only ran the two new arms
would leave that claim as an assertion. Any bite it catches is a bite it catches, and any
bite it misses is the argument, quantified.

⊘ **A bite that only `hardware` catches and `oracle` misses is expected to be rare** — the
census's values are small (chids 4–9, upper field 0/1/2/8), so a decoder wrong only in the
high bits agrees with it. The reverse (oracle-only) is expected to be common. Both numbers
are printed; neither is asserted, because asserting them would be asserting the conclusion.

Usage:
    scripts/bite_doorbell_token.py [--only N] [--list]

Every bite is applied to the working tree and restored afterwards, and the file's mtime is
touched after restoring — `shutil`/`cp -a` preserve mtime, cargo then serves a stale rlib,
and the next bite is measured against the previous one's binary.
"""

import argparse
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGET = os.path.join(ROOT, "crates/kayfabe-chips/src/ga10x.rs")

ORACLE_TEST = ["cargo", "test", "-q", "--test", "worksubmit_token_oracle"]
HARDWARE_TEST = ["cargo", "test", "-q", "--test", "doorbell_token"]
MOCK_TEST = ["cargo", "test", "-q", "-p", "kayfabe-mocks"]

# The decoder, as it ships. Every bite below rewrites exactly this block, so a refactor
# that moves it turns every anchor red at once rather than silently disarming the harness.
BODY = """        let raw = u32::try_from(token).ok()?;
        // The two fields RM defines…
        let vector = raw & 0x0000_0FFF; // NV_CTRL_VF_DOORBELL_VECTOR      11:0
        let runlist = (raw >> 16) & 0x0000_007F; // NV_CTRL_VF_DOORBELL_RUNLIST_ID 22:16
        // …and everything RM's encoder cannot have written.
        if raw & !0x007F_0FFF != 0 {
            return None;
        }"""


# What a bite is expected to do. Two values, and the second one is a claim that can fail.
RED = "RED"
#: ★★★ An **equivalent mutant**: a rewrite that cannot change the function's behaviour on
#: any input the function accepts, so NO test can catch it and a harness reporting it as
#: "missed" is reporting a gap that does not exist. Every `EQUIVALENT` row below carries
#: its proof in the name, and the harness **requires it to stay GREEN** — if one ever turns
#: red, the equivalence argument was wrong and that is a finding, not a pass.
EQUIVALENT = "EQUIVALENT"


def bite(name, replacement, expect=RED):
    return (name, BODY, replacement, expect)


# (name, old, new, expect) — `old` must appear EXACTLY ONCE in the file.
BITES = [
    # ---- the field WIDTHS: what hardware could not pin, because its values were small --
    #
    # ★★ The first two are EQUIVALENT, and finding that out is the most useful thing this
    # harness has produced. The decoder refuses any token with a bit outside `0x007F_0FFF`
    # BEFORE the field masks are read, so on every input it accepts `raw & 0xFFFF` and
    # `raw & 0x0FFF` are the same number, as are `(raw >> 16) & 0xFFFF` and `& 0x7F`.
    # Checked exhaustively over the whole admitted space (2^24 inputs, zero
    # counterexamples). The field masks and the refusal are redundant with each other by
    # construction — which is defence in depth, not a hole; bites 14 and 15 relax the
    # refusal and show the widths become load-bearing the moment it is gone.
    bite(
        "the chid field is 16 bits, not 12 — EQUIVALENT: the reserved-bit refusal already "
        "rejects every token with a bit in 15:12, so the two masks agree on every "
        "accepted input (checked exhaustively over 2^24)",
        BODY.replace("raw & 0x0000_0FFF", "raw & 0x0000_FFFF"),
        EQUIVALENT,
    ),
    bite(
        "the chid field is 11 bits — one too narrow",
        BODY.replace("raw & 0x0000_0FFF", "raw & 0x0000_07FF"),
    ),
    bite(
        "the runlist field is 16 bits, not 7 — EQUIVALENT, for bite 0's reason: bits 31:23 "
        "are refused before the mask is read",
        BODY.replace("(raw >> 16) & 0x0000_007F", "(raw >> 16) & 0x0000_FFFF"),
        EQUIVALENT,
    ),
    bite(
        "the runlist field is 6 bits — one too narrow",
        BODY.replace("(raw >> 16) & 0x0000_007F", "(raw >> 16) & 0x0000_003F"),
    ),
    # ---- the field SHIFTS ------------------------------------------------------------
    bite(
        "the runlist field starts at bit 12, not bit 16",
        BODY.replace("(raw >> 16)", "(raw >> 12)"),
    ),
    bite(
        "the runlist field starts at bit 17",
        BODY.replace("(raw >> 16)", "(raw >> 17)"),
    ),
    bite(
        "the chid is taken from bit 1 upward",
        BODY.replace("raw & 0x0000_0FFF", "(raw >> 1) & 0x0000_0FFF"),
    ),
    # ---- the two halves SWAPPED ------------------------------------------------------
    bite(
        "chid and runlist are swapped",
        BODY.replace("let vector = raw & 0x0000_0FFF;", "let runlist_ = raw & 0x0000_0FFF;")
        .replace(
            "let runlist = (raw >> 16) & 0x0000_007F;",
            "let vector = (raw >> 16) & 0x0000_007F;\n        let runlist = runlist_;",
        ),
    ),
    # ---- the collapse: every token to one channel (the `vchid_from_userd_flags` shape) -
    bite(
        "every token decodes to channel 0 — the silent single-channel mis-route",
        BODY.replace("let vector = raw & 0x0000_0FFF;", "let vector = 0u32;"),
    ),
    bite(
        "the runlist is dropped and reported as 0",
        BODY.replace("let runlist = (raw >> 16) & 0x0000_007F;", "let runlist = 0u32;"),
    ),
    # ---- the REFUSAL: accepting a token RM could not have written ---------------------
    bite(
        "the reserved-bit check is removed — any 32-bit word decodes",
        BODY.replace(
            """        if raw & !0x007F_0FFF != 0 {
            return None;
        }""",
            "",
        ),
    ),
    bite(
        "the reserved-bit mask lets bits 15:12 through",
        BODY.replace("!0x007F_0FFF", "!0x007F_FFFF"),
    ),
    bite(
        "the reserved-bit mask lets bits 31:23 through",
        BODY.replace("!0x007F_0FFF", "!0xFFFF_0FFF"),
    ),
    bite(
        "a 64-bit token is truncated rather than refused",
        BODY.replace(
            "let raw = u32::try_from(token).ok()?;",
            "#[expect(clippy::cast_possible_truncation, reason = \"planted bite\")]\n"
            "        let raw = token as u32;",
        ),
    ),
    # ---- the total refusal: back to what the seam said before E3 ----------------------
    # ---- ★ the COMPOSED bites: relax the refusal AND widen the field together ---------
    #
    # Bites 0 and 2 are equivalent only because the refusal covers for them. These two
    # remove that cover, so the field width becomes observable — and a suite that caught
    # bites 11/12 but not these would be catching the refusal alone and calling it the
    # encoding.
    bite(
        "the chid field is 16 bits AND the refusal lets 15:12 through — the width, "
        "observable",
        BODY.replace("raw & 0x0000_0FFF", "raw & 0x0000_FFFF").replace(
            "!0x007F_0FFF", "!0x007F_FFFF"
        ),
    ),
    bite(
        "the runlist field is 9 bits AND the refusal lets 31:23 through — the width, "
        "observable",
        BODY.replace("(raw >> 16) & 0x0000_007F", "(raw >> 16) & 0x0000_01FF").replace(
            "!0x007F_0FFF", "!0x01FF_0FFF"
        ),
    ),
    bite(
        "the decoder refuses everything, as it did before E3",
        BODY.replace("let raw = u32::try_from(token).ok()?;", "return None;\n        #[allow(unreachable_code)]\n        let raw = u32::try_from(token).ok()?;"),
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
        for i, (name, _, _, expect) in enumerate(BITES):
            print(f"{i:2d}  [{expect:10s}] {name}")
        return 0

    env = dict(os.environ)
    if not env.get("CARGO_TARGET_DIR"):
        env.pop("CARGO_TARGET_DIR", None)

    original = open(TARGET, encoding="utf-8").read()

    # ★ The clean tree must be GREEN in every arm, or a red bite proves nothing. And the
    # ORACLE arm must have actually RUN: on a box with no vendored ogkm tree it announces
    # `TOKEN-ORACLE-GATE: SKIPPED` and exits 0, so every bite would read as "missed by the
    # oracle" and the harness would produce a confidently wrong table.
    print("== baseline ==", flush=True)
    base = {k: run(c, env) for k, c in
            (("oracle", ORACLE_TEST), ("hardware", HARDWARE_TEST), ("mock", MOCK_TEST))}
    if any(r.returncode != 0 for r in base.values()):
        print("BASELINE IS NOT GREEN — every bite below would be meaningless.")
        for k, r in base.items():
            if r.returncode != 0:
                print(f"--- {k} ---\n{r.stdout.decode()[-3000:]}")
        return 2
    if b"TOKEN-ORACLE-GATE: RAN" not in base["oracle"].stdout:
        print(
            "THE ORACLE ARM DID NOT RUN. This box has no vendored open-kernel-modules "
            "tree, so `worksubmit_token_oracle.rs` SKIPPED and asserted nothing. Every "
            "bite would be reported as 'the oracle missed it', which is false and is "
            "exactly the kind of flattering table this project has been bitten by. Set "
            "KAYFABE_OGKM_580 and re-run."
        )
        return 2
    print("baseline: oracle GREEN (and RAN), hardware GREEN, mock GREEN\n", flush=True)

    results = []
    todo = range(len(BITES)) if args.only is None else [args.only]
    try:
        for i in todo:
            name, old, new, expect = BITES[i]
            n = original.count(old)
            if n != 1:
                print(f"{i:2d}  {name}\n    ★ ANCHOR MATCHED {n} TIMES — bite not applied")
                results.append((i, name, None, None, None, expect))
                continue
            if new == old:
                print(f"{i:2d}  {name}\n    ★ BITE IS A NO-OP — the replacement is the "
                      f"original, so this row would report a false GREEN")
                results.append((i, name, None, None, None, expect))
                continue
            with open(TARGET, "w", encoding="utf-8") as f:
                f.write(original.replace(old, new))
            os.utime(TARGET, None)
            time.sleep(0.01)
            red = {}
            for k, c in (("oracle", ORACLE_TEST), ("hardware", HARDWARE_TEST),
                         ("mock", MOCK_TEST)):
                red[k] = run(c, env).returncode != 0
            caught_by = [k for k in ("oracle", "hardware", "mock") if red[k]]
            note = ""
            if expect == EQUIVALENT:
                note = (
                    "  [equivalent — GREEN is correct]"
                    if not caught_by
                    else "  ★★★ AN EQUIVALENT MUTANT WENT RED — the equivalence argument "
                    "in this row's name is WRONG"
                )
            elif not caught_by:
                note = "  ★★ MISSED BY EVERYTHING"
            elif caught_by == ["hardware"]:
                note = "  <== HARDWARE ALONE"
            elif caught_by == ["oracle"]:
                note = "  <== ORACLE ALONE"
            print(
                f"{i:2d}  oracle={'RED ' if red['oracle'] else 'GREEN'} "
                f"hw={'RED ' if red['hardware'] else 'GREEN'} "
                f"mock={'RED ' if red['mock'] else 'GREEN'}  {name}{note}",
                flush=True,
            )
            results.append(
                (i, name, red["oracle"], red["hardware"], red["mock"], expect)
            )
    finally:
        with open(TARGET, "w", encoding="utf-8") as f:
            f.write(original)
        os.utime(TARGET, None)

    applied = [r for r in results if r[2] is not None]
    live = [r for r in applied if r[5] == RED]
    equiv = [r for r in applied if r[5] == EQUIVALENT]
    caught = [r for r in live if r[2] or r[3]]
    missed = [r for r in live if not r[2] and not r[3]]
    # ★ An equivalent mutant that went RED means the equivalence claim is false. It is a
    # failure of this harness's own reasoning, and it must not be reported as a pass.
    broken_equiv = [r for r in equiv if r[2] or r[3] or r[4]]
    mock_caught = [r for r in live if r[4]]
    oracle_only = [r for r in live if r[2] and not r[3]]
    hw_only = [r for r in live if r[3] and not r[2]]
    print(
        f"\n{len(caught)}/{len(live)} live bites caught by the E3 guards "
        f"({len(oracle_only)} by the ORACLE alone, {len(hw_only)} by HARDWARE alone).\n"
        f"{len(mock_caught)}/{len(live)} caught by the pre-existing mock suite — "
        f"`execution_plane_increments.md` §2.1 predicts this is ZERO, and that prediction "
        f"is the reason the mock arm is here at all.\n"
        f"{len(equiv)} rows are EQUIVALENT MUTANTS (no test can catch them; they are "
        f"required to stay green and {len(broken_equiv)} did not)."
    )
    for r in missed:
        print(f"  MISSED: {r[0]:2d} {r[1]}")
    for r in broken_equiv:
        print(f"  EQUIVALENCE CLAIM FALSIFIED: {r[0]:2d} {r[1]}")
    if len(applied) != len(results):
        print("  ★ SOME BITES WERE NOT APPLIED — see the ANCHOR lines above.")
        return 2
    return 1 if (missed or broken_equiv) else 0


if __name__ == "__main__":
    sys.exit(main())
