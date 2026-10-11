#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
REV=0f9c01f9
TREE=/data/kf-slice-host-0f9c01f9
RUN=$W/codex-copy-slice-hostdriver-20261011-$REV
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=/data/kf-slice-target-0f9c01f9
export BENCH_DIR=$RUN/bench KF_FASTGUEST_DIR=$RUN/fastguest KF_LOCK=$RUN/private.lock KF_LOCKED=1 KF_DEVICE=kf3 KF_FROM_HOST=1
export QEMU_BIN=$W/readtrace/kf3-bins/$REV/qemu-system-x86_64
mkdir -p "$RUN/logs" "$BENCH_DIR"
trap 'rc=$?; echo "SLICE_HOSTDRIVER_EXIT=$rc at=$(date -Is)"' EXIT
echo "SLICE_HOSTDRIVER_START=$(date -Is)"
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
cd "$TREE"
test "$(git rev-parse --short=8 HEAD)" = "$REV"
CLIENT=$CARGO_TARGET_DIR/release/kayfabe-rm-ladder bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 "$KF_FASTGUEST_DIR" > "$RUN/logs/fastguest.log" 2>&1
bash scripts/fastguest/fast_suite.sh codexslicehost 180 > "$RUN/logs/fast.log" 2>&1
