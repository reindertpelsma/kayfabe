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

deferred_note=$(mktemp)
mapfile -t steps < <(python3 - "$want_all" 2>"$deferred_note" <<'PY'
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
    out.append(json.dumps({"name": name, "run": run}))
for name in deferred:
    print(f"__DEFERRED__{name}", file=sys.stderr)
print("\n".join(out))
PY
) || { echo "could not parse ci.yml (need python3 + pyyaml)"; exit 2; }

fail=0
for s in "${steps[@]}"; do
  name=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["name"])' "$s")
  run=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["run"])' "$s")
  printf '\n=== %s\n' "$name"
  if bash -e -c "$run"; then
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
  echo "ALL GATES CLEAN (${#steps[@]} steps)"
  [ "$want_all" -eq 0 ] && echo "★ Before pushing a CI change, run with --all — it is the only mode that covers every step."
else
  echo "★ AT LEAST ONE GATE FAILED — do not push."
fi
exit "$fail"
