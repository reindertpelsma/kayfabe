#!/usr/bin/env bash
set -euo pipefail
REV=$(git -C /data/kf-memory-access-host-1a666d783 rev-parse HEAD)
TREE=/data/kf-memory-access-host-1a666d783
ROOT=/var/lib/kf-windows-20261005/codex-prepared-native-1a666d783
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 107374182400
mkdir -p "$ROOT"
date -Is > "$ROOT/START"
trap 'rc=$?; echo "$rc" > "$ROOT/EXIT"; echo PREPARED_NATIVE_EXIT=$rc' EXIT
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=/data/kf-fifo-refusal-target-b081f705 CARGO_INCREMENTAL=0
cd "$TREE"
cargo build --release -p kf-harness --bin kf-deferred-update-probe > "$ROOT/build.log" 2>&1
python3 "$TREE/traces/windows_matrix_20261011/gravity-vulkan/prepared-native-migration/run.py" "$CARGO_TARGET_DIR/release/kf-deferred-update-probe" "$ROOT/result" "$REV"
