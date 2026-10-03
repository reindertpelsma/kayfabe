#!/bin/bash
# dispsw_run.sh <tag> [kf3-extra] — one display-lane run with the host probe around it (the x11-dispsw
# A/B, docs/design/V3_DISPLAY.md; traces/v3_display/dispsw_20261003/). Run as root on the bench host,
# from anywhere, detached (`nohup … &`): it writes /root/dsw/<tag>.log with a START line, the REV it
# ran, the lane's DISPLAY_* lines, LANE_RC, the probe's DSW_TRACE_* totals and an EXIT line — wait
# for EXIT, never for "the file stopped growing".
#   A: dispsw_run.sh dsw_off               B: dispsw_run.sh dsw_on x11-dispsw=on
#   env passes through (DISPLAY_X11_BARE=1 adds the bare-X11 step; DSW_TR / KFDSW_SRC override the probe)
TAG=${1:?tag}; EXTRA=${2:-}
mkdir -p /root/dsw
exec > /root/dsw/$TAG.log 2>&1
echo "START $TAG extra=[$EXTRA] $(date -Is)"
cd /root/kayfabe || { echo "EXIT no-repo $(date -Is)"; exit 1; }
echo "REV $(git rev-parse --short=8 HEAD) dirty=$(git status --porcelain | grep -v '^??' | wc -l)"
TR=${DSW_TR:-scripts/bench/display/dispsw_trace.sh}
bash $TR start /root/dsw/${TAG}_ktrace.log
if [ -n "$EXTRA" ]; then
  DISPLAY_DESKTOP=1 KF_DEVICE=kf3 DISPLAY_KF3_EXTRA="$EXTRA" bash scripts/bench/display/lane.sh "$TAG"
else
  DISPLAY_DESKTOP=1 KF_DEVICE=kf3 bash scripts/bench/display/lane.sh "$TAG"
fi
echo "LANE_RC=$?"
bash $TR stop /root/dsw/${TAG}_ktrace.log
echo "EXIT $(date -Is)"
