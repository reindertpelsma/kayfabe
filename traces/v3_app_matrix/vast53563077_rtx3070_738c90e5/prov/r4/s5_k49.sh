#!/bin/bash
# s5_k49.sh — bounded §4.9 reproduction: 3 cycles of {thin-suite arm started; its QEMU SIGKILLed while the
# guest driver initialises (after the guest prints "NVRM: loading", + a per-cycle delay); then the --timer
# arm alone}. Records the victim's last console line and device phase at the kill, and the --timer verdict
# plus the startup-race signature ('no fd-backed guest RAM block') of every boot.
. /root/r4/common.sh
export KF_DEVICE=kf3
echo "S5_START $(date -Is) rev=$REV"
alive_qemu && { echo "A QEMU is up before s5"; exit 6; }
DELAYS=(x 0.2 0.7 1.2)
for c in 1 2 3; do
  d=${DELAYS[$c]}; V=r4k${c}v; T=r4k${c}t
  echo "CYCLE $c START $(date -Is) delay=${d}s"
  bash scripts/fastguest/fast_suite.sh $V 180 --timer > $L/k${c}_victim.run 2>&1 &
  VP=$!
  TTY=$B/fast_${V}_timer_ttyS0.log; seen=0
  for i in $(seq 1 900); do grep -aq 'NVRM: loading NVIDIA' "$TTY" 2>/dev/null && { seen=1; break; }; sleep 0.2; done
  echo "CYCLE $c NVRM_SEEN=$seen $(date -Is)"
  sleep "$d"
  pids=$(pgrep -x qemu-system-x86 | tr '\n' ' ')
  echo "CYCLE $c KILL -9 pids=[$pids] at $(date +%H:%M:%S.%N)"
  pkill -9 -x qemu-system-x86
  wait $VP; echo "CYCLE $c VICTIM_SUITE_RC=$?"
  echo "CYCLE $c VICTIM_LAST_CONSOLE: $(grep -a '^\[' "$TTY" | tail -1 | cut -c1-200)"
  echo "CYCLE $c VICTIM_PHASE: $(grep -ao 'phase=[A-Za-z]*' $B/fast_${V}_timer_qemu.log | tail -1) nvrm_lines=$(grep -ac NVRM "$TTY")"
  grep -a 'timer' $B/${V}_suite.out | sed "s/^/CYCLE $c VICTIM /"
  gpu_idle
  bash scripts/fastguest/fast_suite.sh $T 180 --timer > $L/k${c}_timer.run 2>&1
  echo "CYCLE $c TIMER_SUITE_RC=$?"
  grep -a 'FAST_SUITE_PASS\|timer' $B/${T}_suite.out | sed "s/^/CYCLE $c TIMER /"
  for q in $B/fast_${V}_timer_qemu.log $B/fast_${T}_timer_qemu.log; do
    echo "CYCLE $c $(basename $q) no_fd_ram=$(grep -ac 'no fd-backed guest RAM block' $q) no_ram_obj=$(grep -ac 'no RAM object' $q)"
  done
  echo "CYCLE $c TIMER_GUEST: timeout=$(grep -ac NV_ERR_TIMEOUT $B/fast_${T}_timer_ttyS0.log) ce_utils=$(grep -ac 'ce_utils.c' $B/fast_${T}_timer_ttyS0.log)"
done
echo "S5_EXIT rc=0 $(date -Is)"
