#!/usr/bin/env bash
set -euo pipefail
REV=a38602015d0c295fdbb9a03e2e28fbabf78174ae
TREE=/data/kf-memory-access-host-a38602015
OUT=/var/lib/kf-windows-20261005/codex-user-worker-a38602015
NATIVE=$OUT/native-witness
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=/data/kf-fifo-refusal-target-b081f705 CARGO_INCREMENTAL=0
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
mkdir -p "$NATIVE"
START=$(date -Is)
echo "$START" > "$OUT/START"
trap 'rc=$?; echo "$rc" > "$OUT/EXIT"; echo USER_LEVEL_LINUX_EXIT=$rc' EXIT
python3 /tmp/kf-user-level-cleanup-1560.py
cd "$TREE"
test "$(git rev-parse HEAD)" = "$REV"
echo 'HYPOTHESIS synthetic witness LEVEL1 retains CONTROL NOP SYNC_WAIT host-consumption ordering.'
echo 'FALSIFIER native storage copy/fetch/held-prefix oracle fails or hostXid; witness LEVEL is the only behavioral change in this native storage path.'
cargo build --release -p kf-harness --bin kf-user-ring-probe > "$NATIVE/build.log" 2>&1
BIN=$CARGO_TARGET_DIR/release/kf-user-ring-probe
sha256sum "$BIN" > "$NATIVE/binary.sha256"
for mode in storage-control-barrier-phased storage-assembled-phased; do
    echo "NATIVE_WITNESS_START mode=$mode at=$(date -Is)"
    setpriv --reuid=65534 --regid=65534 --clear-groups "$BIN" "$mode" > "$NATIVE/$mode.log" 2>&1
    tail -7 "$NATIVE/$mode.log"
done
journalctl -k --since "$START" --no-pager | sed -n '/NVRM: Xid/p' > "$NATIVE/host-xid.log"
test ! -s "$NATIVE/host-xid.log"
echo 0 > "$NATIVE/EXIT"
bash "$TREE/traces/windows_matrix_20261011/gravity-vulkan/user-memory-access/run_linux.sh" "$REV"
bash "$TREE/traces/windows_matrix_20261011/gravity-vulkan/user-worker/run_linux_desktop.sh" "$REV"
