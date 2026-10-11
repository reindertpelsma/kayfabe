#!/usr/bin/env bash
# Serial regression of the pending display-model branch; run on the trusted GPU host.
# Invoke under: flock -o /tmp/kayfabe-fastguest.lock bash run.sh
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
W=/var/lib/kf-windows-20261005
REV=${1:?source revision}
OUT=$W/codex-gl-regression-20261011-$REV
REPO=/data/kf-gl730101-host
export CARGO_TARGET_DIR=/data/kf-glcrash-target
mkdir -p "$OUT/logs" "$OUT/bench"
cd "$REPO"
test "$(git rev-parse --short=8 HEAD)" = "$REV"
test -z "$(git status --porcelain --untracked-files=no)"
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
if pgrep qemu-system-x86 >/dev/null; then echo QEMU_ALREADY_RUNNING; exit 1; fi
echo "REGRESSION_START rev=$(git rev-parse HEAD) $(date -Is)"
trap 'rc=$?; echo "REGRESSION_EXIT rc=$rc $(date -Is)"' EXIT
stage() {
    local name=$1 rc=0
    shift
    echo "STAGE_START $name $(date -Is)"
    dmesg | grep -a 'Xid' > "$OUT/logs/${name}_xid_before.txt" || true
    "$@" > "$OUT/logs/$name.log" 2>&1 || rc=$?
    dmesg | grep -a 'Xid' > "$OUT/logs/${name}_xid_after.txt" || true
    echo "STAGE_EXIT $name rc=$rc $(date -Is)"
    return "$rc"
}
stage tests cargo test -p kf-disp -p kf-rm -p kf-qemu
stage gates bash scripts/bench/v3_gates.sh "$OUT/logs/v3_gates.log"
stage build bash scripts/bench/build_kf3.sh "$W/readtrace/qemu-10.2.4" "$W/readtrace/qemu-build-kf3"
QBIN=$W/readtrace/kf3-bins/$REV/qemu-system-x86_64
test -x "$QBIN"
stage client env KAYFABE_BUILD_REV=$REV cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder
stage fastguest env KF_FROM_HOST=1 CLIENT=$CARGO_TARGET_DIR/release/kayfabe-rm-ladder bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 "$OUT/fastguest"
stage fast env KF_LOCK=$OUT/private.lock KF_DEVICE=kf3 BENCH_DIR=$OUT/bench KF_FASTGUEST_DIR=$OUT/fastguest QEMU_BIN=$QBIN timeout 5000 bash scripts/fastguest/fast_suite.sh codexglreview 180
stage tspace bash -c 'set -e; for log in "$1"/bench/fast_codexglreview_*_qemu.log; do echo "LOG=$log"; python3 -I scripts/p1p2/tspace_log_gate.py "$log" --user; done' _ "$OUT"
# The broker harness selects its binary under /workspace/bench. Make only our revision visible.
LINK=/workspace/bench/kf3-bins/$REV
if [ ! -e "$LINK" ]; then
    ln -s "$W/readtrace/kf3-bins/$REV" "$LINK"
    trap 'rc=$?; rm -f "$LINK"; echo "REGRESSION_EXIT rc=$rc $(date -Is)"' EXIT
fi
stage broker env KF_LOCK=$OUT/private.lock KF3_REV=$REV bash scripts/bench/display/input_proof.sh codexglreview
cp "$W/broker-interactive/proof-codexglreview/proof.log" "$OUT/logs/broker_proof.log"
echo "REGRESSION_DONE $(date -Is)"
