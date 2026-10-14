#!/usr/bin/env bash
set -euo pipefail
REV=7d00679ae56a7cf82ab4621a493e42cd37f1a215
TREE=/data/kf-memory-access-host-7d00679ae
ROOT=/var/lib/kf-windows-20261005/codex-native-level-7d00679ae
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=/data/kf-fifo-refusal-target-b081f705 CARGO_INCREMENTAL=0
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test ! -e "$ROOT"
mkdir -p "$ROOT"
START=$(date -Is)
echo "$START" > "$ROOT/START"
trap 'rc=$?; echo "$rc" > "$ROOT/EXIT"; journalctl -k --since "$START" --no-pager | sed -n "/NVRM: Xid/p" > "$ROOT/host-xid.log"; echo NATIVE_LEVEL_EXIT=$rc' EXIT
cd "$TREE"
test "$(git rev-parse HEAD)" = "$REV"
echo 'HYPOTHESIS public C56F LEVEL1 suppresses progress classification only; same direct commands and packet continuation execute correctly.'
echo 'FALSIFIER any copy/reference/GP_GET/order oracle failure, early reference, or hostXid; one changed firstGP LEVEL bit per matched pair.'
cargo build --release -p kf-harness --bin kf-user-ring-probe > "$ROOT/build.log" 2>&1
BIN=$CARGO_TARGET_DIR/release/kf-user-ring-probe
sha256sum "$BIN" > "$ROOT/binary.sha256"
for mode in reference reference-subroutine reference-split-held reference-subroutine-split-held; do
    echo "NATIVE_LEVEL_START mode=$mode at=$(date -Is)"
    setpriv --reuid=65534 --regid=65534 --clear-groups "$BIN" "$mode" > "$ROOT/$mode.log" 2>&1
    tail -5 "$ROOT/$mode.log"
done
