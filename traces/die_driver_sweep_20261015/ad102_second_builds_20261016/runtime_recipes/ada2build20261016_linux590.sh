#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/ada2build_linux590.log 2>&1
trap 'rc=$?; echo "LINUX590_EXIT rc=$rc $(date -u -Is)"' EXIT
cd /root/kayfabe
export PATH=/root/.cargo/bin:$PATH CARGO_BUILD_JOBS=4 JOBS=4
check_deadline(){ test "$(date +%s)" -lt "$(date -u -d '2026-10-17 00:15:00' +%s)"; }
echo "LINUX590_QUEUED $(date -u -Is)"
for _ in $(seq 1000); do
 if grep -q '^RESUME_WINDOWS_EXIT rc=' /root/prov/ada2build_resume_windows_v2.log; then break; fi
 check_deadline;sleep 3
done
grep -q '^RESUME_WINDOWS_COMPLETE ' /root/prov/ada2build_resume_windows_v2.log
test -z "$(pgrep -x qemu-system-x86 || true)"
test "$(git rev-parse --short=9 HEAD)" = 9d6f210ba
check_deadline
echo "LINUX590_PREPARE source=$(git rev-parse HEAD) $(date -u -Is)"
timeout --signal=TERM 1200 bash scripts/drivermatrix/stage_guest_driver.sh 590.48.01 /workspace/drivers >/root/prov/ada2build_stage590.log 2>&1
git -C /workspace/drivers/src/ogkm-590.48.01 rev-parse HEAD >/root/prov/ada2build_ogkm590_revision.txt
KF_FROM_HOST=1 KF_GUEST_DRIVER_DIR=/workspace/drivers/590.48.01 bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.empty /workspace/bench/fastguest590 >/root/prov/ada2build_fast590_build.log 2>&1
check_deadline
test -z "$(pgrep -x qemu-system-x86 || true)"
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export KF_DEVICE=kf3 KF_FASTGUEST_DIR=/workspace/bench/fastguest590 ADA2BUILD_LABEL=normal
export ADA2BUILD_SOURCE=$(git rev-parse HEAD)
export ADA2BUILD_QEMU_BIN=/workspace/bench/kf3-bins/9d6f210b/qemu-system-x86_64 QEMU_BIN=/root/ada2build_fast_wrapper590.py
sha256sum "$ADA2BUILD_QEMU_BIN" target/release/kayfabe-rm-ladder /workspace/bench/fastguest590/initrd.cpio.gz
printf 'GUEST_MODULE_VERSION='; modinfo -F version /workspace/drivers/590.48.01/modules/nvidia.ko
printf 'HOST_DRIVER='; nvidia-smi --query-gpu=driver_version --format=csv,noheader
echo "LINUX590_RAW30_START actual_KF3={} normal6 budget180 $(date -u -Is)"
timeout --signal=TERM 1500 bash scripts/fastguest/fast_suite.sh codex_ada2build_full590 180 >/root/prov/ada2build_full590_suite.log 2>&1
if find /root/prov/ada2build-fast590-receipts -name '*.FAILED' -print | grep -q .; then exit 1; fi
bash scripts/bench/box/birth_census.sh /workspace/bench codex_ada2build_full590 /root/prov/ada2build_gates.log >/root/prov/ada2build_births590.log
echo "LINUX590_RAW30_COMPLETE $(date -u -Is)"
