#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
RUN=$W/codex-native-deferred-update-20261011
PROBE=/tmp/kf-deferred-update-probe
START=$(date -u '+%Y-%m-%d %H:%M:%S UTC')
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
! test -e "$RUN/probe.log"
mkdir -p "$RUN"
echo "DEFERRED_UPDATE_START=$(date -Is)" > "$RUN/probe.log"
sha256sum "$PROBE" >> "$RUN/probe.log"
cleanup() {
 rc=$?; trap - EXIT
 python3 /tmp/kf-capture-xids.py "$RUN" "$START" || true
 echo "DEFERRED_UPDATE_EXIT=$rc at=$(date -Is)" >> "$RUN/probe.log"
}
trap cleanup EXIT
# Hypothesis: public unprivileged5080 V2 registration permits an authored
# self-owned channel-info update to execute at the real GPU software trigger.
# Falsifier: registration refusal, GPU tail missing, cleanup failure or Xid.
# Same-config update only: success does NOT prove migration or guest app repair.
failed=0
for mode in nop update; do
 echo "DEFERRED_UPDATE_CASE_START mode=$mode at=$(date -Is)" >> "$RUN/probe.log"
 set +e
 timeout 30 setpriv --reuid=65534 --regid=65534 --clear-groups "$PROBE" "$mode" >> "$RUN/probe.log" 2>&1
 rc=$?
 set -e
 echo "DEFERRED_UPDATE_CASE_EXIT mode=$mode rc=$rc at=$(date -Is)" >> "$RUN/probe.log"
 if test "$rc" != 0; then failed=1; fi
done
test "$failed" = 0
