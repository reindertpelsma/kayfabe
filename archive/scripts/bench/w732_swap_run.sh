#!/usr/bin/env bash
# ★★★★★ w732 — BUILD AND BOOT THE WALKER SWAP (`SINGLE_STORE_PLAN.md` §6 step 2).
# Run ON the bench box.
#
#   usage: bash scripts/bench/w732_swap_run.sh [tag]
#
# ⊘ Lives in the repo and not on the box: `vast is compute, never storage`.
#
# ## ★★ THE TWO ARMS, AND WHY THEY ARE `on` AND `swap` RATHER THAN `off` AND `swap`
#
# The control is **w731's own proven-clean arm**, not the shipped path. The variable under
# test is *"does the kernel's answer get PUBLISHED"* — everything else (the scratchpad, its
# CUDA context, the image staging, the round trip, the comparison) is identical on both. ⊘ An
# `off` control would differ in five things at once and could not attribute a difference to
# any of them.
#
# ## ⊘⊘ WHAT THIS RUN CAN AND CANNOT ESTABLISH — read before grading
#
# The swap is observationally neutral **by construction**: it substitutes only where the two
# walkers agree, and under agreement the two leaf sets are the same set. ⇒ a clean client on
# the `swap` arm is a **necessary** condition and not a sufficient one, and a parity number
# says nothing at all. What this run establishes is:
#   (a) the swap arm **boots and the raw client still passes** — i.e. nothing about putting the
#       kernel on the publish path breaks the guest;
#   (b) `decided=N` with **N > 0** — the kernel actually decided something, which is the only
#       thing that separates a live swap from a decider nobody consulted;
#   (c) every `fell_back` is **named**, and every one printed a `WALK-SWAP FALLBACK` line at
#       the moment it happened rather than at teardown.
# That the substitution reaches `AddressTable` at all is proved OFFLINE and deliberately so,
# in `tests/tests/walk_swap_decides.rs`, by a decider that disagrees on purpose.
#
# ⚠ Traps encoded inline:
#   - the kill goes on a line of ITS OWN, in its own command (nvkvm-pv, 2026-08-17).
#   - `pgrep -x qemu-system-x86_64` can NEVER match (/proc/PID/comm truncates at 15).
#   - `grep -c` on its own line, never piped into `grep -q` (SIGPIPE + pipefail = 141).
set -uo pipefail
REPO=${KAYFABE_REPO:-/root/kayfabe}
BENCH=${BENCH_DIR:-/workspace/bench}
TAG=${1:-w732}
cd "$REPO" || { echo "⊘ no repo at $REPO"; exit 2; }

echo "=== W732 WALKER SWAP RUN $(date -Is) tag=$TAG ==="
echo "TREE_REV=$(git rev-parse HEAD)"

export KAYFABE_SHIM_FEATURES="host-isolates cuda-scratchpad"
echo "== KAYFABE_SHIM_FEATURES=$KAYFABE_SHIM_FEATURES"
QEMU_SRC=${KAYFABE_QEMU_SRC:-$BENCH/qemu-10.2.4}
QEMU_BUILD=${KAYFABE_QEMU_BUILD:-$BENCH/qemu-build}
[ -f "$QEMU_SRC/VERSION" ] || { echo "⊘ no hypervisor source tree at $QEMU_SRC"; exit 2; }
echo "== qemu src=$QEMU_SRC build=$QEMU_BUILD"
bash scripts/build_qom_shim.sh "$QEMU_SRC" "$QEMU_BUILD" > "$BENCH/w732_build.log" 2>&1
rc=$?
echo "SHIM_RC=$rc"
if [ "$rc" -ne 0 ]; then
  echo "⊘ BUILD FAILED — the last 40 lines:"; tail -40 "$BENCH/w732_build.log"; exit 3
fi

Q_BIN="$BENCH/qemu-build/qemu-system-x86_64"
n_ws=$(strings "$Q_BIN" 2>/dev/null | grep -c 'WALK-SHADOW')
n_sw=$(strings "$Q_BIN" 2>/dev/null | grep -c 'WALK-SWAP FALLBACK')
n_img=$(strings "$Q_BIN" 2>/dev/null | grep -c 'kayfabe-isolate-cuda')
echo "W732-CONTENT: walk_shadow_strings=$n_ws walk_swap_strings=$n_sw cuda_image_symbols=$n_img"
# ★★★ CONTENT, not a stamp. A binary that predates step 2 accepts `KAYFABE_WALK_SHADOW=swap`
# nowhere — it REFUSES the value — so the symptom would be a refused realize, not a silent
# shadow run. Checked anyway: the refusal and "the arm did nothing" read the same in a summary.
if [ "$n_ws" -eq 0 ]; then echo "⊘ the binary has no WALK-SHADOW strings — wrong tree"; exit 4; fi
if [ "$n_sw" -eq 0 ]; then echo "⊘ the binary predates §6 step 2 — it cannot swap. STOP."; exit 4; fi
if [ "$n_img" -eq 0 ]; then
  echo "⊘ built WITHOUT cuda-scratchpad: the kernel CANNOT run and both arms measure nothing."
  exit 5
fi

for arm in on swap; do
  pkill -f '[q]emu-system-x86'
  sleep 3
  echo
  echo "############ ARM=$arm ############"
  PREFIX="${TAG}${arm}" SHADOW="$arm" bash scripts/bench/single_store_e6_boot.sh \
    2>&1 | tee "$BENCH/w732_${arm}.log"
done

echo
echo "=== W732 SUMMARY ==="
for arm in on swap; do
  f="$BENCH/w732_${arm}.log"
  echo "-- arm=$arm"
  grep -aE '^\[client\]|^E6-|^HOST_DMESG_XID' "$f" 2>/dev/null | cut -c1-220
done
echo "=== W732 END $(date -Is) ==="
