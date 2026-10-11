#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
TREE=/data/kf-managed-host
REV=72741e2b
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
MEDIA=$W/codex-supplement-20261011/full-image-a61af1c3/kfapps_supplement.img
RUN=$W/codex-managed-remaining-apps-20261011-$REV
POST=/tmp/kf-reviewed-remaining-post.sh
START=$(date -u '+%Y-%m-%d %H:%M:%S UTC')
export KF_LOCKED=1
! pgrep qemu-system
# Managed remains a known TIMEOUT; continue coverage only after its fresh-VM
# regression checks pass and the timestamp-bounded host journal is clean.
grep -q '^MANAGED_ROUTE_FOCUS_EXIT=1 ' "$W/codex-managed-route-focus.log"
python3 - "$W/codex-managed-route-focus-20261011-$REV/win.res" <<'CHECK'
import pathlib,sys
rows={}
for line in pathlib.Path(sys.argv[1]).read_text().splitlines():
 if line.startswith('APPRES '):
  d=dict(x.split('=',1) for x in line.split() if '=' in x);rows[d['app']]=d
assert set(rows)=={'managed_t','glgears_wgl','vectorAdd_demo','cup8_ladder'},rows
assert rows['managed_t']['verdict']=='TIMEOUT',rows
assert all(d['guest_tdr']=='0' and d['gsp_cycles']=='0' for d in rows.values()),rows
assert all(d['verdict']=='PASS' for k,d in rows.items() if k!='managed_t'),rows
print('REVIEWED_FOCUS=3_clean_smokes_plus_known_managed_timeout')
CHECK
python3 -c 'import json; assert json.load(open("/var/lib/kf-windows-20261005/codex-managed-route-focus-20261011-72741e2b/kernel-xids.json"))["xid_records"] == 0'
test "$(git -C "$TREE" rev-parse --short=8 HEAD)" = "$REV"
! test -e "$RUN/session.log"
! test -e "$W/kf3-bins/$REV"
echo "REMAINING_START=$(date -Is) source=$(git -C "$TREE" rev-parse HEAD)"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
cleanup() {
 rc=$?; trap - EXIT
 for state in "$W"/windows-broker-run{1270..1319}.state "$W"/windows-broker-run{1340..1344}.state; do
  test -f "$state" || continue
  n=${state##*run}; n=${n%.state}; bash "$B" stop "$n" || true
 done
 bash "$POST"
 python3 /tmp/kf-capture-xids.py "$RUN" "$START" || true
 rm -f "$W/kf3-bins/$REV"
 echo "REMAINING_EXIT=$rc at=$(date -Is)"
}
trap cleanup EXIT
export WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS= WIN_DISABLE_MPO=1
export WIN_PRE_GUEST_CMD="test \$(df -B1 --output=avail /var/lib | tail -1) -gt 107374182400 && bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash $POST"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
# Bench-only stream change738b5385: remove Torch import from the CuPy-only
# case; preserve original120s and independently verify every result on CPU.
# Falsifier: deadline, failed full readback, driver recovery or new host Xid.
# NbodyGUI is an unchanged fresh retry of the original production oracle.
bash /data/kf-stream-bench-738b5385/scripts/bench/windows/appmatrix/run_windows_apps.sh \
 --run-dir "$W/codex-direct-stream-nbody-20261011-$REV" \
 --kf3-rev "$REV" --broker "$B" --run-base 1340 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$MEDIA" --apps stream_perthread_t,nbody_demo_gl --tier 3 --per-guest 1 --no-isolate

# Hypothesis: FurMark success survives removing bounded diagnostic logging.
# Falsifier: the unchanged benchmark fails, recovers, or has a host Xid with
# WIN_FLAGS empty. Preserve the separate traced PASS and all original failures.
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" --run-dir "$W/codex-managed-furmark-off-20261011-$REV" \
 --kf3-rev "$REV" --broker "$B" --run-base 1270 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$MEDIA" --apps furmark_gl_bench --tier 3 --per-guest 1 --no-isolate
# Coverage, not a root-cause experiment: remaining Windows-applicable rows use
# unchanged workloads/oracles/timeouts. A failed oracle is a failure, even if
# other apps pass. GravityMarkVK also times out on matching stock Windows.
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" --run-dir "$RUN" \
 --kf3-rev "$REV" --broker "$B" --run-base 1280 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$MEDIA" --apps valley_opengl,vkcube,llama_cuda_gen,furmark_gl_stress,torch_burn,dxva2_h264,dxdiag_report,llama_cuda_bench,llama_vulkan_bench,blender_cycles,vkpeak,gravitymark_d3d12_raster,gravitymark_d3d12_rt,torch_ai_bench,gravitymark_d3d12,gravitymark_gl,gravitymark_d3d11,clpeak_ocl,gravitymark_vk \
 --tier 3 --per-guest 4 --no-isolate
# Isolate the long CUDA clpeak test; original1500s budget stays unchanged.
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" --run-dir "$W/codex-managed-clpeak-fresh-20261011-$REV" \
 --kf3-rev "$REV" --broker "$B" --run-base 1310 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$MEDIA" --apps clpeak_cuda --tier 3 --per-guest 1 --no-isolate
