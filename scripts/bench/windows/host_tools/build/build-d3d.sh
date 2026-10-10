#!/bin/bash
set -euo pipefail
umask 077
export PATH=/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
exec 9>/tmp/kayfabe-fastguest.lock
flock -n 9
trap "rc=\$?; echo D3D_BUILD_EXIT=\$rc" EXIT
echo D3D_BUILD_START="$(date -Is)"
cd /var/lib/kf-windows-20261005
REV=$1
git -C kayfabe fetch -q origin claude/code43-d3d-20261008
git -C kayfabe worktree add --detach /var/lib/kf-windows-20261005/kayfabe-d3d $REV
cd kayfabe-d3d
[ -z "$(git status --porcelain --untracked-files=no)" ]
export CARGO_BUILD_JOBS=6
export CARGO_TARGET_DIR=/var/lib/kf-windows-20261005/target
bash scripts/bench/build_kf3.sh /var/lib/kf-windows-20261005/qemu-10.2.4 /var/lib/kf-windows-20261005/qemu-build-kf3
ls /var/lib/kf-windows-20261005/kf3-bins/
