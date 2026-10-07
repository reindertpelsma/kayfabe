#!/usr/bin/env bash
# ★★★★★ w327 — LOCALISE THE ALLOCATION CLIFF, AND DELIVER THE CEILING AS A NUMBER.
#
#   usage: scripts/bench/w327_sweep.sh <tag> <bw-size-list> [env assignments...]
#
# w322 §6.5 measured *"every allocation >= 32 MiB in the FB-leaf chain dies rc=719"* at THREE
# sizes only — 4, 16, 32 — so **16 and 32 are adjacent points on its grid** and the sweep can
# say nothing between them. That gap is the whole question this rung opens with:
#
#   - a failure that first appears at **exactly 32 MiB** and not at 31 is a CONSTANT;
#   - a failure that already appears at 17 or 20 MiB is EMERGENT, and 32 was an artefact of
#     w322's power-of-two grid.
#
# ⇒ ★★★ THE FIRST MEASUREMENT IS A DENSER GRID, NOT A FIX. It costs one boot and it decides
#   which of the brief's shapes is even in play. `w322_operands.sh`'s `bw` arm already takes
#   `KAYFABE_BENCH_BW` from the environment, so no workload change is needed and the binary is
#   master's.
#
# ## ⚠ WHY THE LIST IS ASCENDING AND THE ROWS ARE INDEPENDENT
#
# The bw workload prints each row as THAT row ends and holds one buffer live at a time, so an
# ascending list gives the ceiling directly: the last `BWROW ... read_GBps=` line is the
# largest size that WORKED, and the first `UNMEASURED` line is the smallest that did not.
# ⊘ A refusal takes the CUDA context down (w322 §6.5), so every row after the first failure is
# `reason=alloc_failed` and says NOTHING about its own size. This script therefore reports the
# ceiling as an INTERVAL — `(last_ok, first_fail]` — and never as a single number.
#
# ## ⊘ WHAT THIS SCRIPT DOES NOT DO
#
# It changes no device code. Every arm is master's binary at the pinned base; the only things
# that move are `KAYFABE_BENCH_BW` and whatever assignments the caller adds.
set -uo pipefail
TAG=${1:?usage: w327_sweep.sh <tag> <bw-list> [env...]}
BW=${2:?usage: w327_sweep.sh <tag> <bw-list> [env...]}
shift 2 || true
for kv in "$@"; do export "${kv?}"; done

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w327}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w327}
export KAYFABE_TAG="$TAG"
export KAYFABE_BENCH_BW="$BW"
export PATH=/root/.cargo/bin:$PATH

# ⚠ THE ARCHIVE-AGE TRAP, PAID FOR BY w322: `build_qom_shim.sh` refuses an archive more than
#   30 minutes old, and on an UNCHANGED tree cargo has nothing to rebuild — so the second and
#   later arms of any long batch fail as a *build* refusal that reads like a device fault.
#   Touching a source that is genuinely compiled into the archive is the documented fix.
touch "$REPO/crates/kayfabe-qemu-raw/src/shim.rs"

# ⚠⚠ THE DIRT GATE, HOISTED — measured 2026-08-14, and it cost FIVE ARMS IN 43 SECONDS.
#   `w290p_run.sh:50` refuses a dirty tree with rc=91, and the FIRST build of a fresh clone
#   rewrites `Cargo.lock`. So arm 1 of a batch boots normally and every LATER arm dies as a
#   *tree* refusal — while the batch writes its terminator and exits 0. ⊘ The failure is
#   silent exactly where the CLAUDE.md trap says to look: a complete, healthy-looking
#   artefact that measured nothing.
# ★ Checked HERE so the refusal names itself before ~40 s of build and boot are spent, and so
#   `⊘UNMEASURED` in the ceiling block can never be confused with "the sweep ran and found no
#   ceiling".
DIRT=$(cd "$REPO" && git status --porcelain --untracked-files=no)
if [ -n "$DIRT" ]; then
  echo "=== ⊘⊘ W327 REFUSES TO BOOT: THE TREE IS DIRTY, and w290p_run.sh would exit 91 anyway."
  echo "$DIRT" | sed 's/^/===   /'
  echo "W327_SWEEP_TERMINATOR tag=$TAG rc=91 DIRTY-TREE $(date -Is)"
  exit 91
fi

