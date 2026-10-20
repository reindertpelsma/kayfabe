#!/bin/bash
set -euo pipefail
. /root/bwr/common.sh
export KF_DEVICE=kf3
cd /root/kayfabe
exec >/root/prov/bwr_native_baseline.log 2>&1
trap 'rc=$?; echo "NATIVE_BASELINE_EXIT rc=$rc $(date -u -Is)"' EXIT
[ "$(git rev-parse HEAD)" = $BASE ]
echo "NATIVE_BASELINE_START $(date -u -Is)"
echo 'H_NATIVE falsifier: any stock raw failure/crash or CUDA mismatch; stop before guest attribution.'
export KF_LADDER=/root/kayfabe/target/release/kayfabe-rm-ladder
bash scripts/fastguest/bare_metal_suite.sh bwr7f22d18d_gb202_native595 180 >/root/prov/bwr_bare595.log 2>&1
grep '^BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30$' /root/prov/bwr_bare595.log
bash scripts/bench/cuda_ladder.sh host bwr7f22d18d_gb202 1 >/root/prov/bwr_cuda_host.log 2>&1
python3 - <<'CHECK'
from pathlib import Path
rows=[l for l in Path('/workspace/bench/cl_bwr7f22d18d_gb202_host.out').read_text().splitlines() if l.startswith('CL_ROW ')]
assert len(rows)==4 and all('verdict=PASS ' in l and ' rc=0 ' in l for l in rows),rows
print('NATIVE_CUDA4_PASS n=1')
CHECK
echo "NATIVE_BASELINE_COMPLETE $(date -u -Is)"
