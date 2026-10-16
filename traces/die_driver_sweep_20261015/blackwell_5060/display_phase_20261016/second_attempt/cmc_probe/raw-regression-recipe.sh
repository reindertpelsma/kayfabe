#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/gb206phase_cmc_raw.log 2>&1
trap 'rc=$?; echo "CMC_RAW_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "CMC_RAW_QUEUED $(date -u -Is)"
# This queue performs no build or GPU operation until the complete paired diagnostic stopped.
while kill -0 107663 2>/dev/null; do
 test "$(date +%s)" -lt "$(date -u -d '2026-10-16T21:30:00Z' +%s)"
 sleep 3
done
grep -q '^CMC_CONTROLLER_COMPLETE ' /root/prov/gb206phase_cmc_controller.log
test -z "$(pgrep -x qemu-system-x86 || true)"
cd /root/kayfabe
test "$(git rev-parse --short=9 HEAD)" = e1bd2df4d
git diff --quiet
echo "CMC_RAW_PREPARE source=$(git rev-parse HEAD) $(date -u -Is)"
export PATH=/root/.cargo/bin:$PATH CARGO_TARGET_DIR=/root/kayfabe/target CARGO_BUILD_JOBS=4
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder >/root/prov/gb206phase_cmc_raw_build.log 2>&1
KF_FROM_HOST=1 bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.empty /workspace/bench/fastguest-cmc595 >/root/prov/gb206phase_cmc_fast_build.log 2>&1
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export KF_DEVICE=kf3 KF_FASTGUEST_DIR=/workspace/bench/fastguest-cmc595 GB206PHASE_LABEL=normal
export GB206PHASE_SOURCE=$(git rev-parse HEAD)
export GB206PHASE_QEMU_BIN=/workspace/bench/kf3-bins/e1bd2df4/qemu-system-x86_64 QEMU_BIN=/root/gb206phase_cmc_fast_wrapper.py
sha256sum "$GB206PHASE_QEMU_BIN" target/release/kayfabe-rm-ladder "$KF_FASTGUEST_DIR/initrd.cpio.gz"
echo "CMC_RAW30_START normal_loglevel6 actual_KF3={} budget180 $(date -u -Is)"
timeout --signal=TERM 1500 bash scripts/fastguest/fast_suite.sh codex_gb206phase_cmc595 180 >/root/prov/gb206phase_cmc_full595_suite.log 2>&1
test -z "$(find /root/prov/gb206phase-cmc-fast-receipts -name '*.FAILED' -print -quit)"
echo "CMC_RAW30_COMPLETE $(date -u -Is)"
