#!/bin/bash
# build-irqflood2.sh — kf3 of claude/irqflood-bisect-20261009 in a PRIVATE copy of the observer-patched QEMU tree
set -euo pipefail
umask 022
export PATH=/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
trap "rc=\$?; echo IRQFLOOD4_BUILD_EXIT=\$rc \$(date -Is)" EXIT
echo IRQFLOOD4_BUILD_START=$(date -Is)
W=/var/lib/kf-windows-20261005
cd $W/kayfabe-irqflood
git fetch -q origin claude/irqflood-bisect-20261009
git checkout -q --detach FETCH_HEAD
[ -z "$(git status --porcelain --untracked-files=no)" ]
echo REV=$(git rev-parse --short=8 HEAD)
export CARGO_BUILD_JOBS=16
export CARGO_TARGET_DIR=$W/target-irqflood
bash scripts/bench/build_kf3.sh $W/irqflood-qemu/qemu-10.2.4 $W/irqflood-qemu/qemu-build-kf3
