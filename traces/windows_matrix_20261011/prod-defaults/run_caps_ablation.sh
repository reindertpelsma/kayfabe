#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
REV=3252a087
TREE=/data/kf-prod-defaults-host
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
MEDIA=$W/codex-supplement-20261011/full-image-a61af1c3/kfapps_supplement.img
RUN=$W/codex-prod-caps-ablation-20261011-$REV
POST=/tmp/kf-prod-caps-post.sh
START=$(date -u '+%Y-%m-%d %H:%M:%S UTC')
export KF_LOCKED=1
! pgrep qemu-system
! test -e "$RUN/session.log"
! test -e "$W/kf3-bins/$REV"
for n in {1400..1404}; do
 ! test -e "$W/boundary-kayfabe-$n"
 ! test -e "$W/windows-broker-run$n.state"
done
test -f "$TREE/scripts/bench/windows/signin.py"
echo "PROD_CAPS_ABLATION_START=$(date -Is) product=$REV harness=3252a087"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
cleanup() {
 rc=$?; trap - EXIT
 for state in "$W"/windows-broker-run{1400..1404}.state; do
  test -f "$state" || continue
  n=${state##*run}; n=${n%.state}; bash "$B" stop "$n" || true
 done
 bash "$POST"
 python3 /tmp/kf-capture-xids.py "$RUN" "$START" || true
 rm -f "$W/kf3-bins/$REV"
 echo "PROD_CAPS_ABLATION_EXIT=$rc at=$(date -Is)"
}
trap cleanup EXIT
export WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS= WIN_DROP="KF3_TIMER_MAP KF3_DISPLAY_CAPS_PROBE" WIN_DISABLE_MPO=1
export WIN_PRE_GUEST_CMD="test \$(df -B1 --output=avail /var/lib | tail -1) -gt 107374182400 && bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash $POST"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
# Hypothesis: the current derived display caps suffice for Windows580.88
# without the captured caps probe. Baseline is the new-timer focus, same source.
# Falsifier: adapter/boot/app/proof failure, recovery or unexpected host Xid.
# This experiment changes only caps-probe selection; retain failure as evidence.
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" --run-dir "$RUN" \
 --kf3-rev "$REV" --broker "$B" --run-base 1400 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$MEDIA" --apps glgears_wgl,vectorAdd_demo,cup8_ladder --tier 3 --per-guest 3 --no-isolate
