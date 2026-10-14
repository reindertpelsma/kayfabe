#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
echo "REFERENCE_BEFORE_DEFAULTS_START=$(date -Is)"
set +e
bash "$W/codex-vfio-gravity-20261011/tooling/run_reference.sh" > "$W/codex-vfio-gravity-reference.log" 2>&1
reference_rc=$?
set -e
echo "VFIO_GRAVITY_REFERENCE_RECORDED_RC=$reference_rc at=$(date -Is)"
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
bash /tmp/kf-prod-defaults-campaign-without-reference.sh
