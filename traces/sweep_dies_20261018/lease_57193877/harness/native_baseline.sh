#!/bin/bash
set -euo pipefail
export PATH=/root/.cargo/bin:$PATH KF_DEVICE=kf3
cd /root/kayfabe
exec >/root/prov/sweep_native_baseline.log 2>&1
trap 'rc=$?; echo "NATIVE_BASELINE_EXIT rc=$rc $(date -u -Is)"' EXIT
[ "$(git rev-parse HEAD)" = 1700543393c34f93adc0611430e3d49ab38eca3a ]
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
echo "NATIVE_BASELINE_START $(date -u -Is)"
echo 'H_NATIVE falsifier any stock raw failure/crash or CUDA result mismatch; stop before guest attribution.'
export KF_LADDER=/root/kayfabe/target/release/kayfabe-rm-ladder
bash scripts/fastguest/bare_metal_suite.sh sweep17005433_ad102_native595 180 >/root/prov/sweep_bare595.log 2>&1
grep '^BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30$' /root/prov/sweep_bare595.log
bash scripts/bench/cuda_ladder.sh host sweep17005433_ad102 1 >/root/prov/sweep_cuda_host.log 2>&1
python3 - <<'CHECK'
from pathlib import Path
rows=[l for l in Path('/workspace/bench/cl_sweep17005433_ad102_host.out').read_text().splitlines() if l.startswith('CL_ROW ')]
assert len(rows)==4 and all('verdict=PASS ' in l and ' rc=0 ' in l for l in rows),rows
print('NATIVE_CUDA4_PASS n=1 spread=N/A')
CHECK
echo "NATIVE_BASELINE_COMPLETE $(date -u -Is)"
