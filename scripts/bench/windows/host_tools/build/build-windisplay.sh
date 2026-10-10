#!/bin/bash
# build-windisplay.sh REV|BRANCH — Windows-display agent 2026-10-08: build kf3 for a revision into
# /var/lib/kf-windows-20261005/kf3-bins/<rev>. Own worktree and target dir; serialized by the build
# dir lock inside build_kf3.sh (not the GPU lock: the Linux demo VM holds that).
set -euo pipefail
umask 022
export PATH=/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
trap "rc=\$?; echo WINDISPLAY_BUILD_EXIT=\$rc \$(date -Is)" EXIT
echo WINDISPLAY_BUILD_START="$(date -Is)"
W=/var/lib/kf-windows-20261005
cd $W/kayfabe
git fetch -q origin "$1" 2>/dev/null || git fetch -q origin
REV=$(git rev-parse FETCH_HEAD)
[ -d $W/kayfabe-windisplay ] || git worktree add --detach $W/kayfabe-windisplay $REV
cd $W/kayfabe-windisplay
git checkout -q --detach $REV
[ -z "$(git status --porcelain --untracked-files=no)" ]
echo "REV=$(git rev-parse --short=8 HEAD)"
export CARGO_BUILD_JOBS=12
export CARGO_TARGET_DIR=$W/target-windisplay
bash scripts/bench/build_kf3.sh $W/qemu-10.2.4 $W/qemu-build-kf3
ls -la $W/kf3-bins/$(git rev-parse --short=8 HEAD)/
