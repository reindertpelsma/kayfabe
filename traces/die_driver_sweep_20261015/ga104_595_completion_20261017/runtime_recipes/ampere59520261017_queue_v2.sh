#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/ampere595_queue.log 2>&1
trap 'rc=$?; echo "AMPERE595_QUEUE_EXIT rc=$rc $(date -u -Is)"' EXIT
export PATH=/root/.cargo/bin:$PATH CARGO_BUILD_JOBS=6
echo "AMPERE595_QUEUE_START $(date -u -Is)"
test "$(cat /root/prov/OWN_SWEEP_ID)" = 56326220
for _ in $(seq 2400); do
 if grep -q '^AMPERE595_LINUX_EXIT rc=' /root/prov/ampere595_linux.log; then break; fi
 test "$(date +%s)" -lt "$(date -u -d '2026-10-17 02:45:00' +%s)" || { echo BUILD_TIMEBOX_EXCEEDED=1; exit 2; }
 sleep 3
done
grep -q '^AMPERE595_LINUX_EXIT rc=0 ' /root/prov/ampere595_linux.log
cd /root/kayfabe
test "$(git rev-parse HEAD)" = 9d6f210ba1dc6ab0edbc49ff265101091e8b08f1
test -z "$(pgrep -x qemu-system-x86 || true)"
echo "NATIVE595_WORKLOADS_START $(date -u -Is)"
bash scripts/bench/cuda_ladder.sh host codex_ampere595_native 1 >/root/prov/ampere595_native_cuda.log 2>&1
test "$(grep -c 'CL_ROW.*verdict=PASS' /workspace/bench/cl_codex_ampere595_native_host.out)" = 4
bash scripts/fastguest/bare_metal_suite.sh codex_ampere595_native 180 >/root/prov/ampere595_native_raw.log 2>&1
grep -q 'BARE_SUITE_PASS=30' /root/prov/ampere595_native_raw.log
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export KF_DEVICE=kf3 KF_FASTGUEST_DIR=/workspace/bench/fastguest595 AMPERE595_LABEL=normal
export AMPERE595_SOURCE=$(git rev-parse HEAD)
export AMPERE595_QEMU_BIN=/workspace/bench/kf3-bins/9d6f210b/qemu-system-x86_64 QEMU_BIN=/root/ampere595_fast_wrapper.py
sha256sum "$AMPERE595_QEMU_BIN" target/release/kayfabe-rm-ladder /workspace/bench/fastguest595/initrd.cpio.gz
echo "LINUX595_RAW30_START actual_KF3={} normal6 budget180 $(date -u -Is)"
bash scripts/fastguest/fast_suite.sh codex_ampere595_full 180 >/root/prov/ampere595_full_suite.log 2>&1
test -z "$(find /root/prov/ampere595-fast-receipts -name '*.FAILED' -print -quit)"
bash scripts/bench/box/birth_census.sh /workspace/bench codex_ampere595_full /root/prov/ampere595_gates.log >/root/prov/ampere595_births.log
echo "LINUX595_RAW30_COMPLETE $(date -u -Is)"
test -z "$(pgrep -x qemu-system-x86 || true)"
test "$(date +%s)" -lt "$(date -u -d '2026-10-17 03:05:00' +%s)" || { echo FAT_PREP_NOTRUN=lease_time; exit 2; }
echo "FAT595_PREP_START $(date -u -Is)"
if ! timeout --signal=TERM 1500 bash /root/ampere595_fat_recipe.sh >/root/prov/ampere595_fat_prepare.log 2>&1; then
 python3 /root/ampere595_stop_stock.py || true
 exit 2
fi
python3 /root/ampere595_stop_stock.py
grep -q '^version=595.91.07$' /workspace/bench/guest.qcow2.STAGED
test -z "$(pgrep -x qemu-system-x86 || true)"
qemu-img create -q -f qcow2 -F qcow2 -b /workspace/bench/guest.qcow2 /workspace/bench/guest-595.91.07.qcow2
{ echo version=595.91.07; echo image=/workspace/bench/guest-595.91.07.qcow2; echo base=/workspace/bench/guest.qcow2; echo "staged=$(date -u -Is)"; } >/workspace/bench/guest-595.91.07.qcow2.STAGED
bash scripts/bench/provision_guest_gfx.sh host >/root/prov/ampere595_gfx_host_prepare.log 2>&1
if ! timeout --signal=TERM 900 env KF_GUEST_IMG=/workspace/bench/guest-595.91.07.qcow2 bash scripts/bench/provision_guest_gfx.sh >/root/prov/ampere595_gfx_guest_prepare.log 2>&1; then
 python3 /root/ampere595_stop_stock.py || true
 exit 2
fi
python3 /root/ampere595_stop_stock.py
test -z "$(pgrep -x qemu-system-x86 || true)"
bash scripts/bench/gfx/gfx_steps.sh host /root/gfxbin >/root/prov/ampere595595_native_gfx.log 2>&1
for step in 2 3 4 5; do grep -q "^GFX_S$step=PASS$" /root/prov/ampere595595_native_gfx.log; done
echo "NATIVE595_GRAPHICS_COMPLETE $(date -u -Is)"
# The raw wrapper is only for the fast guest; broker uses the per-revision binary directly.
unset QEMU_BIN
unset AMPERE595_QEMU_BIN AMPERE595_LABEL AMPERE595_SOURCE
bash /root/ampere595_broker595.sh
test -z "$(pgrep -x qemu-system-x86 || true)"
echo "AMPERE595_APPLICATIONS_COMPLETE $(date -u -Is)"
