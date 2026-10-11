#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
TREE=/data/kf-supplement-harness-c5c97c00
REV=0f9c01f9
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
RUN=$W/codex-matrix-supplement-c5c97c00-20261011
MEDIA=$W/codex-supplement-20261011/full-image-c5c97c00/kfapps_supplement.img
POST=/tmp/kf-supplement-post.sh
START=$(date -u '+%Y-%m-%d %H:%M:%S UTC')
export KF_LOCKED=1
cleanup() {
 rc=$?; trap - EXIT
 for state in "$W"/windows-broker-run{1060..1159}.state; do
  test -f "$state" || continue
  n=${state##*run}; n=${n%.state}
  bash "$B" stop "$n" || true
 done
 bash "$POST"
 python3 /tmp/kf-capture-xids.py "$RUN" "$START" || true
 rm -f "$W/kf3-bins/$REV"
 echo "SUPPLEMENT_EXIT=$rc at=$(date -Is)"
}
echo "SUPPLEMENT_START at=$(date -Is)"
! pgrep qemu-system
! test -e "$RUN/session.log"
! test -e "$W/kf3-bins/$REV"
# Hypothesis: the authored four-zero-byte session contract works on595.91.07.
# Falsifier: any acquire/release failure or changed reply. Run as nobody, no guest bytes.
set +e
setpriv --reuid=65534 --regid=65534 --clear-groups timeout 30 /tmp/kf-encoder-session-probe > "$W/codex-encoder-session-probe.log" 2>&1
probe_rc=$?
set -e
echo "ENCODER_PROBE_EXIT=$probe_rc at=$(date -Is)" >> "$W/codex-encoder-session-probe.log"
# Build complete fresh-guest media, preserving the original verified base image.
bash "$TREE/traces/windows_matrix_20261011/supplement/build_full_media.sh" "$TREE"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
trap cleanup EXIT
export WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS= WIN_DISABLE_MPO=1
export WIN_PRE_GUEST_CMD="test \$(df -B1 --output=avail /var/lib | tail -1) -gt 107374182400 && bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash $POST"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
APPS=$(python3 - "$W/codex-matrix-slice-full-20261011" <<'PY'
import pathlib,sys
r=pathlib.Path(sys.argv[1]); verdict={}
for name in ('win.res','win_isolated.res'):
 p=r/name
 if not p.exists():continue
 for l in p.read_text().splitlines():
  if not l.startswith('APPRES '):continue
  d=dict(x.split('=',1) for x in l.split() if '=' in x);verdict[d['app']]=d['verdict']
a=['hf_generate','cdp_quicksort','vectorAdd_demo','cup8_ladder','stream_perthread_t','cupy_sha256','cupy_cg','cupy_asynccopy','cupy_check']
a+=sorted(k for k,v in verdict.items() if v!='PASS' and k not in a and k not in ('clpeak_cuda','managed_t','nvenc_av1','nvenc_h264','nvenc_hevc','scale_cuda_nvenc'))
# Long-running clpeak and the managed-memory investigation each start in a fresh VM.
print(','.join(a))
PY
)
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" --run-dir "$RUN" \
 --kf3-rev "$REV" --broker "$B" --run-base 1060 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$MEDIA" --apps "$APPS" --tier 3 --per-guest 4 --no-isolate
# Managed and clpeak are scheduled separately after review of the broad results.
