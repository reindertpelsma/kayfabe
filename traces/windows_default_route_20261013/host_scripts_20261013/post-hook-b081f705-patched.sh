#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
if pgrep qemu-system; then
    echo 'POST_GUEST_REFUSED another QEMU remains; no IOMMU change'
    exit 1
fi
bash "$W/pti-iommu_nogdm.sh" DMA-FQ
rm -f /tmp/kf-stop-winprod
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
# 2026-10-13 (coordinator): with no QEMU running, drop finished disposable overlays of the regression-matrix run numbers (children of baseline only),
# so a guest restart does not leave the disk under the 100 GiB floor and break the next pre-guest hook. Evidence (logs) stays.
for q in "$W"/boundary-kayfabe-17[0-9][0-9]/windows.qcow2 "$W"/boundary-kayfabe-1[89][0-9][0-9]/windows.qcow2; do
    [ -f "$q" ] || continue
    if qemu-img info "$q" 2>/dev/null | grep -q "backing file: $W/baseline/windows.qcow2"; then rm -f "$q"; fi
done
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 107374182400