echo "=== ★★★★★ W327 SWEEP tag=$TAG $(date -Is)"
echo "===   bw=[$BW]"
echo "===   HEAD=[$(cd "$REPO" && git rev-parse HEAD)]"
echo "===   DIRT=[$(cd "$REPO" && git status --porcelain --untracked-files=no | head -3)]"
echo "===   KAYFABE_DRAIN_BATCH=[${KAYFABE_DRAIN_BATCH:-⊘unset ⇒ off ⇒ master behaviour}]"
echo "===   KAYFABE_VAS_DRAIN_ROW_LIMIT=[${KAYFABE_VAS_DRAIN_ROW_LIMIT:-⊘unset ⇒ 65536 default}]"
echo "===   KAYFABE_VAS_DRAIN_BUDGET_MS=[${KAYFABE_VAS_DRAIN_BUDGET_MS:-⊘unset ⇒ default}]"
echo "===   extra env: $* "

pkill -x qemu-system-x86 2>/dev/null; sleep 4
bash "$REPO/scripts/bench/w322_operands.sh" "${W327_ARM:-bw}"
RC=$?

P=/workspace/bench/run_${TAG}_probe.log
Q=/workspace/bench/run_${TAG}_qemu.log
D=/workspace/bench/run_${TAG}_hostdmesg.log

echo ""
echo "=== ★★★★★ W327 CEILING — the interval, never a single number"
OK=$(grep -ah 'BWROW ' "$P" 2>/dev/null | grep -a 'read_GBps=' | sed -n 's/.*BWROW mib=\([0-9]*\) .*/\1/p' | sort -n | tail -1)
FAIL=$(grep -ah 'BWROW ' "$P" 2>/dev/null | grep -a 'UNMEASURED' | sed -n 's/.*BWROW mib=\([0-9]*\) .*/\1/p' | sort -n | head -1)
echo "    W327_LAST_OK_MIB=${OK:-⊘UNMEASURED}"
echo "    W327_FIRST_FAIL_MIB=${FAIL:-⊘NONE_FAILED}"
echo "    W327_ASKED=[$BW]"
echo "    ⊘ a row printed UNMEASURED *after* the first failure is a dead context, not a"
echo "      statement about its own size. Only the FIRST failure is a datum."
echo "    rows, verbatim:"
grep -ah 'BWROW ' "$P" 2>/dev/null | sed 's/^ */      /' || echo "      ⊘ NO BWROW LINES — UNMEASURED"
echo "    fill/alloc refusals, verbatim:"
grep -ah 'BW_FILL_FAIL\|BW_ALLOC_FAIL' "$P" 2>/dev/null | sed 's/^ */      /' || echo "      (none)"

echo ""
echo "=== ★★ THE DEVICE SIDE — anchored; an absent line prints UNMEASURED and never 0"
echo "    W327_XID_HOST=[$(grep -ac 'Xid' "$D" 2>/dev/null)] (host dmesg)"
echo "    W327_XID_QEMU=[$(grep -ac 'Xid' "$Q" 2>/dev/null)]"
echo "    drain budget lines:"
grep -ao 'DRAIN-TIMING max_drain_us=[0-9]* disposed=[0-9]* residue=[0-9]* turns=[0-9]* budget_hit=[a-z]*' "$Q" 2>/dev/null \
  | sort | uniq -c | sed 's/^/      /' || echo "      ⊘ NONE — UNMEASURED"
echo "    publish census, last 3:"
grep -ao 'total=[0-9]* already_host=[0-9]* already_pinned=[0-9]* guest_ram=[0-9]* not_vidmem=[0-9]* not_granular=[0-9]*[^]]*' "$Q" 2>/dev/null \
  | tail -3 | sed 's/^/      /' || echo "      ⊘ NONE — UNMEASURED"
echo "    publish leaf-budget cap (VAS_PUBLISH_LEAF_BUDGET=4096) firings:"
grep -ao 'capped=[0-9]*' "$Q" 2>/dev/null | sort | uniq -c | sed 's/^/      /' || echo "      ⊘ NONE — UNMEASURED"
echo "    W327_JOINED=[$(grep -ao 'joined=[0-9]*' "$Q" 2>/dev/null | tail -1)]"
echo "    W327_HOSTROWS=[$(grep -aoE 'host_rows=[0-9]+' "$Q" 2>/dev/null | tail -1)]"
echo "    named host refusals seen in the boot:"
grep -aoE 'FbLeaf[A-Za-z]+|PlacementRefused|NoMemory|InsufficientResources|BadGpa|CrossesEnd|Miss' "$Q" 2>/dev/null \
  | sort | uniq -c | sed 's/^/      /' || echo "      (none)"

echo "W327_SWEEP_TERMINATOR tag=$TAG rc=$RC $(date -Is)"
exit $RC
