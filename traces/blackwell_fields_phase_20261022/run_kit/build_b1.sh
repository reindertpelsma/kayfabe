#!/bin/bash
set -euo pipefail
exec >/root/prov/b1_build.log 2>&1
trap 'rc=$?; echo "B1_BUILD_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "B1_BUILD_START GPU-free preparatory-only $(date -u -Is)"
export PATH=/root/.cargo/bin:$PATH
cd /root/kayfabe
git fetch --depth 1 origin c130aa98ac2b922202282cd793f2a6d64be1f091
git worktree add --detach /workspace/bw-b1-optional c130aa98ac2b922202282cd793f2a6d64be1f091
cd /workspace/bw-b1-optional
export CARGO_TARGET_DIR=/workspace/b1-optional-target CARGO_BUILD_JOBS=16
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-b1-optional
[ -z "$(git status --porcelain --untracked-files=no)" ]
python3 - <<'PY'
from pathlib import Path
import hashlib,json,datetime
p=Path('/workspace/bench/kf3-bins/c130aa98/qemu-system-x86_64')
r={'enabled':True,'tests_verified_before_arms':True,'source':'c130aa98ac2b922202282cd793f2a6d64be1f091','parents':['6e3fd3fd6d809ae2c2a3504cd8be1ecd29b2b31b','178862f82bcf8296de0f7ccab298d4761055eaf2'],'binary':str(p),'binary_sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'verification':'CPU combined release host84/disp324/chan672/qemu436 failed0 ignored9; field policy plus B1; hardwareUNRUN','scope':'includes B1 CI-repair parent, not RC-only causality'}
Path('/root/prov/optional_b1.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r))
PY
