#!/bin/bash
exec >>/root/prov/q2.log 2>&1
echo "Q3_START $(date -u -Is)"
noq(){ for i in $(seq 1 60); do pgrep -x qemu-system-x86 >/dev/null || return 0; sleep 2; done; echo "Q3_QEMU_STUCK"; exit 4; }
pan(){ grep -q GUEST_PANICKED /workspace/bwr-win/kfwin/logs/$1_run.log 2>/dev/null; }
for spec in fix4_off_1:fix4:off:240 fix5_on_1:fix5:on:180 fix4_off_2:fix4:off:240 fix5_on_2:fix5:on:180; do
  IFS=: read -r run prod prof hold <<<"$spec"
  noq; ARM_HOLD=$hold bash /root/bwr/seq.sh "$run:$prod:$prof"
  noq; pan "$run" && { bash /root/bwr/collect.sh "$run"; noq; }
done
echo "Q3_EXIT $(date -u -Is)"
