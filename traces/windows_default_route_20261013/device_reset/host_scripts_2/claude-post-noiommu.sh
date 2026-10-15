#!/usr/bin/env bash
# Post-guest hook for runs while the owner's terminal holds /dev/nvidia (IOMMU group stays in identity mode):
# refuses only if another QEMU remains; no IOMMU change.
set -euo pipefail
if pgrep qemu-system; then echo 'POST_GUEST_REFUSED another QEMU remains'; exit 1; fi
rm -f /tmp/kf-stop-winprod
