#!/usr/bin/env bash
# Run the `stable` CI job's gate steps locally, EXTRACTED FROM ci.yml ITSELF.
#
# ★ Why this exists. On 2026-07-27 three consecutive pushes went out red, and twice the
# fix for a red CI was itself red — because I reasoned about whether a gate would match
# instead of running it. The gate commands were sitting in .github/workflows/ci.yml the
# whole time and take seconds. Agents reporting "ran the CI greps by hand" did not help:
# the job has grown past 20 named steps and nobody was running the whole set.
#
# ★ It extracts rather than duplicates. A hand-copied list of gates drifts from the real
# job silently, and a stale copy that passes is worse than no copy — same
# green-instrument-on-an-unexercised-path failure the test doctrine is about. There is
# one source of truth: ci.yml.
#
# Usage:
#   scripts/ci_gates.sh          # every gate step (skips build/test/clippy/fmt by default)
#   scripts/ci_gates.sh --all    # …including the slow cargo steps
#
# NOTE: this runs the GATE steps — the greps, ratchets and boundary checks. The cargo
# build/test/clippy/fmt steps are skipped unless --all, because you have usually just run
# them. `--all` is what to use before pushing a CI change.
set -uo pipefail
cd "$(dirname "$0")/.."

want_all=0
[ "${1:-}" = "--all" ] && want_all=1

# =====================================================================================
# ★★★ THE FLOOR. Two literals, and everything about how they are written is deliberate.
# =====================================================================================
#
# ★ WHY THIS EXISTS. On 2026-07-30 this script printed **"ALL GATES CLEAN (0 steps)"**
# and exited 0. A heredoc terminator at column 0 inside ci.yml had truncated the YAML
# block scalar it sat in; the file stopped parsing; the extractor below returned nothing;
# and the runner reported success **having run no gates at all**. The cause was fixed
# where it happened. The CLASS was not: an empty or shortened `steps` array still reported
# clean, so any future truncation, bad glob, mis-quoted expansion or early return
# reproduces the identical silent pass through a different door.
#
# This project has now been bitten four separate times by a gate that cannot fire. This is
# the purest form of it — the gate *runner* reporting success having run nothing — and it
# is the one that hides all the others.
#
# ★★ LITERALS, NOT DERIVED. The obvious "fix" is `[ ${#steps[@]} -ge 1 ]`, or comparing
# the count against something computed from the same extraction. Both are the same defect
# one level up: a floor derived from the thing it checks moves silently when that thing
# moves, which is exactly what a floor must not do. A literal is a FACT SOMEONE MUST
# CONSCIOUSLY CHANGE, and a diff that changes it is a diff a reviewer can see.
#
# ★★ BOTH MODES ARE PINNED, rather than pinning `--all` and deriving the default as
# "all minus the deferred ones". The relationship is real but it is not an invariant this
# script owns: the default mode skips heavy steps AND consumer steps, so deriving it means
# re-implementing the extractor's two skip rules in the checker — a second copy of the
# logic under test, which is how the ratchet went blind earlier today. Two independent
# facts, each cheap to re-measure with `scripts/ci_gates.sh [--all] | tail -1`.
#
# ★★ PLACED ABOVE THE EXTRACTOR ON PURPOSE. The construct that can be cut short is the
# python heredoc below; a floor living inside it would be removed by the same edit that
# empties the array. These are plain shell assignments, lexically before it, and the
# comparison runs BEFORE any gate does — so a truncated list aborts having printed nothing
# reassuring. `tests/tests/gate_runner_floor.rs` asserts from outside the script that both
# literals still exist and are still compared before the loop, so deleting the floor turns
# the suite red rather than turning this script quiet.
# ★ 17 -> 18: the VBIOS-ORACLE reached-count step (the tests that run NVIDIA's own VBIOS
# parser over our generated image). It READS /tmp/kayfabe-test.log, so it is a consumer
# step and is deferred out of the default mode by the extractor's own rule — which is why
# only the `--all` literal moves and `GATE_STEPS_FAST_MIN` stays where it was.
# ★ 18 -> 19 and 10 -> 11: the CLAIM-LEDGER step (docs/design/claim_ledger.md) — a
# measurement word must name its run rather than a source read. It reads only the tree,
# produces no artifact and consumes none, so unlike the VBIOS-ORACLE step it runs in BOTH
# modes and BOTH literals move together.
# ★ 19 -> 20: the GMMU-ORACLE reached-count step (the tests that run NVIDIA's own GMMU
# page-table format encoder over our GA10x decoder). Like the VBIOS-ORACLE step it READS
# the test log, so the extractor defers it out of the default mode — which is why only the
# `--all` literal moves and `GATE_STEPS_FAST_MIN` stays where it was.
# ★ 20 -> 21: the TOKEN-ORACLE reached-count step (the tests that judge
# `Ga10xArch::decode_doorbell` against NVIDIA's own work-submit-token encoder, increment
# E3). Like the other two oracle steps it READS the test log, so the extractor defers it
# out of the default mode — only the `--all` literal moves.
# ★ 21 -> 22: the PUSHBUFFER-ORACLE reached-count step (the tests that judge the GA10x
# pushbuffer and USERD decode against NVIDIA's own `DRF_NUM`/`DRF_DEF`/`DRF_VAL` packing
# over `clc56f.h`/`clc7b5.h`, increment E4). Same shape as the three above — it READS the
# test log, so the extractor defers it and only the `--all` literal moves.
# ⚠ E4 shipped WITHOUT this step and said so, because another agent held `ci.yml` and
# moving a pinned floor under them would have been worse. Until it existed, that whole
# oracle family could have vanished from CI and from a dev box at once with nothing red —
# which is the exact failure this floor was invented for.
GATE_STEPS_ALL_MIN=22
GATE_STEPS_FAST_MIN=11

