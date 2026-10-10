#!/bin/bash
set -euo pipefail
umask 077
export PATH=/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
exec 9>/tmp/kayfabe-fastguest.lock
flock -n 9
trap 'rc=$?; echo ILUT_BUILD_EXIT=$rc' EXIT
echo ILUT_BUILD_START="$(date -Is)"
cd /var/lib/kf-windows-20261005
[ ! -e kf3-bins/d2c7ca1b ]
git -C kayfabe worktree add --detach /var/lib/kf-windows-20261005/kayfabe-ilut d2c7ca1bd5c6f5be677ecd4a1c341ff4de8f6948
cd kayfabe-ilut
[ -z "$(git status --porcelain --untracked-files=no)" ]
export CARGO_BUILD_JOBS=6
export CARGO_TARGET_DIR=/var/lib/kf-windows-20261005/target
bash scripts/bench/build_kf3.sh /var/lib/kf-windows-20261005/qemu-10.2.4 /var/lib/kf-windows-20261005/qemu-build-kf3
python3 - <<'PY'
import json,hashlib
from pathlib import Path
b=Path('/var/lib/kf-windows-20261005');p=b/'kf3-bins/d2c7ca1b'
def sha(p):
 with p.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
r={'product_revision':'d2c7ca1bd5c6f5be677ecd4a1c341ff4de8f6948','observer_revision':'a8845e69847b969d01308a64f13e00394ec76e1d','qemu_sha256':sha(p/'qemu-system-x86_64'),'rust_archive_sha256':sha(b/'qemu-10.2.4/hw/misc/kf3/libkf_qemu.a'),'configure_flags':(b/'qemu-build-kf3/.kf3-configure').read_text()}
(p/'build-receipt.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r))
PY
