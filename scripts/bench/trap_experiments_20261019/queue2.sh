#!/usr/bin/env bash
# After queue.sh: the model-driven spinner arm, then the Linux-guest access-cost probes (caller holds the GPU lock).
ROOT=/data/claude-trapexp-20261019; cd $ROOT/scripts
while ! grep -q QUEUE_DONE $ROOT/queue.log; do sleep 5; done
BASE="KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5"
echo "ARM_START spinb $(date -Is)"
python3 arm.py --arm spinb --rev 1bd33400 --flags "$BASE KF3_DIAG_INTR_SPIN=2 KF3_DIAG_INTR_SPIN_CPU=23" --n 4107 > ../arms_spinb.out 2>&1 < /dev/null
echo "ARM_DONE spinb rc=$? $(date -Is)"
sleep 5
for spec in "ctl|02911cbb-ctl" "spin1|1bd33400|KF3_DIAG_INTR_SPIN=1" "spin2|1bd33400|KF3_DIAG_INTR_SPIN=2"; do
  IFS='|' read -r tag rev env <<<"$spec"
  echo "FG_START $tag $(date -Is)"
  bash fg_probe.sh $tag $rev $env KF3_DIAG_INTR_SPIN_CPU=23 > ../fgprobe_$tag.out 2>&1
  echo "FG_DONE $tag $(date -Is)"
done
echo QUEUE2_DONE $(date -Is)
