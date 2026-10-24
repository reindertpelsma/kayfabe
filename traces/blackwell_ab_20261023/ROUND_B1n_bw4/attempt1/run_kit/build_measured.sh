#!/bin/bash
set -euo pipefail
exec >/root/prov/measured_build.log 2>&1
trap 'rc=$?; echo "MEASURED_BUILD_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "MEASURED_BUILD_START GPU-free preparatory-only $(date -u -Is)"
export PATH=/root/.cargo/bin:$PATH
cd /root/kayfabe
git fetch --depth 1 origin 48b027267a0e05c9ffdb4a049d34f4d719fd1837
git worktree add --detach /workspace/bw-measured 48b027267a0e05c9ffdb4a049d34f4d719fd1837
cd /workspace/bw-measured
export CARGO_TARGET_DIR=/workspace/measured-target CARGO_BUILD_JOBS=8
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-measured
[ -z "$(git status --porcelain --untracked-files=no)" ]
python3 - <<'RECEIPT'
from pathlib import Path
import hashlib,json,datetime
p=Path('/workspace/bench/kf3-bins/48b02726/qemu-system-x86_64')
r={'source':'48b027267a0e05c9ffdb4a049d34f4d719fd1837','binary':str(p),
'binary_sha256':hashlib.sha256(p.read_bytes()).hexdigest(),
'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
'tests_verified_before_arms':True,'separate_target':True,
'verification':'kf-rm849/kf-qemu419/kf-disp324+5ignored; 13 mutants >=2 killers; clippy added0',
'cpu_evidence':'traces/blackwell_consttab_20261023/README.md',
'scope':'measured consttab page only, no B1 merge'}
Path('/root/prov/measured_binary.json').write_text(json.dumps(r,indent=2)+'\n')
print(json.dumps(r))
RECEIPT
