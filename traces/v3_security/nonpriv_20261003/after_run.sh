#!/bin/bash
# AFTER measurement: one fast-guest boot, QEMU run as root exactly as the bench does.
# usage: after_run.sh <tag> [extra env assignments...]
T=$1; shift
cd /root/kayfabe || exit 2
echo "START $(date -Is) rev=$(git rev-parse --short=8 HEAD) dirty=$(git status --porcelain --untracked-files=no | wc -l)"
echo "launcher: euid=$(id -u) $(grep -E '^Cap(Prm|Eff)' /proc/self/status | tr '\t\n' '  ')"
( for i in $(seq 1 600); do p=$(pgrep -x qemu-system-x86 | head -1); if [ -n "$p" ]; then
      echo "qemu pid=$p $(grep -E '^(Uid|CapPrm|CapEff)' /proc/$p/status | tr '\t\n' '  ')"; break; fi; sleep 0.1; done ) &
env "$@" KF_ARMS="--probe-launch-dma" bash scripts/fastguest/run_fast_guest.sh "$T" 180
rc=$?
wait
echo "EXIT rc=$rc $(date -Is)"
