#!/bin/bash
# driver.sh <stage...> — run R4 stages strictly serially; one log per stage; DRIVER_EXIT at the end.
exec >>/root/r4/driver.log 2>&1
echo "DRIVER_START $(date -Is) stages=$*"
trap 'echo "DRIVER_EXIT rc=$? $(date -Is)"' EXIT
for s in "$@"; do
  case $s in
    s0) f=s0_prov.sh; a="";; s1) f=s1_bm.sh; a="";; s2off) f=s2_apps.sh; a=off;; s2on) f=s2_apps.sh; a=on;;
    s4off) f=s4_gfx.sh; a=off;; s4on) f=s4_gfx.sh; a=on;; s5) f=s5_k49.sh; a="";; *) echo "unknown stage $s"; exit 2;;
  esac
  [ -e /root/r4/STOP ] && { echo "STOP file present before $s"; exit 9; }
  echo "STAGE $s START $(date -Is)"
  bash /root/r4/$f $a > /root/r4/stage_$s.log 2>&1
  echo "STAGE $s RC=$? $(date -Is)"
  grep -a '_EXIT\|RESULT\|GSET_SUITE_VERDICT\|GSET_BM1\|^ND \|CYCLE .* TIMER \|NOT_READY\|DIED' /root/r4/stage_$s.log | tail -12
done
