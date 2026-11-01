#!/bin/bash
set -euo pipefail
exec >/root/prov/b1_build.log 2>&1
trap 'rc=$?; echo "B1_BUILD_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "B1_BUILD_START GPU-free preparatory-only $(date -u -Is)"
export PATH=/root/.cargo/bin:$PATH
cd /root/kayfabe
git fetch --depth 1 origin 2add71e8ca8ebb80c419ed2fdb0427dfcdc0ccd4
git worktree add --detach /workspace/bw-b1-optional 2add71e8ca8ebb80c419ed2fdb0427dfcdc0ccd4
cd /workspace/bw-b1-optional
export CARGO_TARGET_DIR=/workspace/b1-optional-target CARGO_BUILD_JOBS=16
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-b1-optional
[ -z "$(git status --porcelain --untracked-files=no)" ]
python3 - <<'PY'
from pathlib import Path
import hashlib,json,datetime
p=Path('/workspace/bench/kf3-bins/2add71e8/qemu-system-x86_64')
r={'enabled':True,'tests_verified_before_arms':True,'source':'2add71e8ca8ebb80c419ed2fdb0427dfcdc0ccd4','parents':['8f3a9bb3a5d652b3f536ca015bf95735ec588924','1c6452a276d7014bece3adf42d68cbd4edffbc18'],'binary':str(p),'binary_sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'verification':'merged source1127/0/4 in3crates; B1 full3999/0/22 and15mutants>=2killers; hardwareUNRUN','scope':'includes B1 CI-repair parent, not RC-only causality'}
Path('/root/prov/optional_b1.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r))
PY
