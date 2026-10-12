#!/usr/bin/env bash
# Caller holds the shared GPU lock. No Windows/personal disk touched.
set -euo pipefail
W=/var/lib/kf-windows-20261005
REV=70aa0d666
TREE=/data/kf-user-ring-host-$REV
ROOT=$W/codex-user-ring-linux-$REV
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=/data/kf-fifo-refusal-target-b081f705
export BENCH_DIR=$ROOT/bench KF_FASTGUEST_DIR=$ROOT/fastguest
export KF_LOCK=$ROOT/private.lock KF_LOCKED=1 KF_DEVICE=kf3 KF_FROM_HOST=1
export QEMU_BIN=$W/readtrace/kf3-bins/$REV/qemu-system-x86_64
test "$(git -C "$TREE" rev-parse --short=9 HEAD)" = "$REV"
test -z "$(git -C "$TREE" status --porcelain --untracked-files=no)"
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 107374182400
test ! -e "$ROOT"
mkdir -p "$ROOT/logs"
date -Is > "$ROOT/START"
echo "USER_RING_LINUX_START=$(date -Is) source=$(git -C "$TREE" rev-parse HEAD)"
finish() {
    rc=$?; trap - EXIT
    echo "$rc" > "$ROOT/EXIT"
    echo "USER_RING_LINUX_EXIT=$rc at=$(date -Is)"
    pgrep -a qemu-system || true
    cat /sys/kernel/iommu_groups/11/type
    df -h /var/lib
}
trap finish EXIT
echo 'HYPOTHESIS unconnected queue model and restricted Windows permit preserve Linux behavior.'
echo 'FALSIFIER any gate, privilege census or fast arm fails on exact source.'
cd "$TREE"
bash scripts/bench/v3_gates.sh "$ROOT/logs/gates.log" > "$ROOT/logs/gates-console.log" 2>&1
bash scripts/bench/build_kf3.sh "$W/readtrace/qemu-10.2.4" "$W/readtrace/qemu-build-kf3" > "$ROOT/logs/build.log" 2>&1
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder > "$ROOT/logs/client.log" 2>&1
CLIENT=$CARGO_TARGET_DIR/release/kayfabe-rm-ladder bash scripts/fastguest/build_fast_guest.sh \
    /workspace/bench/guest.qcow2 "$KF_FASTGUEST_DIR" > "$ROOT/logs/fastguest.log" 2>&1
bash scripts/fastguest/fast_suite.sh codexuserRing 180 > "$ROOT/logs/fast.log" 2>&1
sha256sum "$QEMU_BIN" > "$ROOT/binary.sha256"
