#!/usr/bin/env bash
# Queued stage only: caller retains the GPU lock; read-only query, then native diagnostic.
set -euo pipefail
W=/var/lib/kf-windows-20261005
PRIVATE=$W/codex-private-gravity-20261011/rc-journal
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
! pgrep qemu-system
test ! -e "$PRIVATE"
install -d -m 0700 "$PRIVATE"
OUT=$(mktemp -d /var/tmp/kf-rc-journal-20261011.XXXXXXXX)
chown 65534:65534 "$OUT"
echo "RC_JOURNAL_START=$(date -Is) uid=65534 no_channel_no_gpu_work"
# Falsifier: a matching record lacks extra fault details. Refusal/absent records
# are inconclusive; VFIO detach may have reset the journal. Never clear history.
set +e
timeout 30 setpriv --reuid=65534 --regid=65534 --clear-groups \
 env -i PATH=/usr/bin:/bin \
 /tmp/kf-rc-journal-probe-428ad66d7 595.91.07 \
 /tmp/kf-rc-layout-595.91.07.txt "$OUT/records" > "$PRIVATE/query.log" 2>&1
query_rc=$?
set -e
chown -R root:root "$OUT"
chmod 0700 "$OUT"
mv "$OUT" "$PRIVATE/raw"
printf 'RC_JOURNAL_EXIT=%s at=%s\n' "$query_rc" "$(date -Is)" | tee -a "$PRIVATE/query.log"
# A diagnostic refusal cannot be promoted to a pass or block unrelated controls.
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
# Separate native experiment: original update vs retaining resources for2s after
# the actual GPU tail. Falsifier for early-cleanup cause: Xid arrives before the
# hold ends. No Xid during hold but fault after cleanup keeps that cause alive;
# absence of an Xid alone does not prove deferred-action success.
set +e
python3 /tmp/kf-native-hold-and-journal.py > "$PRIVATE/native-hold-stage.log" 2>&1
native_rc=$?
set -e
echo "NATIVE_HOLD_STAGE_EXIT=$native_rc at=$(date -Is)"
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
exec bash /tmp/kf-prod-defaults-campaign-frozen-3252.sh
