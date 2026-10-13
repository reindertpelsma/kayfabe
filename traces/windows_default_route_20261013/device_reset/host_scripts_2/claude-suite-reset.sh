#!/usr/bin/env bash
set -uo pipefail
export PATH=$HOME/.cargo/bin:$PATH
D=/data/kf-reset-host-bf510bdf4; OUT=/data/claude-reset-validate-bf510bdf; cd $D
echo "SUITE_START $(date -Is) rev=$(git rev-parse --short=8 HEAD)" | tee $OUT/summary2.txt
KF_DEVICE=kf3 bash scripts/fastguest/fast_suite.sh resetval2 180 > $OUT/suite2.out 2>&1; echo "SUITE_RC=$?" | tee -a $OUT/summary2.txt
grep -a "FAST_SUITE_PASS" $OUT/suite2.out | tail -1 | tee -a $OUT/summary2.txt
echo "SUITE_END $(date -Is)" | tee -a $OUT/summary2.txt
