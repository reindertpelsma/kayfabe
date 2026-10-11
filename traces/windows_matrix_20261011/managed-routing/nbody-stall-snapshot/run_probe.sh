#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
REV=72741e2b
TREE=/data/kf-managed-host
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
MEDIA=$W/codex-supplement-20261011/full-image-a61af1c3/kfapps_supplement.img
RUN=$W/codex-nbody-stall-snapshot-20261011-$REV
POST=/tmp/kf-nbody-stall-post.sh
START=$(date -u '+%Y-%m-%d %H:%M:%S UTC')
export KF_LOCKED=1
! pgrep qemu-system
! test -e "$RUN/session.log"
! test -e "$W/kf3-bins/$REV"
for n in {1360..1364}; do
 ! test -e "$W/boundary-kayfabe-$n"
 ! test -e "$W/windows-broker-run$n.state"
done
test -f "$TREE/scripts/bench/windows/signin.py"
echo "NBODY_STALL_SNAPSHOT_START=$(date -Is) product=$REV harness=72741e2b"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
cleanup() {
 rc=$?; trap - EXIT
 for state in "$W"/windows-broker-run{1360..1364}.state; do
  test -f "$state" || continue
  n=${state##*run}; n=${n%.state}; bash "$B" stop "$n" || true
 done
 bash "$POST"
 python3 /tmp/kf-capture-xids.py "$RUN" "$START" || true
 rm -f "$W/kf3-bins/$REV"
 echo "NBODY_STALL_SNAPSHOT_EXIT=$rc at=$(date -Is)"
}
trap cleanup EXIT
export WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS=KF3_PT_STALL_SNAPSHOT=1 WIN_DISABLE_MPO=1
export WIN_PRE_GUEST_CMD="test \$(df -B1 --output=avail /var/lib | tail -1) -gt 107374182400 && bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash $POST"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
# Hypothesis: bounded snapshots identify a SW/deferred method on the faulting
# nbody GR channel near the ordinary-run DEVICE fault.
# Falsifier: a valid late snapshot of that channel excludes such a method.
# Missing coverage or exhausted budgets is INCONCLUSIVE, not falsification.
# If logging changes the failure, retain the ordinary TIMEOUT as the oracle.
# This is measurement only: original app, deadlines and product unchanged.
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" --run-dir "$RUN" \
 --kf3-rev "$REV" --broker "$B" --run-base 1360 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$MEDIA" --apps nbody_demo_gl --tier 3 --per-guest 1 --no-isolate
