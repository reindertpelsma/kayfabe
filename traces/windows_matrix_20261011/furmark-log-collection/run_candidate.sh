#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
REV=72741e2b
TREE=/data/kf-furmark-log-bench-0c5fa450
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
MEDIA=$W/codex-supplement-20261011/full-image-a61af1c3/kfapps_supplement.img
RUN=$W/codex-furmark-log-fresh-20261011-$REV
POST=/tmp/kf-furmark-log-post.sh
START=$(date -u '+%Y-%m-%d %H:%M:%S UTC')
export KF_LOCKED=1
! pgrep qemu-system
! test -e "$RUN/session.log"
! test -e "$W/kf3-bins/$REV"
for n in {1370..1374}; do
 ! test -e "$W/boundary-kayfabe-$n"
 ! test -e "$W/windows-broker-run$n.state"
done
test -f "$TREE/scripts/bench/windows/signin.py"
echo "FURMARK_LOG_FRESH_START=$(date -Is) product=$REV harness=0c5fa450"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
cleanup() {
 rc=$?; trap - EXIT
 for state in "$W"/windows-broker-run{1370..1374}.state; do
  test -f "$state" || continue
  n=${state##*run}; n=${n%.state}; bash "$B" stop "$n" || true
 done
 bash "$POST"
 python3 /tmp/kf-capture-xids.py "$RUN" "$START" || true
 rm -f "$W/kf3-bins/$REV"
 echo "FURMARK_LOG_FRESH_EXIT=$rc at=$(date -Is)"
}
trap cleanup EXIT
export WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS= WIN_DISABLE_MPO=1
export WIN_PRE_GUEST_CMD="test \$(df -B1 --output=avail /var/lib | tail -1) -gt 107374182400 && bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash $POST"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
# Hypothesis: the existing NVIDIA renderer proof is present in the stress
# run's own log, missed by the old root-first-*.log collector.
# Falsifier: unique-suffix logs contain no NVIDIA identity, or the unchanged
# workload fails/recovers/faults. Missing capture is inconclusive about GPU work.
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" --run-dir "$RUN" \
 --kf3-rev "$REV" --broker "$B" --run-base 1370 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$MEDIA" --apps furmark_gl_stress --tier 3 --per-guest 1 --no-isolate
