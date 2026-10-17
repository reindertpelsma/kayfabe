#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/gb206phase_raw.log 2>&1
trap 'rc=$?; echo "GB206PHASE_RAW_EXIT rc=$rc $(date -u -Is)"' EXIT
cd /root/kayfabe
export PATH=/root/.cargo/bin:$PATH
grep -q '^GB206PHASE_BUILD_READY' /root/prov/gb206phase_linux.log
test "$(sha256sum /root/win/base.qcow2 | cut -d' ' -f1)" = 539aaad2f7280d7fb80fcf67c16297792b7561113c9084978070e9d94ace044b
test -z "$(pgrep -x qemu-system-x86 || true)"
echo "GB206PHASE_RAW_START source=$(git rev-parse HEAD) $(date -u -Is)"
bash scripts/bench/cuda_ladder.sh host codex_gb206phase_native595 1 >/root/prov/gb206phase_native_cuda.log 2>&1
test "$(grep -c 'CL_ROW.*verdict=PASS' /workspace/bench/cl_codex_gb206phase_native595_host.out)" = 4
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export KF_DEVICE=kf3 KF_FASTGUEST_DIR=/workspace/bench/fastguest595 GB206PHASE_LABEL=normal
export GB206PHASE_SOURCE=$(git rev-parse HEAD)
rev=$(git rev-parse --short=8 HEAD)
export GB206PHASE_QEMU_BIN=/workspace/bench/kf3-bins/$rev/qemu-system-x86_64 QEMU_BIN=/root/gb206phase_fast_wrapper.py
sha256sum "$GB206PHASE_QEMU_BIN" target/release/kayfabe-rm-ladder /workspace/bench/fastguest595/initrd.cpio.gz
echo "LINUX595_RAW30_START normal_loglevel6 actual_KF3={} budget180 $(date -u -Is)"
set +e
bash scripts/fastguest/fast_suite.sh codex_gb206phase_full595 180 >/root/prov/gb206phase_full595_suite.log 2>&1
rc=$?
set -e
test -z "$(find /root/prov/gb206phase-fast-receipts -name '*.FAILED' -print -quit)"
echo "LINUX595_RAW30_COMPLETE rc=$rc $(date -u -Is)"
exit "$rc"
