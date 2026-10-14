#!/bin/bash
set -euo pipefail
umask 077
export PATH=/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
exec 9>/tmp/kayfabe-fastguest.lock
flock -n 9
trap 'rc=$?; echo OBSERVER_BUILD_EXIT=$rc' EXIT
echo OBSERVER_BUILD_START="$(date -Is)"
cd /var/lib/kf-windows-20261005
[ "$(git -C kayfabe rev-parse HEAD)" = b431aeaf9fca5d78b451754c0de7b8dbbe9d7652 ]
[ -z "$(git -C kayfabe status --porcelain --untracked-files=no)" ]
[ ! -e kf3-bins/a8845e69 ]
python3 vfio-observer-a8845e69/tools/vfio-gsp-observer/apply.py qemu-10.2.4 --check
python3 vfio-observer-a8845e69/tools/vfio-gsp-observer/apply.py qemu-10.2.4
export CARGO_BUILD_JOBS=6
export CARGO_TARGET_DIR=/var/lib/kf-windows-20261005/target
(cd kayfabe; cargo build --release -p kf-qemu)
cmp target/release/libkf_qemu.a qemu-10.2.4/hw/misc/kf3/libkf_qemu.a
for f in kayfabe/qemu/hw/misc/kf3/*.{c,h}; do cmp "$f" "qemu-10.2.4/hw/misc/kf3/$(basename "$f")"; done
flock qemu-build-kf3.lock ninja -C qemu-build-kf3 -j6 qemu-system-x86_64
mkdir -m700 kf3-bins/a8845e69
install -m700 qemu-build-kf3/qemu-system-x86_64 kf3-bins/a8845e69/qemu-system-x86_64
for f in pc-bios qemu-bundle; do ln -s "/var/lib/kf-windows-20261005/qemu-build-kf3/$f" "kf3-bins/a8845e69/$f"; done
python3 - <<'PY'
import json,hashlib,subprocess
from pathlib import Path
b=Path('/var/lib/kf-windows-20261005');p=b/'kf3-bins/a8845e69'
def sha(p):
 with p.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
receipt={'product_revision':'b431aeaf9fca5d78b451754c0de7b8dbbe9d7652','observer_revision_short':'a8845e69','qemu_version':'10.2.4','qemu_sha256':sha(p/'qemu-system-x86_64'),'rust_archive_sha256':sha(b/'qemu-10.2.4/hw/misc/kf3/libkf_qemu.a'),'patch_sha256':sha(b/'vfio-observer-a8845e69/tools/vfio-gsp-observer/integration.patch'),'configure_flags':(b/'qemu-build-kf3/.kf3-configure').read_text()}
(p/'build-receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt))
PY
