#!/usr/bin/env bash
set -euo pipefail
RUN_START=$(date -u "+%Y-%m-%d %H:%M:%S UTC")
W=/var/lib/kf-windows-20261005
TREE=/data/kf-slice-matrix-8037e19b
REV=0f9c01f9
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
RUN=$W/codex-matrix-slice-full-20261011
POST=/tmp/kf-slice-post2.sh
cleanup() {
 rc=$?; trap - EXIT
 # BrokerVm normally stops each guest; recover only this campaign's active run.
 for state in "$W"/windows-broker-run{860..1059}.state; do
  test -f "$state" || continue
  n=${state##*run}; n=${n%.state}
  bash "$B" stop "$n" || true
 done
 bash "$POST"
 python3 /tmp/kf-capture-xids.py "$RUN" "$RUN_START" || true
 rm -f "$W/kf3-bins/$REV"
 echo "MATRIX_SLICE_FULL_EXIT=$rc at=$(date -Is)"
}
! pgrep qemu-system
! test -e "$RUN/session.log"
for n in {860..1059}; do ! test -e "$W/boundary-kayfabe-$n"; done
# Initial cold BlackScholes and subsequent gears timeouts remain recorded.
# Require the completed independent GPU checks and fresh diagnostic recovery.
python3 - <<'PYREADY'
from pathlib import Path
w=Path('/var/lib/kf-windows-20261005')
for name, ids in (
 ('focus', ['cuda_callback','cuda_occupancy','deviceQuery_demo','matmul_t']),
 ('diagnostic', ['glgears_wgl','torch_startup_diagnostic','blackscholes_t'])):
 rows={}
 for line in (w/f'codex-matrix-slice-{name}-20261011'/'win.res').read_text().splitlines():
  kv=dict(x.split('=',1) for x in line.split() if '=' in x)
  rows[kv['app']]=kv['verdict']
 assert all(rows.get(a)=='PASS' for a in ids), (name,rows)
PYREADY
! test -e "$W/kf3-bins/$REV"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
trap cleanup EXIT
export KF_LOCKED=1 WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS= WIN_DISABLE_MPO=1
export WIN_PRE_GUEST_CMD="test \$(df -B1 --output=avail /var/lib | tail -1) -gt 107374182400 && bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash $POST"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" \
 --run-dir "$RUN" --kf3-rev "$REV" --broker "$B" --run-base 860 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$W/appmatrix/image/kfapps_exfat.img" --apps "$(cat /tmp/kf-slice-app-order.txt)" --tier 3 --per-guest 8 \
 --exclude overlay_yuv_fullscreen,overlay_yuv_hide,overlay_yuv_move,overlay_yuv_occlude,overlay_yuv_probe,overlay_yuv_recreate,overlay_yuv_resize,overlay_yuv_soak
