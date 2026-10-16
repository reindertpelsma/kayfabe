#!/usr/bin/env bash
# Mutation check of the Windows USER-ring lifecycle (docs/design/V3_USER_RING_LIFECYCLE_20261016.md section 8).
#
# For every deliberate defect (a machine mutant, `kf_chan::user_lifecycle::Mutant`, selected with
# KF_LIFECYCLE_MUTANT, or a glue defect of the test transliteration, selected with KF_LIFECYCLE_GLUE_MUTANT)
# this re-runs the GPU-free lifecycle tests with the `lifecycle-mutants` feature and lists the tests that
# FAIL. A mutant no test catches is a missing test. The unmutated run must be green first.
#
# usage: scripts/lifecycle_mutation_check.sh [outdir]       (set CARGO_TARGET_DIR to keep the build elsewhere)
# GPU-free; never touches hardware.
set -u
cd "$(dirname "$0")/.."
OUT=${1:-/tmp/lifecycle_mutation}
mkdir -p "$OUT"
export KF3_STRESS_SECS=${KF3_STRESS_SECS:-1}

TESTS=(--lib --test user_lifecycle_model --test user_lifecycle_props --test user_lifecycle_stress)

run_one() { # name, "VAR=value" or ""
  local name=$1 envset=$2
  if [ -n "$envset" ]; then
    env "$envset" cargo test -j2 --release --no-fail-fast -p kf-chan --features lifecycle-mutants "${TESTS[@]}" >"$OUT/$name.log" 2>&1
  else
    cargo test -j2 --release --no-fail-fast -p kf-chan --features lifecycle-mutants "${TESTS[@]}" >"$OUT/$name.log" 2>&1
  fi
  local rc=$?
  grep -E "^test .* FAILED$" "$OUT/$name.log" | sed 's/^test //; s/ \.\.\. FAILED$//' | sort -u >"$OUT/$name.failing"
  printf '%s\trc=%s\tfailing=%s\n' "$name" "$rc" "$(wc -l <"$OUT/$name.failing")"
  sed 's/^/    /' "$OUT/$name.failing"
}

echo "== baseline (no mutant): must be green"
run_one baseline ""

echo "== machine mutants (KF_LIFECYCLE_MUTANT)"
for m in pointer_scans no_seed forget_sync pointer_to_scan revive_after_teardown exit_ignores_live mask_frozen; do
  run_one "machine_$m" "KF_LIFECYCLE_MUTANT=$m"
done

echo "== glue mutants (KF_LIFECYCLE_GLUE_MUTANT)"
for m in epoch_before_put double_fetch no_gate_reset flip_at_put ignore_seed ignore_events drop_flip_entry skip_validation bound_without_datum; do
  run_one "glue_$m" "KF_LIFECYCLE_GLUE_MUTANT=$m"
done
