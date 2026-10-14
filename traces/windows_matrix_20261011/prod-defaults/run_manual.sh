#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
REV=3252a087
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
RUN=$W/boundary-kayfabe-1380
APPROVED=$W/codex-prod-defaults-20261011/manual-approved-revision
export KF_LOCKED=1
! pgrep qemu-system
! test -e "$RUN"
! test -e "$W/kf3-bins/$REV"
test "$(cat "$APPROVED")" = "$REV"
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 107374182400
rm -f /tmp/kf-stop-winprod /tmp/kf-stop-codex-manual-1380
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
cleanup() {
 rc=$?; trap - EXIT
 bash "$B" stop 1380 || true
 bash "$W/pti-iommu_nogdm.sh" DMA-FQ
 rm -f "$W/kf3-bins/$REV" /tmp/kf-stop-winprod /tmp/kf-stop-codex-manual-1380
 echo "MANUAL_SESSION_EXIT=$rc at=$(date -Is)"
}
trap cleanup EXIT
bash "$W/pti-iommu_nogdm.sh" identity
# Clear inherited KF3 switches before applying the explicit production profile.
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
# Production behavior list, no added diagnostics, tracing or observer.
# Native timer comes from code defaults; other promotions remain under audit.
export KF3_REV=$REV WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS= PROD_EXTRA_FLAGS= WIN_DROP=KF3_TIMER_MAP
export WIN_TITLE='kayfabe Windows manual 1380 — no automation' WIN_MAX_SECONDS=0
bash "$B" run 1380
python3 /tmp/kf-prepare-manual-1380.py
QPID=$(sed -n 's/^QPID=//p' "$W/windows-broker-run1380.state")
echo "MANUAL_SESSION_READY=$(date -Is) run=1380 product=$REV no_input=true"
# Hold the shared GPU lock for this one VM. Never restart it or launch a test.
while kill -0 "$QPID" 2>/dev/null; do
 if test -e /tmp/kf-stop-codex-manual-1380; then
  echo "MANUAL_SESSION_STOP_REQUEST=$(date -Is)"
  break
 fi
 sleep 5
done
