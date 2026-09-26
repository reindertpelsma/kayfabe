#!/usr/bin/env bash
# ★★★★★ w754 — COLLECT THE WORKSPACE'S FAILING-TEST NAME SET, so a rung can prove it added
# none.
#
# ⊘ **The SET, never the COUNT.** `[measured, this campaign, repeatedly]` a count is
# compatible with one failure being fixed and another introduced, and that is exactly the
# trade a refactor makes by accident. The deliverable is a sorted list of names that `diff`
# can be run over.
#
# ⚠ **Run it on the SAME box with the SAME command on both revisions.** A baseline collected
# on another machine, another toolchain or another feature set is not a baseline; it is a
# second measurement being compared to the first
# (`a_rulings_date_is_part_of_the_citation`, applied to a test set).
#
# ⊘ Compile errors are captured too, and deliberately: a target that FAILS TO BUILD produces
# zero failing test names, which is indistinguishable from a target where everything passes.
# That is the `dlen=0` shape and it is the one way this instrument could lie in the
# flattering direction.
#
#   usage: bash scripts/bench/w754_test_set.sh <label>   (run ON the bench box)
set -uo pipefail
LABEL=${1:?usage: w754_test_set.sh <label>}
OUT=${OUT_DIR:-/workspace/bench}
REPO=${KAYFABE_REPO:-/root/kayfabe}
cd "$REPO" || exit 2
export PATH=$PATH:/root/.cargo/bin
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0

RAW="$OUT/w754_tests_${LABEL}.raw"
SET="$OUT/w754_tests_${LABEL}.set"
echo "W754_TESTSET_START label=$LABEL rev=$(git rev-parse HEAD) $(date -Is)" | tee "$OUT/w754_tests_${LABEL}.meta"
cargo test --workspace --no-fail-fast > "$RAW" 2>&1
rc=$?
echo "CARGO_RC=$rc" >> "$OUT/w754_tests_${LABEL}.meta"
# ⊘ Two kinds of line, one set: a named test that FAILED, and a target that could not be
# built at all. Both are "something is red here", and only naming both keeps a build break
# from reading as a clean sweep.
{
  grep -aE '^test .* \.\.\. FAILED$' "$RAW" | sed -E 's/^test (.*) \.\.\. FAILED$/TEST \1/'
  grep -aE '^error(\[E[0-9]+\])?: ' "$RAW" | sed -E 's/^/BUILD /' | sort -u
} | sort -u > "$SET"
echo "W754_TESTSET_TERMINATOR label=$LABEL names=$(wc -l < "$SET") rc=$rc $(date -Is)" \
  | tee -a "$OUT/w754_tests_${LABEL}.meta"