# ★★ A PER-INVOCATION test log, MEASURED 2026-07-30.
#
# `ci.yml`'s test step writes a log and three reached-count steps read it. The path used to be
# the fixed `/tmp/kayfabe-test.log`, which is a SHARED name — invisible on GitHub, where every
# job owns its runner, and wrong on a real box, where two `--all` runs at once clobber each
# other and the gates then count the other run's tests. It was seen: a box with both vendored
# ogkm trees present reported `VBIOS-oracle gate: ran=0 skipped=0 total=0` and failed, having
# read a concurrent run's file.
#
# ★ Exported, not passed: the steps are extracted from ci.yml verbatim and run in child
# shells, so the environment is the only channel that reaches all four of them. ci.yml keeps
# `/tmp/kayfabe-test.log` as its default, so GitHub's behaviour is byte-identical and the
# extractor's `PRODUCED` substring still matches.
KAYFABE_TEST_LOG="${KAYFABE_TEST_LOG:-$(mktemp -t kayfabe-test.XXXXXX.log)}"
export KAYFABE_TEST_LOG
trap 'rm -f "$KAYFABE_TEST_LOG"' EXIT

deferred_note=$(mktemp)
# ★ `$( … )` and THEN `mapfile`, not `mapfile < <( … )`. The second form was here before
# and its `|| exit 2` was DECORATIVE: `mapfile`'s exit status is the builtin's, never the
# process substitution's, so a python traceback produced an empty array and a status of 0.
# That is the second dead guard found in this one function, and it is why the truncation
# above surfaced as "0 steps" rather than as "could not parse ci.yml".
steps_raw=$(python3 - "$want_all" 2>"$deferred_note" <<'PY'
import sys, yaml, json
want_all = sys.argv[1] == "1"
job = yaml.safe_load(open(".github/workflows/ci.yml"))["jobs"]["stable"]
heavy = ("cargo build", "cargo test", "cargo clippy", "cargo fmt")
# Steps that CONSUME an artifact a heavy step produces. Skipping the producer while
# running the consumer reports a failure that says nothing about the tree — which is
# exactly the kind of misleading red this script exists to prevent. Detected by the
# artifact path rather than by step name, so a rename cannot silently decouple them.
PRODUCED = "/tmp/kayfabe-test.log"
out, deferred = [], []
for step in job.get("steps", []):
    run = step.get("run")
    if not run:
        continue                      # uses: actions/checkout etc. — nothing to run locally
    name = step.get("name", "(unnamed)")
    is_heavy = any(run.strip().startswith(h) for h in heavy)
    consumes = PRODUCED in run and not is_heavy
    if not want_all and (is_heavy or consumes):
        if consumes:
            deferred.append(name)
        continue
    # ★ `working-directory` is part of the step, MEASURED 2026-07-30. One step
    # ("Format check (fuzz workspace)") carries `working-directory: fuzz`, and this
    # extractor dropped it — so locally that step ran `cargo fmt --all --check` at the
    # REPOSITORY ROOT: a second copy of the previous step, and the fuzz workspace's own
    # formatting was never checked by this runner at all. Two identical gates where there
    # should be two different ones is the same class as a gate that cannot fire, and it
    # hides in plain sight because both are green on a clean tree.
    out.append(json.dumps({
        "name": name, "run": run, "wd": step.get("working-directory", "."),
    }))
for name in deferred:
    print(f"__DEFERRED__{name}", file=sys.stderr)
print("\n".join(out))
PY
) || {
  echo "★ COULD NOT PARSE ci.yml (need python3 + pyyaml). The extractor said:"
  cat "$deferred_note"
  rm -f "$deferred_note"
  exit 2
}

