#!/usr/bin/env bash
# Retry only the NOT_RUN fast suite, after completed initial runner EXIT2.
# Caller holds the shared GPU lock; exact product/source unchanged.
set -euo pipefail
W=/var/lib/kf-windows-20261005
TREE=/data/kf-user-ring-host-70aa0d666
ROOT=$W/codex-user-ring-linux-70aa0d666
export CARGO_TARGET_DIR=/data/kf-fifo-refusal-target-b081f705
export BENCH_DIR=$ROOT/fast-retry/bench KF_FASTGUEST_DIR=$ROOT/fastguest
export KF_LOCK=$ROOT/private.lock KF_LOCKED=1 KF_DEVICE=kf3 KF_FROM_HOST=1
# build_kf3 installs by EIGHT-character source prefix.
export QEMU_BIN=$W/readtrace/kf3-bins/70aa0d66/qemu-system-x86_64
test "$(git -C "$TREE" rev-parse HEAD)" = 70aa0d666c22289583d8659e325b0763bef4a982
test -z "$(git -C "$TREE" status --porcelain --untracked-files=no)"
test "$(cat "$ROOT/EXIT")" = 2
test -x "$QEMU_BIN"
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 107374182400
test ! -e "$ROOT/fast-retry"
mkdir -p "$BENCH_DIR"
date -Is > "$ROOT/fast-retry/START"
echo "USER_RING_FAST_RETRY_START=$(date -Is) source=$(git -C "$TREE" rev-parse HEAD)"
finish() {
    rc=$?; trap - EXIT
    echo "$rc" > "$ROOT/fast-retry/EXIT"
    echo "USER_RING_FAST_RETRY_EXIT=$rc at=$(date -Is)"
    pgrep -a qemu-system || true
    cat /sys/kernel/iommu_groups/11/type
    df -h /var/lib
}
trap finish EXIT
echo 'HYPOTHESIS exact70aa prototype preserves Linux fast suite.'
echo 'FALSIFIER any FAIL/CRASH/NOTRUN or missing original30-arm result.'
cd "$TREE"
bash scripts/fastguest/fast_suite.sh codexuserRing 180 > "$ROOT/fast-retry/fast.log" 2>&1
sha256sum "$QEMU_BIN" > "$ROOT/fast-retry/binary.sha256"
