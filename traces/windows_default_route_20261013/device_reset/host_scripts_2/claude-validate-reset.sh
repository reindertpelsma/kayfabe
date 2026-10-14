#!/usr/bin/env bash
# Trusted host. Regression gates for the reset build (bf510bdf): v3_gates then the 30-arm fast suite, serial, own log.
set -uo pipefail
export PATH=$HOME/.cargo/bin:$PATH
D=/data/kf-reset-host-bf510bdf4; OUT=/data/claude-reset-validate-bf510bdf; mkdir -p $OUT; cd $D
echo "VALIDATE_START $(date -Is) rev=$(git rev-parse --short=8 HEAD)" | tee $OUT/summary.txt
flock /tmp/kayfabe-fastguest.lock bash -c "
  bash scripts/bench/v3_gates.sh $OUT/gates.log > $OUT/gates.out 2>&1; echo GATES_RC=\$? ;
  grep -a 'V3_GATES_SUMMARY' $OUT/gates.log | tail -1 ;
  KF_DEVICE=kf3 bash scripts/fastguest/fast_suite.sh resetval 180 > $OUT/suite.out 2>&1; echo SUITE_RC=\$? ;
  grep -a 'FAST_SUITE_PASS' $OUT/suite.out | tail -1" 2>&1 | tee -a $OUT/summary.txt
echo "VALIDATE_END $(date -Is)" | tee -a $OUT/summary.txt
