#!/usr/bin/env bash
set -euo pipefail
REV=16a1b6a78a02a9280c6e50010e831b73d01beed7
TREE=/data/kf-memory-access-host-16a1b6a78
OUT=/var/lib/kf-windows-20261005/codex-user-mask-zero-16a1b6a78
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=/data/kf-fifo-refusal-target-b081f705 CARGO_INCREMENTAL=0
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test ! -e "$OUT"
mkdir -p "$OUT"
START=$(date -Is)
echo "$START" > "$OUT/START"
finish() {
 rc=$?;trap - EXIT
 journalctl -k --since "$START" --no-pager | sed -n '/NVRM: Xid/p' > "$OUT/host-xid.log"
 echo "$rc" > "$OUT/EXIT"
 echo "NATIVE_ZERO_HARNESS_EXIT=$rc"
}
trap finish EXIT
cd "$TREE"
test "$(git rev-parse HEAD)" = "$REV"
echo 'HYPOTHESIS native USER default-zero and explicit-id1 allocations have equal initial ACTIVE/SET STORE USE behavior for the identical bounded mask fixture.'
echo 'FALSIFIER default-zero faults or has missing initial marker/excluded reference99/wrong copy or ordering while id1 control passes. This measures host595.91.07 USER only, never guest580.88 privileged GSP equivalence.'
echo 'ONE_VARIABLE native allocation subDeviceId0 vs explicit1; same fixture, caller uid65534, ring, USER privilege and physical-CE prohibition.'
cargo build --release -p kf-harness --bin kf-user-ring-probe > "$OUT/build.log" 2>&1
BIN=$CARGO_TARGET_DIR/release/kf-user-ring-probe
sha256sum "$BIN" > "$OUT/binary.sha256"
for mode in storage-subdevice-default-phased storage-subdevice-phased; do
 echo "MODE_START $mode $(date -Is)"
 set +e
 setpriv --reuid=65534 --regid=65534 --clear-groups "$BIN" "$mode" > "$OUT/$mode.log" 2>&1
 rc=$?
 set -e
 echo "$rc" > "$OUT/$mode.EXIT"
 tail -12 "$OUT/$mode.log"
 echo "MODE_EXIT $mode $rc"
 nvidia-smi --query-gpu=name,driver_version --format=csv,noheader > "$OUT/health-after-$mode.txt"
done
