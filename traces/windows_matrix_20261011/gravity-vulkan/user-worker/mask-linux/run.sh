#!/usr/bin/env bash
set -euo pipefail
REV=b50774a7a5343cf10af849db1445b577e9b21b3f
TREE=/data/kf-memory-access-host-b50774a7a
OUT=/var/lib/kf-windows-20261005/codex-user-worker-b50774a7a
NATIVE=$OUT/native-mask
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=/data/kf-fifo-refusal-target-b081f705 CARGO_INCREMENTAL=0
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
mkdir -p "$NATIVE"
START=$(date -Is)
echo "$START" > "$OUT/START"
trap 'rc=$?; echo "$rc" > "$OUT/EXIT"; echo USER_SPAN_LINUX_EXIT=$rc' EXIT
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 107374182400
cd "$TREE"
test "$(git rev-parse HEAD)" = "$REV"
echo 'HYPOTHESIS explicit per-channel id1 matches initial ACTIVE and SET STORE USE filtering, including all excluded bits, while inactive descriptor fetch preserves native HOST ordering.'
echo 'FALSIFIER missing initial marker, excluded bit reference99, held GET>=3/early reference, wrong copy/reference/final GET4 or hostXid. One behavior variable is explicit mask identity and public mask controls; GPU work/copy/barrier oracle unchanged.'
cargo build --release -p kf-harness --bin kf-user-ring-probe > "$NATIVE/build.log" 2>&1
BIN=$CARGO_TARGET_DIR/release/kf-user-ring-probe
sha256sum "$BIN" > "$NATIVE/binary.sha256"
for mode in storage-control-barrier-phased storage-subdevice-phased; do
    echo "NATIVE_WITNESS_START mode=$mode at=$(date -Is)"
    setpriv --reuid=65534 --regid=65534 --clear-groups "$BIN" "$mode" > "$NATIVE/$mode.log" 2>&1
    tail -7 "$NATIVE/$mode.log"
done
journalctl -k --since "$START" --no-pager | sed -n '/NVRM: Xid/p' > "$NATIVE/host-xid.log"
test ! -s "$NATIVE/host-xid.log"
echo 0 > "$NATIVE/EXIT"
bash "$TREE/traces/windows_matrix_20261011/gravity-vulkan/user-memory-access/run_linux.sh" "$REV"
bash "$TREE/traces/windows_matrix_20261011/gravity-vulkan/user-worker/run_linux_desktop.sh" "$REV"
