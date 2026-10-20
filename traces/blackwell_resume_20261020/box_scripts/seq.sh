#!/bin/bash
# usage: seq.sh run1:prod:profile run2:prod:profile ...
exec >>/root/prov/seq.log 2>&1
echo "SEQ_START $* $(date -u -Is)"
for spec in "$@"; do
  IFS=: read -r run prod prof <<<"$spec"
  bash /root/bwr/arm.sh "$prod" "$prof" "$run"
  echo "SEQ_ARM_DONE $run $(tail -1 /root/prov/arm_${run}.log) $(grep -o 'BW580_RESULT.*' /root/prov/arm_${run}_diag.txt 2>/dev/null)"
  if pgrep -x qemu-system-x86 >/dev/null; then echo "SEQ_STOP qemu still running after $run"; exit 3; fi
done
echo "SEQ_EXIT $(date -u -Is)"
