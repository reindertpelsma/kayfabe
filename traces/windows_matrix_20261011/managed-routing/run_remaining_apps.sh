#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
TREE=/data/kf-managed-host
REV=72741e2b
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
MEDIA=$W/codex-supplement-20261011/full-image-a61af1c3/kfapps_supplement.img
RUN=$W/codex-managed-remaining-apps-20261011-$REV
POST=/tmp/kf-remaining-post.sh
START=$(date -u '+%Y-%m-%d %H:%M:%S UTC')
export KF_LOCKED=1
! pgrep qemu-system
grep -q '^MANAGED_ROUTE_FOCUS_EXIT=0 ' "$W/codex-managed-route-focus.log"
grep -q '^MANAGED_ROUTE_FOCUS_CLEAN_PASS=4/4' "$W/codex-managed-route-focus.log"
python3 -c 'import json; assert json.load(open("/var/lib/kf-windows-20261005/codex-managed-route-focus-20261011-72741e2b/kernel-xids.json"))["xid_records"] == 0'
test "$(git -C "$TREE" rev-parse --short=8 HEAD)" = "$REV"
! test -e "$RUN/session.log"
! test -e "$W/kf3-bins/$REV"
echo "REMAINING_START=$(date -Is) source=$(git -C "$TREE" rev-parse HEAD)"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
cleanup() {
 rc=$?; trap - EXIT
 for state in "$W"/windows-broker-run{1270..1319}.state; do
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
