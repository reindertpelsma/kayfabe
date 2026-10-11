#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
TREE=/data/kf-encoder-host
REV=a61af1c3
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
MEDIA=$W/codex-supplement-20261011/full-image-$REV/kfapps_supplement.img
POST=/tmp/kf-encoder-post.sh
RUN=$W/codex-corrected-apps-20261011-$REV
START=$(date -u '+%Y-%m-%d %H:%M:%S UTC')
export KF_LOCKED=1
! pgrep qemu-system
test "$(git -C "$TREE" rev-parse --short=8 HEAD)" = "$REV"
grep -q '^ENCODER_REGRESSION_EXIT=0 ' "$W/codex-encoder-regression.log"
! test -e "$RUN/session.log"
! test -e "$W/kf3-bins/$REV"
echo "CORRECTED_START=$(date -Is) source=$(git -C "$TREE" rev-parse HEAD)"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
cleanup() {
 rc=$?; trap - EXIT
 for state in "$W"/windows-broker-run{1170..1259}.state; do
  test -f "$state" || continue
  n=${state##*run}; n=${n%.state}; bash "$B" stop "$n" || true
 done
 bash "$POST"
 python3 /tmp/kf-capture-xids.py "$RUN" "$START" || true
 rm -f "$W/kf3-bins/$REV"
 echo "CORRECTED_EXIT=$rc at=$(date -Is)"
}
trap cleanup EXIT
export WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS= WIN_DISABLE_MPO=1
export WIN_PRE_GUEST_CMD="test \$(df -B1 --output=avail /var/lib | tail -1) -gt 107374182400 && bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash $POST"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
# Hypothesis: the reported Edge launch recovery reproduces in a fresh browser-only
# session. Falsifier: both unchanged Edge GPU tests finish with no recovery.
# This tests those workloads, not every possible manual Edge launch/profile.
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" --run-dir "$W/codex-edge-fresh-20261011-$REV" \
 --kf3-rev "$REV" --broker "$B" --run-base 1170 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$MEDIA" --apps edge_webgl,edge_webgpu --tier 3 --per-guest 2 --no-isolate
# Validation: corrected packaging and numerical proof must execute real GPU work.
# Falsifier: any corrected case still fails its unchanged numerical/GUI requirement.
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" --run-dir "$RUN" \
 --kf3-rev "$REV" --broker "$B" --run-base 1180 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$MEDIA" --apps oceanFFT_demo,randomFog_demo,hf_generate,cdp_quicksort,vectorAdd_demo,cup8_ladder,stream_perthread_t,cupy_sha256,cupy_cg,cupy_asynccopy,cupy_check \
 --tier 3 --per-guest 4 --no-isolate
# Diagnostic: only the bounded PB peek flag changes. Exhausting its first512
# GP entries/64 software-method budget makes absence inconclusive.
export WIN_FLAGS=KF3_RELAY_PB_PEEK
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" --run-dir "$W/codex-furmark-peek-20261011-$REV" \
 --kf3-rev "$REV" --broker "$B" --run-base 1250 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$MEDIA" --apps furmark_gl_bench --tier 3 --per-guest 1 --no-isolate
