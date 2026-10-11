#!/usr/bin/env bash
# Caller owns the GPU lock. This stage runs before manual VM creation only.
set -euo pipefail
W=/var/lib/kf-windows-20261005
echo "NATIVE_HOLD_BEFORE_MANUAL_START=$(date -Is)"
set +e
python3 /tmp/kf-native-hold-and-journal.py > "$W/codex-native-hold-queued.log" 2>&1
native_rc=$?
set -e
echo "NATIVE_HOLD_BEFORE_MANUAL_EXIT=$native_rc at=$(date -Is)"
# Diagnostic FAIL/refusal stays in its own evidence. Unsafe host state blocks
# manual launch. Never touch input or run a test after the owner VM is ready.
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
exec bash /tmp/kf-prod-manual-1380-frozen.sh
