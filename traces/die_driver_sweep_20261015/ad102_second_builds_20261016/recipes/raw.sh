#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/ada2build_raw.log 2>&1
trap 'rc=$?; echo "ADA2BUILD_RAW_EXIT rc=$rc $(date -u -Is)"' EXIT
cd /root/kayfabe
export PATH=/root/.cargo/bin:$PATH
grep -q '^ADA2BUILD_LINUX_EXIT rc=0 ' /root/prov/ada2build_linux.log
grep -q '^VARIANTS_EXIT rc=0 ' /root/prov/ada2build_variants.log
test "$(git rev-parse --short=9 HEAD)" = 9d6f210ba
test -z "$(pgrep -x qemu-system-x86 || true)"
echo "ADA2BUILD_RAW_START source=$(git rev-parse HEAD) $(date -u -Is)"
bash scripts/bench/cuda_ladder.sh host codex_ada2build_native595 1 >/root/prov/ada2build_native_cuda.log 2>&1
test "$(grep -c 'CL_ROW.*verdict=PASS' /workspace/bench/cl_codex_ada2build_native595_host.out)" = 4
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export KF_DEVICE=kf3 KF_FASTGUEST_DIR=/workspace/bench/fastguest595 ADA2BUILD_LABEL=normal
export ADA2BUILD_SOURCE=$(git rev-parse HEAD)
export ADA2BUILD_QEMU_BIN=/workspace/bench/kf3-bins/9d6f210b/qemu-system-x86_64 QEMU_BIN=/root/ada2build_fast_wrapper.py
sha256sum "$ADA2BUILD_QEMU_BIN" target/release/kayfabe-rm-ladder /workspace/bench/fastguest595/initrd.cpio.gz
echo "LINUX595_RAW30_START actual_KF3={} normal6 budget180 $(date -u -Is)"
timeout --signal=TERM 1500 bash scripts/fastguest/fast_suite.sh codex_ada2build_full595 180 >/root/prov/ada2build_full595_suite.log 2>&1
test -z "$(find /root/prov/ada2build-fast-receipts -name '*.FAILED' -print -quit)"
bash scripts/bench/box/birth_census.sh /workspace/bench codex_ada2build_full595 /root/prov/ada2build_gates.log >/root/prov/ada2build_births.log
echo "LINUX595_RAW30_COMPLETE $(date -u -Is)"
