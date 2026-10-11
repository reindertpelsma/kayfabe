#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
TREE=/data/kf-encoder-host
REV=$(git -C "$TREE" rev-parse --short=8 HEAD)
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
RUN=$W/codex-encoder-windows-focus-20261011-$REV
MEDIA=$W/codex-supplement-20261011/full-image-$REV/kfapps_supplement.img
POST=/tmp/kf-encoder-post.sh
START=$(date -u '+%Y-%m-%d %H:%M:%S UTC')
export KF_LOCKED=1
! pgrep qemu-system
grep -q '^ENCODER_REGRESSION_EXIT=0 ' "$W/codex-encoder-regression.log"
grep -q 'V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS' "$W/codex-encoder-regression-20261011-$REV/logs/gates.log"
! test -e "$RUN/session.log"
! test -e "$W/kf3-bins/$REV"
echo "ENCODER_WINDOWS_START=$(date -Is) source=$(git -C "$TREE" rev-parse HEAD)"
bash "$TREE/traces/windows_matrix_20261011/supplement/build_full_media.sh" "$TREE"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
cleanup() {
 rc=$?; trap - EXIT
 for state in "$W"/windows-broker-run{1160..1169}.state; do
  test -f "$state" || continue
  n=${state##*run}; n=${n%.state}; bash "$B" stop "$n" || true
 done
 bash "$POST"
 python3 /tmp/kf-capture-xids.py "$RUN" "$START" || true
 rm -f "$W/kf3-bins/$REV"
 echo "ENCODER_WINDOWS_EXIT=$rc at=$(date -Is)"
}
trap cleanup EXIT
export WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS= WIN_DISABLE_MPO=1
export WIN_PRE_GUEST_CMD="test \$(df -B1 --output=avail /var/lib | tail -1) -gt 107374182400 && bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash $POST"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
# Hypothesis: the measured host ABI fixes Windows NVENC session creation.
# Falsifier: any unchanged encoder fails to produce frames after acquire is accepted.
# Gears and zero unexpected recovery remain independent acceptance requirements.
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" --run-dir "$RUN" \
 --kf3-rev "$REV" --broker "$B" --run-base 1160 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$MEDIA" \
 --apps glgears_wgl,nvenc_h264,nvenc_hevc,nvenc_av1 --tier 3 --per-guest 4 --no-isolate
python3 - "$RUN/win.res" <<'PY'
from pathlib import Path
import sys
rows={}
for l in Path(sys.argv[1]).read_text().splitlines():
 if l.startswith('APPRES '):
  d=dict(x.split('=',1) for x in l.split() if '=' in x);rows[d['app']]=d
expected={'glgears_wgl','nvenc_h264','nvenc_hevc','nvenc_av1'}
assert set(rows)==expected,rows
assert all(d['verdict']=='PASS' and d['guest_tdr']=='0' and d['gsp_cycles']=='0' for d in rows.values()),rows
print('ENCODER_WINDOWS_CLEAN_PASS=4/4')
PY
