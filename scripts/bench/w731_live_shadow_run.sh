#!/usr/bin/env bash
# ★★★★★ w731 — BUILD AND BOOT THE LIVE WALK SHADOW. Run ON the bench box.
#
#   usage: bash scripts/bench/w731_live_shadow_run.sh [tag]
#
# ⊘ Lives in the repo and not on the box: `vast is compute, never storage` — a harness that
# exists only on a rented machine is gone the moment it is destroyed.
#
# It does three things, in order, and each one refuses loudly rather than continuing:
#   1. build the archive WITH `cuda-scratchpad` — without it the second, glibc-linked isolate
#      image is EMPTY and the shadow answers `CUDA_WALK=ABSENT`. The script checks the built
#      binary BY CONTENT, because that difference is invisible in an exit status.
#   2. boot the CONTROL arm (`SHADOW=off`) — the shipped path, so the armed arm has something
#      to be compared against. ⊘ A single armed boot cannot tell "the shadow cost the guest
#      nothing" from "this box was having a bad day".
#   3. boot the ARMED arm (`SHADOW=on`).
#
# ⚠ Traps encoded inline:
#   - the kill goes on a line of ITS OWN, in its own command: `pkill -f '[q]emu-system-x86_64'`
#     matches its own shell if any later word on the same line names the binary (nvkvm-pv,
#     2026-08-17), and everything after the pkill then silently never runs.
#   - `pgrep -x qemu-system-x86_64` can NEVER match (/proc/PID/comm truncates at 15).
#   - `grep -c` on its own line, never piped into `grep -q` (a closing pipe returns 141 under
#     pipefail and MANUFACTURES a failure when the string IS there).
set -uo pipefail
REPO=${KAYFABE_REPO:-/root/kayfabe}
BENCH=${BENCH_DIR:-/workspace/bench}
TAG=${1:-w731}
cd "$REPO" || { echo "⊘ no repo at $REPO"; exit 2; }

echo "=== W731 LIVE SHADOW RUN $(date -Is) tag=$TAG ==="
echo "TREE_REV=$(git rev-parse HEAD)"

# ── 1. the build, WITH the feature ──────────────────────────────────────────────────────
export KAYFABE_SHIM_FEATURES="host-isolates cuda-scratchpad"
echo "== KAYFABE_SHIM_FEATURES=$KAYFABE_SHIM_FEATURES"
bash scripts/build_qom_shim.sh > "$BENCH/w731_build.log" 2>&1
rc=$?
echo "SHIM_RC=$rc"
if [ "$rc" -ne 0 ]; then
  echo "⊘ BUILD FAILED — the last 40 lines:"; tail -40 "$BENCH/w731_build.log"; exit 3
fi

Q_BIN="$BENCH/qemu-build/qemu-system-x86_64"
n_ws=$(strings "$Q_BIN" 2>/dev/null | grep -c 'WALK-SHADOW')
n_img=$(strings "$Q_BIN" 2>/dev/null | grep -c 'kayfabe-isolate-cuda')
echo "W731-CONTENT: walk_shadow_strings=$n_ws cuda_image_symbols=$n_img"
if [ "$n_ws" -eq 0 ]; then echo "⊘ the binary has no WALK-SHADOW strings — wrong tree"; exit 4; fi
if [ "$n_img" -eq 0 ]; then
  echo "⊘ built WITHOUT cuda-scratchpad: the shadow CANNOT run and would answer ABSENT."
  exit 5
fi

# ── 2/3. the two boots, control first ───────────────────────────────────────────────────
for arm in off on; do
  pkill -f '[q]emu-system-x86'
  sleep 3
  echo
  echo "############ ARM=$arm ############"
  PREFIX="${TAG}${arm}" SHADOW="$arm" bash scripts/bench/single_store_e6_boot.sh \
    2>&1 | tee "$BENCH/w731_${arm}.log"
done

echo
echo "=== W731 SUMMARY ==="
for arm in off on; do
  f="$BENCH/w731_${arm}.log"
  echo "-- arm=$arm"
  grep -aE '^\[client\]|^E6-|^HOST_DMESG_XID' "$f" 2>/dev/null | cut -c1-200
done
echo "=== W731 END $(date -Is) ==="
