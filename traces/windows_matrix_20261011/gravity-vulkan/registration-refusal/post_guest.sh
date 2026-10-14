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
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 107374182400
