#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/gb206phase_cmc_controller.log 2>&1
trap 'rc=$?; echo "CMC_CONTROLLER_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "CMC_CONTROLLER_START $(date -u -Is)"
cd /root/kayfabe
test -z "$(pgrep -x qemu-system-x86 || true)"
test ! -e /root/win/codex-gb206phase/qpid
test "$(date +%s)" -lt "$(date -u -d '2026-10-16T21:35:00Z' +%s)"
export PATH=/root/.cargo/bin:$PATH CARGO_TARGET_DIR=/root/kayfabe/target CARGO_BUILD_JOBS=4
git fetch -q origin codex/gb206-cmc-hw-20261016
git checkout --detach e1bd2df4d
test "$(git rev-parse --short=9 HEAD)" = e1bd2df4d
git diff --quiet
cargo test -p kf-rm --test cmc_log_probe >/root/prov/gb206phase_cmc_cpu.log 2>&1
timeout --signal=TERM 600 bash scripts/bench/v3_gates.sh /root/prov/gb206phase_cmc_gates.log >/root/prov/gb206phase_cmc_gates_wrapper.log 2>&1
timeout --signal=TERM 600 bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-kf3 >/root/prov/gb206phase_cmc_build.log 2>&1
echo "CMC_PRIVATE_READY source=$(git rev-parse HEAD) $(date -u -Is)"
bash /root/gb206phase_cmc_windows.sh
echo "CMC_CONTROLLER_COMPLETE $(date -u -Is)"
