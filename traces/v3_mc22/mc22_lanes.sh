#!/bin/bash
# mc22_lanes.sh — the v3-mc22 lanes, strictly serial, AFTER the merge bar mc22b passed, all from the
# bar's own verify worktree (so every boot runs kf3-bins/<that rev>/). Every step has a start and
# an rc line; the log ends with LANES_EXIT.
set -u
LOG=/root/prov/mc22_lanes.log
exec >>"$LOG" 2>&1
trap 'echo "LANES_EXIT rc=$? $(date -Is)"' EXIT
echo "LANES_WAIT_FOR_BAR $(date -Is)"
until grep -q '^EXIT' /root/prov/mc22b.log 2>/dev/null; do
  pgrep -f '[m]erge_check.sh v3-mc22 mc22b' >/dev/null || { sleep 5; grep -q '^EXIT' /root/prov/mc22b.log || { echo "BAR_DIED (no EXIT line)"; exit 3; }; }
  sleep 15
done
grep -q '^EXIT rc=0' /root/prov/mc22b.log || { echo "BAR_FAILED: $(grep '^EXIT' /root/prov/mc22b.log)"; exit 4; }
grep -q '^FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30$' /workspace/bench/mc22b_suite.out || { echo "BAR_SUITE_NOT_30"; exit 5; }
W=$(sed -n 's/^HEAD=[0-9a-f]* CHECKOUT=//p' /root/prov/mc22b.log)
cd "$W" || { echo "NO_WORKTREE $W"; exit 6; }
REV=$(git rev-parse --short=8 HEAD)
[ -x "/workspace/bench/kf3-bins/$REV/qemu-system-x86_64" ] || { echo "NO_KF3_BIN $REV"; exit 7; }
export PATH="$HOME/.cargo/bin:$PATH" KF_DEVICE=kf3
echo "LANES_START $(date -Is) rev=$REV worktree=$W"
gpu_idle() { for _ in $(seq 1 120); do u=$(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits | head -1 | tr -dc 0-9); [ -n "$u" ] && [ "$u" -le 512 ] && return 0; sleep 1; done; echo "GPU_NOT_IDLE used=$u"; }
alive_qemu() { pgrep -x qemu-system-x86 >/dev/null || ss -tln | grep -q ':2223 '; }
# 1. the doorbell fast path ON: the 30-arm thin suite, and the CUDA ladder OFF then ON
echo "STEP dbfast START $(date -Is)"
bash scripts/bench/dbfast_lane.sh mc22dbl suite_on,ladder; echo "STEP dbfast RC=$? $(date -Is)"
tail -1 /workspace/bench/dbfast/mc22dbl.log
alive_qemu && echo "WARN a qemu is still up after dbfast"
gpu_idle
# 2. the display lane, fast path at its default (off)
echo "STEP disp_off START $(date -Is)"
bash scripts/bench/display/lane.sh mc22disp > /root/prov/mc22disp.lane.log 2>&1; echo "STEP disp_off RC=$? $(date -Is)"
grep -a '^DISPLAY_LANE_EXIT' /root/prov/mc22disp.lane.log
alive_qemu && echo "WARN a qemu is still up after disp_off"
gpu_idle
# 3. the display lane with the doorbell fast path ON
echo "STEP disp_on START $(date -Is)"
DISPLAY_KF3_EXTRA=doorbell-ioeventfd=on bash scripts/bench/display/lane.sh mc22dispon > /root/prov/mc22dispon.lane.log 2>&1; echo "STEP disp_on RC=$? $(date -Is)"
grep -a '^DISPLAY_LANE_EXIT' /root/prov/mc22dispon.lane.log
