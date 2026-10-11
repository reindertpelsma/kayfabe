#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
REV=0f9c01f9
TREE=/data/kf-slice-host-0f9c01f9
RUN=$W/codex-copy-slice-20261011-$REV
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=/data/kf-slice-target-0f9c01f9
export BENCH_DIR=$RUN/bench KF_FASTGUEST_DIR=$RUN/fastguest KF_LOCK=$RUN/private.lock KF_LOCKED=1 KF_DEVICE=kf3
export QEMU_BIN=$W/readtrace/kf3-bins/$REV/qemu-system-x86_64
mkdir -p "$RUN/logs" "$BENCH_DIR"
cleanup() {
 rc=$?; trap - EXIT
 bash "$W/pti-iommu_nogdm.sh" DMA-FQ
 echo "SLICE_REGRESSION_EXIT=$rc at=$(date -Is)"
}
trap cleanup EXIT
echo "SLICE_REGRESSION_START=$(date -Is)"
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
git -C /data/kf-gl730101-host worktree add --detach "$TREE" "$REV"
cd "$TREE"
echo "SOURCE=$(git rev-parse HEAD)"
bash scripts/bench/v3_gates.sh "$RUN/logs/gates.log" > "$RUN/logs/gates-console.log" 2>&1
bash scripts/bench/build_kf3.sh "$W/readtrace/qemu-10.2.4" "$W/readtrace/qemu-build-kf3" > "$RUN/logs/build.log" 2>&1
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder > "$RUN/logs/client.log" 2>&1
CLIENT=$CARGO_TARGET_DIR/release/kayfabe-rm-ladder bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 "$KF_FASTGUEST_DIR" > "$RUN/logs/fastguest.log" 2>&1
bash scripts/fastguest/fast_suite.sh codexslice 180 > "$RUN/logs/fast.log" 2>&1
sha256sum "$QEMU_BIN"
