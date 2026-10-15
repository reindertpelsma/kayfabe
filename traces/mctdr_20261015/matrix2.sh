#!/bin/bash
# matrix.sh : fresh boot per entry, one idle 150 s Minecraft trial each (control = no PresentMon, p = PresentMon at +36 s). Run on the host, detached.
cd /data/claude-mctdr/ev
declare -A CNT START
START[eb9c]=1
entry() {
  local rev=$1 cond=$2; local key="${rev:0:3}$cond"; CNT[$key]=$(( ${CNT[$key]:-0} + 1 )); local n=$(( ${CNT[$key]} + ${START[$key]:-0} ))
  local q; q=$(sed -n 's/^QPID=//p' /var/lib/kf-windows-20261005/windows-broker-run2990.state)
  if [ -n "$q" ] && kill -0 $q 2>/dev/null; then bash vmstop.sh "before-F${key}${n}" > /dev/null 2>&1 || { echo "vmstop failed, abort"; exit 9; }; fi
  bash vmstart.sh $rev 0 || { echo "vmstart failed"; exit 8; }
  bash waitguest.sh
  if [ "$cond" = p ]; then PM=1 NODRIVE=1 bash trial.sh F$key $n agent 150 > trials/F${key}_t$n.out 2>&1
  else NODRIVE=1 bash trial.sh F$key $n agent 150 > trials/F${key}_t$n.out 2>&1; fi
  echo "ENTRY $rev $cond n=$n: $(cat trials/F${key}_t$n/result.txt 2>/dev/null) $(date +%T)"
}
echo "MATRIX START $(date -Is)"
until grep -q TRIAL_DONE trials/Feb9c_t1.out 2>/dev/null; do sleep 5; done; echo "ENTRY eb9f2aa3 c n=1: $(cat trials/Feb9c_t1/result.txt)"
for e in "eb9f2aa3 c" "2f99e673 c" "eb9f2aa3 c" "666ea966 p" "eb9f2aa3 p" "a07a9ba0 p" "eb9f2aa3 c" "2f99e673 c"; do entry $e; done
echo "MATRIX END $(date -Is)"
