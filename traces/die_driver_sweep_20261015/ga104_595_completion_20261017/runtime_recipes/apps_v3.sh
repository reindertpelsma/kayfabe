#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/ampere595_apps_queue.log 2>&1
trap 'rc=$?; echo "AMPERE595_APPS_EXIT rc=$rc $(date -u -Is)"' EXIT
export PATH=/root/.cargo/bin:$PATH CARGO_BUILD_JOBS=6
cd /root/kayfabe
test "$(cat /root/prov/OWN_SWEEP_ID)" = 56326220
test "$(git rev-parse HEAD)" = 9d6f210ba1dc6ab0edbc49ff265101091e8b08f1
grep -q '^LINUX595_RAW30_COMPLETE ' /root/prov/ampere595_queue.log
grep -q 'FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0' /workspace/bench/codex_ampere595_full_suite.out
test -s /root/NVIDIA-Linux-x86_64-595.91.07.run
test -z "$(pgrep -x qemu-system-x86 || true)"
echo "AMPERE595_APPS_START cached595package; firstfatattemptfailedbeforeguestboot $(date -u -Is)"
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
