#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
TREE=/data/kf-managed-host
REV=$(git -C "$TREE" rev-parse --short=8 HEAD)
RUN=$W/codex-managed-route-regression-20261011-$REV
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=/data/kf-managed-route-target-$REV
export BENCH_DIR=$RUN/bench KF_FASTGUEST_DIR=$RUN/fastguest KF_LOCK=$RUN/private.lock KF_LOCKED=1 KF_DEVICE=kf3 KF_FROM_HOST=1
export QEMU_BIN=$W/readtrace/kf3-bins/$REV/qemu-system-x86_64
mkdir -p "$RUN/logs" "$BENCH_DIR"
cleanup() {
 rc=$?; trap - EXIT
 bash "$W/pti-iommu_nogdm.sh" DMA-FQ
 echo "MANAGED_ROUTE_REGRESSION_EXIT=$rc at=$(date -Is)"
}
trap cleanup EXIT
echo "MANAGED_ROUTE_REGRESSION_START=$(date -Is) source=$(git -C "$TREE" rev-parse HEAD)"
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
[[ $(df -B1 --output=avail /var/lib | tail -1) -gt 161061273600 ]]
# The preceding isolated managed run has one already diagnosed Xid32. No
# unidentified fault is accepted here; gates below must freshly prove health.
grep -q '^RECOVERY_DIAGNOSTICS_EXIT=0 ' "$W/codex-recovery-diagnostics.log"
python3 - "$W/codex-recovery-diagnostics-20261011-a61af1c3" <<'CHECK'
import json,pathlib,sys
p=pathlib.Path(sys.argv[1]);m=json.loads((p/'kernel-xids.json').read_text())
assert m['query_exit']==0 and m['xid_records']==1,m
lines=(p/'kernel-xids.txt').read_text().splitlines()
assert len(lines)==1 and '16:15:17' in lines[0] and 'pid=82479,' in lines[0] and 'intr1 00000004 HCE_DBG0 00000300 HCE_DBG1 00003185' in lines[0],lines
s=(p/'native-video-semaphore/probe.log').read_text()
assert 'VIDEO_SEMAPHORE_EXIT mode=release rc=0' in s and 'VIDEO_SEMAPHORE_EXIT mode=trap rc=0' in s,s
CHECK
cd "$TREE"
# Hypothesis: kernel UVM routing preserves the existing Linux paths.
# Falsifier: any gate, privilege census or fast arm fails on this exact revision.
bash scripts/bench/v3_gates.sh "$RUN/logs/gates.log" > "$RUN/logs/gates-console.log" 2>&1
bash scripts/bench/build_kf3.sh "$W/readtrace/qemu-10.2.4" "$W/readtrace/qemu-build-kf3" > "$RUN/logs/build.log" 2>&1
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder > "$RUN/logs/client.log" 2>&1
CLIENT=$CARGO_TARGET_DIR/release/kayfabe-rm-ladder bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 "$KF_FASTGUEST_DIR" > "$RUN/logs/fastguest.log" 2>&1
bash scripts/fastguest/fast_suite.sh codexuvm 180 > "$RUN/logs/fast.log" 2>&1
sha256sum "$QEMU_BIN"
