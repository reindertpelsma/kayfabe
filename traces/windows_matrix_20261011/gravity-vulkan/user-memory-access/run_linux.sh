#!/usr/bin/env bash
# Caller holds flock -n -o /tmp/kayfabe-fastguest.lock for this whole runner.
# Fresh disposable Linux guest only. Personal Windows chain is never opened.
set -euo pipefail
REV=${1:?full source revision required}
[[ "$REV" =~ ^[0-9a-f]{40}$ ]]
W=/var/lib/kf-windows-20261005
TREE=/data/kf-memory-access-host-${REV:0:9}
ROOT=$W/codex-memory-access-linux-${REV:0:9}
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=/data/kf-fifo-refusal-target-b081f705
export CARGO_INCREMENTAL=0
export BENCH_DIR=$ROOT/bench KF_FASTGUEST_DIR=$ROOT/fastguest
export KF_LOCK=$ROOT/private.lock KF_LOCKED=1 KF_DEVICE=kf3 KF_FROM_HOST=1
export QEMU_BIN=$W/readtrace/kf3-bins/${REV:0:8}/qemu-system-x86_64
test "$(git -C "$TREE" rev-parse HEAD)" = "$REV"
test -z "$(git -C "$TREE" status --porcelain --untracked-files=no)"
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 107374182400
test ! -e "$ROOT"
mkdir -p "$ROOT/logs" "$BENCH_DIR"
START=$(date -Is)
echo "$START" > "$ROOT/START"
printf 'USER_MEMORY_ACCESS_LINUX_START=%s source=%s\n' "$START" "$REV"
finish() {
    rc=$?; trap - EXIT
    echo "$rc" > "$ROOT/EXIT"
    printf 'USER_MEMORY_ACCESS_LINUX_EXIT=%s at=%s\n' "$rc" "$(date -Is)"
    pgrep -a qemu-system || true
    cat /sys/kernel/iommu_groups/11/type
    df -h /var/lib
    journalctl -k --since "$START" --until "$(date -Is)" --no-pager \
        | sed -n '/NVRM: Xid/p' > "$ROOT/host-xid.log" || true
}
trap finish EXIT
echo 'HYPOTHESIS exact source preserves Linux object policy, USER birth/isolation and original fast arms with the shared graph wrapper.'
echo 'FALSIFIER any gate, privilege census or fast arm FAIL/CRASH/TIMEOUT/NOTRUN.'
cd "$TREE"
bash scripts/bench/v3_gates.sh "$ROOT/logs/gates.log" > "$ROOT/logs/gates-console.log" 2>&1
bash scripts/bench/build_kf3.sh "$W/readtrace/qemu-10.2.4" "$W/readtrace/qemu-build-kf3" > "$ROOT/logs/build.log" 2>&1
test -x "$QEMU_BIN"
sha256sum "$QEMU_BIN" > "$ROOT/binary.sha256"
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder > "$ROOT/logs/client.log" 2>&1
CLIENT=$CARGO_TARGET_DIR/release/kayfabe-rm-ladder bash scripts/fastguest/build_fast_guest.sh \
    /workspace/bench/guest.qcow2 "$KF_FASTGUEST_DIR" > "$ROOT/logs/fastguest.log" 2>&1
bash scripts/fastguest/fast_suite.sh codexMemoryAccess 180 > "$ROOT/logs/fast.log" 2>&1
test -z "$(git status --porcelain --untracked-files=no)"
