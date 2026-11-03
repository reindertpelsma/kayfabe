#!/bin/bash
set -euo pipefail
kind=${1:?raw or cuda}
version=${2:?580 or 595}
[[ "$kind" = raw || "$kind" = cuda ]]
[[ "$version" = 580 || "$version" = 595 ]]
exec >"/root/prov/sweep_linux_${kind}${version}.log" 2>&1
trap 'rc=$?; echo "LINUX_ARM_EXIT kind=$kind version=$version rc=$rc $(date -u -Is)"' EXIT
export PATH=/root/.cargo/bin:$PATH
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = e8553e6ce7ec557aef9ff96c3c876e3060a6b9f8 ]
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
[ ! -e "/root/prov/sweep_linux_${kind}${version}.started" ]
[ "$(date -u +%s)" -lt "$(date -u -d '2026-10-20T01:15:00Z' +%s)" ]
date -u -Is >"/root/prov/sweep_linux_${kind}${version}.started"
while IFS= read -r key; do unset "$key"; done < <(compgen -A variable KF3_)
export KF_DEVICE=kf3 QEMU_BIN=/workspace/bench/kf3-bins/e8553e6c/qemu-system-x86_64
export KF_LADDER=/root/kayfabe/target/release/kayfabe-rm-ladder
export KF_FASTGUEST_DIR=/workspace/bench/fastguest${version}
echo "LINUX_ARM_START kind=$kind actual_guest_axis=$version source=$(git rev-parse HEAD) $(date -u -Is)"
echo 'FALSIFIER any grade/value/driver mismatch, timeout, crash, missing marker, guestXid or flip timeout; no performance ratio at n=1.'
sha256sum "$QEMU_BIN"
tag=sweepe855_ga104_${kind}${version}
if [ "$kind" = raw ]; then
 bash scripts/fastguest/fast_suite.sh "$tag" 180
 cat "/workspace/bench/${tag}_suite.out"
 grep '^FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30$' "/workspace/bench/${tag}_suite.out"
else
 if [ "$version" = 580 ]; then export KF_GUEST_IMG=/workspace/bench/sweep-guest580.qcow2; else export KF_GUEST_IMG=/workspace/bench/sweep-guest595-flat.qcow2; fi
 qemu-img info --output=json "$KF_GUEST_IMG" >"/root/prov/sweep_linux_cuda${version}_image.json"
 python3 - "$version" <<'PY'
import json,sys
j=json.load(open('/root/prov/sweep_linux_cuda'+sys.argv[1]+'_image.json'))
assert 'backing-filename' not in j,'CUDA image must be standalone, no mutable backing parent'
PY
 cat "$KF_GUEST_IMG.STAGED"
 bash scripts/bench/cuda_ladder.sh guest "$tag" 1
 python3 - "$tag" <<'PY'
from pathlib import Path
import sys
rows=[s for s in Path('/workspace/bench/cl_'+sys.argv[1]+'_guest.out').read_text().splitlines() if s.startswith('CL_ROW ')]
assert len(rows)==4 and all('verdict=PASS ' in x and 'boot_rc=0 ' in x for x in rows),rows
print('GUEST_CUDA4_PASS n=1 spread=N/A; value checks required by each rung')
PY
fi
echo "LINUX_ARM_COMPLETE kind=$kind version=$version $(date -u -Is)"
