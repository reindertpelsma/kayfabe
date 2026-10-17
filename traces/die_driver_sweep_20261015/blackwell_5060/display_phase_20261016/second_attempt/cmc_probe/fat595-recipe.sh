#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/gb206phase_cmc_fat595.log 2>&1
trap 'rc=$?; echo "CMC_FAT595_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "CMC_FAT595_QUEUED $(date -u -Is)"
while kill -0 109892 2>/dev/null; do
 test "$(date +%s)" -lt "$(date -u -d '2026-10-16T21:30:00Z' +%s)"
 sleep 3
done
grep -q '^CMC_RAW30_COMPLETE ' /root/prov/gb206phase_cmc_raw.log
test -z "$(pgrep -x qemu-system-x86 || true)"
test "$(date +%s)" -lt "$(date -u -d '2026-10-16T21:30:00Z' +%s)"
cd /root/kayfabe
test "$(git rev-parse --short=9 HEAD)" = e1bd2df4d
echo "CMC_FAT595_PREPARE $(date -u -Is)"
mkdir -p /workspace/drivers/595.91.07
ln /root/NVIDIA-Linux-x86_64-595.91.07.run /workspace/drivers/595.91.07/NVIDIA-Linux-x86_64-595.91.07.run
# Private bench recipe only: cap the stock guest's installer build at4 jobs.
python3 - <<'PY'
from pathlib import Path
p=Path('scripts/drivermatrix/stage_fat_guest.sh');s=p.read_text();assert '-j$(nproc)' in s
Path('/root/gb206phase_cmc_stage_fat.sh').write_text(s.replace('-j$(nproc)','-j4'))
PY
timeout --signal=TERM 900 bash /root/gb206phase_cmc_stage_fat.sh 595.91.07 >/root/prov/gb206phase_cmc_fat_stage.log 2>&1
test -z "$(pgrep -x qemu-system-x86 || true)"
# No GPU timing is active during provisioning; this adds public Khronos test tools only.
timeout --signal=TERM 600 bash scripts/bench/provision_guest_gfx.sh host >/root/prov/gb206phase_cmc_gfx_host_prepare.log 2>&1
timeout --signal=TERM 600 env KF_GUEST_IMG=/workspace/bench/guest-595.91.07.qcow2 bash scripts/bench/provision_guest_gfx.sh >/root/prov/gb206phase_cmc_gfx_guest_prepare.log 2>&1
test -z "$(pgrep -x qemu-system-x86 || true)"
echo "CMC_FAT595_READY $(date -u -Is)"
test "$(date +%s)" -lt "$(date -u -d '2026-10-16T21:45:00Z' +%s)"
bash scripts/bench/gfx/gfx_steps.sh host /root/gfxbin >/root/prov/gb206phase_cmc_native_gfx.log 2>&1
for step in 2 3 4 5; do grep -q "^GFX_S$step=PASS$" /root/prov/gb206phase_cmc_native_gfx.log; done
dmesg | grep 'NVRM: Xid' > /root/prov/gb206phase_cmc_before_headless_xids.log || true
timeout --signal=TERM 900 python3 /root/gb206phase_headless595.py >/root/prov/gb206phase_cmc_headless595.log 2>&1
dmesg | grep 'NVRM: Xid' > /root/prov/gb206phase_cmc_after_headless_xids.log || true
cmp /root/prov/gb206phase_cmc_before_headless_xids.log /root/prov/gb206phase_cmc_after_headless_xids.log
echo "CMC_FAT595_VALIDATION_COMPLETE $(date -u -Is)"