steps=()
[ -n "$steps_raw" ] && mapfile -t steps <<<"$steps_raw"

# ---- THE FLOOR, checked before a single gate runs -----------------------------------
if [ "$want_all" -eq 1 ]; then
  floor=$GATE_STEPS_ALL_MIN
  mode="--all"
else
  floor=$GATE_STEPS_FAST_MIN
  mode="default"
fi
if [ "${#steps[@]}" -lt "$floor" ]; then
  echo "★★★ GATE LIST TRUNCATED — ${#steps[@]} step(s) extracted, floor is $floor for $mode mode."
  echo ""
  echo "This is NOT 'a gate failed'. It is the gate LIST itself being suspect: fewer"
  echo "steps came out of .github/workflows/ci.yml than this script is pinned to expect,"
  echo "so whatever those missing steps check has NOT been checked and nothing red would"
  echo "have said so. Do not read anything into the steps that did run."
  echo ""
  echo "Most likely causes, commonest first:"
  echo "  - ci.yml no longer parses as the YAML you think it does. A heredoc terminator"
  echo "    at column 0 inside a 'run: |' block ends the block scalar and truncates the"
  echo "    file from there — this exact bug is why the floor exists."
  echo "  - a step lost its 'run:' key, or the job was renamed away from 'stable'."
  echo "  - steps were deliberately removed. If so, LOWER THE PINNED LITERAL in this"
  echo "    script in the same commit, and say in the message which gate went and why."
  echo ""
  echo "Cross-check with:  python3 -c \"import yaml;yaml.safe_load(open('.github/workflows/ci.yml'))\""
  rm -f "$deferred_note"
  exit 3
fi

fail=0
for s in "${steps[@]}"; do
  name=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["name"])' "$s")
  run=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["run"])' "$s")
  wd=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1]).get("wd","."))' "$s")
  printf '\n=== %s\n' "$name"
  if [ "$wd" != "." ]; then printf '    (in %s/)\n' "$wd"; fi
  if ( cd "$wd" && bash -e -c "$run" ); then
    printf '    ok\n'
  else
    printf '    ★ FAILED\n'
    fail=1
  fi
done

printf '\n'
while IFS= read -r line; do
  [ -n "$line" ] && echo "  deferred (needs --all, it reads the test log): ${line#__DEFERRED__}"
done < "$deferred_note"
rm -f "$deferred_note"

if [ "$fail" -eq 0 ]; then
  # ★ The count and the floor are printed TOGETHER. The old line said only "(N steps)",
  # which reads as reassurance whatever N is — it said "(0 steps)" the day this script
  # reported a clean run of nothing. A number beside the bar it cleared is evidence; a
  # number on its own is decoration.
  echo "ALL GATES CLEAN (${#steps[@]} steps, floor $floor for $mode mode)"
  [ "$want_all" -eq 0 ] && echo "★ Before pushing a CI change, run with --all — it is the only mode that covers every step."
else
  echo "★ AT LEAST ONE GATE FAILED — do not push."
fi
exit "$fail"
