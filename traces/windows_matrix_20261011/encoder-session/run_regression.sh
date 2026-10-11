#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
TREE=/data/kf-encoder-host
REV=$(git -C "$TREE" rev-parse --short=8 HEAD)
RUN=$W/codex-encoder-regression-20261011-$REV
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=/data/kf-encoder-target-$REV
export BENCH_DIR=$RUN/bench KF_FASTGUEST_DIR=$RUN/fastguest KF_LOCK=$RUN/private.lock KF_LOCKED=1 KF_DEVICE=kf3 KF_FROM_HOST=1
export QEMU_BIN=$W/readtrace/kf3-bins/$REV/qemu-system-x86_64
mkdir -p "$RUN/logs" "$BENCH_DIR"
cleanup() {
 rc=$?; trap - EXIT
 bash "$W/pti-iommu_nogdm.sh" DMA-FQ
 echo "ENCODER_REGRESSION_EXIT=$rc at=$(date -Is)"
}
trap cleanup EXIT
echo "ENCODER_REGRESSION_START=$(date -Is) source=$(git -C "$TREE" rev-parse HEAD)"
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
[[ $(df -B1 --output=avail /var/lib | tail -1) -gt 161061273600 ]]
cd "$TREE"
# Hypothesis: exact opaque-contract support preserves the existing Linux paths.
# Falsifier: any gate, privilege census or fast arm fails on this exact revision.
bash scripts/bench/v3_gates.sh "$RUN/logs/gates.log" > "$RUN/logs/gates-console.log" 2>&1
bash scripts/bench/build_kf3.sh "$W/readtrace/qemu-10.2.4" "$W/readtrace/qemu-build-kf3" > "$RUN/logs/build.log" 2>&1
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder > "$RUN/logs/client.log" 2>&1
CLIENT=$CARGO_TARGET_DIR/release/kayfabe-rm-ladder bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 "$KF_FASTGUEST_DIR" > "$RUN/logs/fastguest.log" 2>&1
bash scripts/fastguest/fast_suite.sh codexenc 180 > "$RUN/logs/fast.log" 2>&1
sha256sum "$QEMU_BIN"
