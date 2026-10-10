#!/bin/bash
# build-wintimeout.sh — Windows-reenable agent 2026-10-08: GPU-free tests then kf3 for the checked-out revision.
set -uo pipefail
umask 022
export PATH=/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
trap "rc=\$?; echo REENABLE_BUILD_EXIT=\$rc \$(date -Is)" EXIT
echo REENABLE_BUILD_START="$(date -Is)"
W=/var/lib/kf-windows-20261005
cd $W/kayfabe
git fetch -q origin claude/windows-reenable-20261008
REV=$(git rev-parse FETCH_HEAD)
cd $W/kayfabe-reenable
git checkout -q --detach $REV
echo "REV=$(git rev-parse --short=8 HEAD)"
export CARGO_BUILD_JOBS=12
export CARGO_TARGET_DIR=$W/target-windisplay
cargo test -q -p kf-chan 2>&1 | grep -E "test result|FAILED|panicked|error" | head -20
cargo test -q -p kf-rm 2>&1 | grep -E "test result|FAILED|panicked|^error" | head -20
cargo test -q -p kf-disp 2>&1 | grep -E "test result|FAILED|panicked|^error" | head -20
cargo test -q -p kf-qemu 2>&1 | grep -E "test result|FAILED|panicked|error" | head -20
[ "${TESTS_ONLY:-0}" = 1 ] && exit 0
bash scripts/bench/build_kf3.sh $W/qemu-10.2.4 $W/qemu-build-kf3 2>&1 | tail -15
ls -la $W/kf3-bins/$(git rev-parse --short=8 HEAD)/
