#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/ada2build_fat590.log 2>&1
trap 'rc=$?; echo "FAT590_EXIT rc=$rc $(date -u -Is)"' EXIT
cd /root/kayfabe
export PATH=/root/.cargo/bin:$PATH CARGO_BUILD_JOBS=4 KAYFABE_REPO=/root/kayfabe
check_deadline(){ test "$(date +%s)" -lt "$(date -u -d '2026-10-17 00:20:00' +%s)"; }
echo "FAT590_QUEUED $(date -u -Is)"
for _ in $(seq 1600); do
 if grep -q '^LINUX590_EXIT rc=' /root/prov/ada2build_linux590.log; then break; fi
 check_deadline;sleep 3
done
grep -q '^LINUX590_RAW30_COMPLETE ' /root/prov/ada2build_linux590.log
test -z "$(pgrep -x qemu-system-x86 || true)"
test "$(git rev-parse --short=9 HEAD)" = 9d6f210ba
check_deadline
echo "FAT590_PREPARE $(date -u -Is)"
timeout --signal=TERM 1500 bash /root/ada2build_fat590_recipe.sh >/root/prov/ada2build_fat590_prepare.log 2>&1
test -z "$(pgrep -x qemu-system-x86 || true)"
grep -q '^version=590.48.01$' /workspace/bench/guest.qcow2.STAGED
qemu-img create -q -f qcow2 -F qcow2 -b /workspace/bench/guest.qcow2 /workspace/bench/guest-590.48.01.qcow2
{ echo version=590.48.01; echo image=/workspace/bench/guest-590.48.01.qcow2; echo base=/workspace/bench/guest.qcow2; echo "staged=$(date -u -Is)"; } >/workspace/bench/guest-590.48.01.qcow2.STAGED
timeout --signal=TERM 600 bash scripts/bench/provision_guest_gfx.sh host >/root/prov/ada2build_gfx_host_prepare.log 2>&1
timeout --signal=TERM 600 env KF_GUEST_IMG=/workspace/bench/guest-590.48.01.qcow2 bash scripts/bench/provision_guest_gfx.sh >/root/prov/ada2build_gfx_guest590_prepare.log 2>&1
test -z "$(pgrep -x qemu-system-x86 || true)"
check_deadline
bash scripts/bench/gfx/gfx_steps.sh host /root/gfxbin >/root/prov/ada2build590_native_gfx.log 2>&1
for step in 2 3 4 5; do grep -q "^GFX_S$step=PASS$" /root/prov/ada2build590_native_gfx.log; done
dmesg | grep 'NVRM: Xid' >/root/prov/ada2build590_before_headless_xids.log || true
timeout --signal=TERM 900 python3 /root/ada2build_headless590.py >/root/prov/ada2build_headless590.log 2>&1
dmesg | grep 'NVRM: Xid' >/root/prov/ada2build590_after_headless_xids.log || true
cmp /root/prov/ada2build590_before_headless_xids.log /root/prov/ada2build590_after_headless_xids.log
echo "FAT590_CUDA_GRAPHICS_COMPLETE $(date -u -Is)"
