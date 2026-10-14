#!/usr/bin/env bash
set -euo pipefail
TREE=/data/kf-memory-access-host-1a666d783
ROOT=/var/lib/kf-windows-20261005/codex-native-cursors-242647766
mkdir "$ROOT"
date -Is > "$ROOT/START"
trap 'rc=$?; echo "$rc" > "$ROOT/EXIT"; echo NATIVE_CURSOR_CAMPAIGN_EXIT=$rc' EXIT
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test -z "$(git -C "$TREE" status --porcelain)"
git -C "$TREE" fetch https://github.com/reindertpelsma/kayfabe.git codex/windows-matrix-20261011
git -C "$TREE" checkout --detach 242647766
REV=$(git -C "$TREE" rev-parse HEAD)
echo SOURCE="$REV"
export PATH="$HOME/.cargo/bin:$PATH" CARGO_INCREMENTAL=0
# The same owning checkout/target used for the recovered native reference.
# No dependency source changed since that build; only this bench binary changed.
export CARGO_TARGET_DIR=/data/kf-fifo-refusal-target-b081f705
cd "$TREE"
cargo build --release -p kf-harness --bin kf-deferred-update-probe > "$ROOT/build.log" 2>&1
python3 traces/windows_matrix_20261011/gravity-vulkan/prepared-native-migration/run.py "$CARGO_TARGET_DIR/release/kf-deferred-update-probe" "$ROOT/result" "$REV" cursors
