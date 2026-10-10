#!/bin/bash
# build-irqflood.sh — build kf3 for claude/debug-irq-flood-20261009 into kf3-bins/<rev> (own worktree and target dir).
set -euo pipefail
umask 022
export PATH=/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
trap "rc=\$?; echo IRQFLOOD_BUILD_EXIT=\$rc \$(date -Is)" EXIT
echo IRQFLOOD_BUILD_START=$(date -Is)
W=/var/lib/kf-windows-20261005
cd $W/kayfabe
git fetch -q origin claude/debug-irq-flood-20261009
REV=$(git rev-parse FETCH_HEAD)
[ -d $W/kayfabe-irqflood ] || git worktree add --detach $W/kayfabe-irqflood $REV
cd $W/kayfabe-irqflood
git checkout -q --detach $REV
[ -z "$(git status --porcelain --untracked-files=no)" ]
echo REV=$(git rev-parse --short=8 HEAD)
export CARGO_BUILD_JOBS=12
export CARGO_TARGET_DIR=$W/target-irqflood
bash scripts/bench/build_kf3.sh $W/qemu-10.2.4 $W/qemu-build-kf3
ls -la $W/kf3-bins/$(git rev-parse --short=8 HEAD)/
