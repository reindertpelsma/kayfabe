#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
TREE=/data/kf-managed-host
REV=$(git -C "$TREE" rev-parse --short=8 HEAD)
RUN=$W/codex-managed-route-focus-20261011-$REV
REG=$W/codex-managed-route-regression-20261011-$REV
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
MEDIA=$W/codex-supplement-20261011/full-image-a61af1c3/kfapps_supplement.img
POST=/tmp/kf-managed-focus-post.sh
START=$(date -u '+%Y-%m-%d %H:%M:%S UTC')
export KF_LOCKED=1
! pgrep qemu-system
grep -q '^MANAGED_ROUTE_REGRESSION_EXIT=0 ' "$W/codex-managed-route-regression.log"
grep -q 'V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS' "$REG/logs/gates.log"
grep -q 'FAST_SUITE_PASS=30' "$REG/logs/fast.log"
! test -e "$RUN/session.log"
! test -e "$W/kf3-bins/$REV"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
echo "MANAGED_ROUTE_FOCUS_START=$(date -Is) source=$(git -C "$TREE" rev-parse HEAD) media_product=a61af1c3"
cleanup() {
 rc=$?; trap - EXIT
 for state in "$W"/windows-broker-run{1320..1329}.state; do
  test -f "$state" || continue
  n=${state##*run}; n=${n%.state}; bash "$B" stop "$n" || true
 done
 bash "$POST"
 python3 /tmp/kf-capture-xids.py "$RUN" "$START" || true
 rm -f "$W/kf3-bins/$REV"
 echo "MANAGED_ROUTE_FOCUS_EXIT=$rc at=$(date -Is)"
}
trap cleanup EXIT
export WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS= WIN_DISABLE_MPO=1
export WIN_PRE_GUEST_CMD="test \$(df -B1 --output=avail /var/lib | tail -1) -gt 107374182400 && bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash $POST"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
# Hypothesis: KERNEL+UVM_OWNED translation removes the physical-copy fault.
# Falsifier: unchanged managed allocation/visibility fails in180s, refuses an
# authored operation, or produces a host Xid. Managed is FIRST in a fresh VM.
# Gears/vectorAdd/CUP8 are independent regression checks, with original oracles.
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" --run-dir "$RUN" \
 --kf3-rev "$REV" --broker "$B" --run-base 1320 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$MEDIA" --apps managed_t,glgears_wgl,vectorAdd_demo,cup8_ladder --tier 3 --per-guest 4 --no-isolate
python3 - "$RUN/win.res" <<'CHECK'
import pathlib,sys
rows={}
for l in pathlib.Path(sys.argv[1]).read_text().splitlines():
 if l.startswith('APPRES '):
  d=dict(x.split('=',1) for x in l.split() if '=' in x);rows[d['app']]=d
assert set(rows)=={'managed_t','glgears_wgl','vectorAdd_demo','cup8_ladder'},rows
assert all(d['verdict']=='PASS' and d['guest_tdr']=='0' and d['gsp_cycles']=='0' for d in rows.values()),rows
print('MANAGED_ROUTE_FOCUS_CLEAN_PASS=4/4')
CHECK
