#!/bin/bash
# s4_gfx.sh <off|on> — the headless-graphics set in the fat guest, graded against THIS box's bare metal
# (r4bm, GSET_HOST_FROM) and the 14 bare-metal Cycles renders (GSET_NOISE_DIRS = nd/h1..h12).
. /root/r4/common.sh
M=${1:?off|on}
case $M in off) unset KF3_DEV_EXTRA ;; on) export KF3_DEV_EXTRA=$ON ;; *) exit 2 ;; esac
echo "S4_START $(date -Is) rev=$REV mode=$M KF3_DEV_EXTRA=${KF3_DEV_EXTRA:-<unset>}"
alive_qemu && { echo "A QEMU is up before s4"; exit 6; }
# ⊘ nd/h2 of s1 failed to attach its image (hostroot: /dev/nbd1p1 never appeared — the nbd partition race):
#   re-measure any bare-metal noise run that left no result, BEFORE the first guest run (still bare metal, serial).
if [ "$M" = off ]; then
  exec 9>/tmp/kayfabe-fastguest.lock; flock 9
  for k in $(seq 1 12); do [ -s $GR/nd/h$k/host.res ] || step nd_retry_h$k bash scripts/bench/gfxset/host.sh "$GR/nd/h$k" blender_cycles_cuda blender_cycles_optix; done
  flock -u 9; exec 9>&-
  echo "ND_AFTER_RETRY $(cat $GR/nd/h*/host.res | grep -c verdict=PASS)/$(cat $GR/nd/h*/host.res | grep -c '^GSET_RES')"
fi
ND=$(for d in $GR/nd/h*; do [ -s $d/host.res ] && echo -n "$d "; done)
echo "GSET_NOISE_DIRS=$ND"
step gset_${M} env GSET_HOST_FROM=r4bm GSET_NOISE_DIRS="$ND" bash scripts/bench/gfxset/suite.sh r4g${M}
grep -a 'GSET_SUITE_VERDICT' $GR/r4g${M}/suite.log
echo "S4_EXIT rc=0 $(date -Is)"
