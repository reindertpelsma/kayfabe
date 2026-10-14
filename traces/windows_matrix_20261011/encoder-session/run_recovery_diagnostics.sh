#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
TREE=/data/kf-encoder-host
REV=a61af1c3
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
MEDIA=$W/codex-supplement-20261011/full-image-$REV/kfapps_supplement.img
RUN=$W/codex-recovery-diagnostics-20261011-$REV
START=$(date -u '+%Y-%m-%d %H:%M:%S UTC')
export KF_LOCKED=1
! pgrep qemu-system
! test -e "$RUN/edge/session.log"
! test -e "$W/kf3-bins/$REV"
echo "RECOVERY_DIAGNOSTICS_START=$(date -Is) source=$(git -C "$TREE" rev-parse HEAD)"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
cleanup() {
 rc=$?; trap - EXIT
 for state in "$W"/windows-broker-run{1260..1269}.state; do
  test -f "$state" || continue
  n=${state##*run}; n=${n%.state}; bash "$B" stop "$n" || true
 done
 bash "$W/pti-iommu_nogdm.sh" DMA-FQ
 ! pgrep qemu-system
 python3 - <<'PY'
import pathlib,json,subprocess
w=pathlib.Path('/var/lib/kf-windows-20261005')
for n in range(1260,1270):
 p=w/f'boundary-kayfabe-{n}'/'windows.qcow2'
 if not p.exists():continue
 d=json.loads(subprocess.check_output(['qemu-img','info','--output=json',str(p)]))
 assert d.get('full-backing-filename')==str(w/'baseline/windows.qcow2')
 print('RECLAIM_STOPPED_OVERLAY',n,p.stat().st_blocks*512);p.unlink()
PY
 python3 /tmp/kf-capture-xids.py "$RUN" "$START" || true
 rm -f "$W/kf3-bins/$REV"
 echo "RECOVERY_DIAGNOSTICS_EXIT=$rc at=$(date -Is)"
}
trap cleanup EXIT
export WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS= WIN_DISABLE_MPO=1
export WIN_PRE_GUEST_CMD="test \$(df -B1 --output=avail /var/lib | tail -1) -gt 107374182400 && bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash $W/pti-iommu_nogdm.sh DMA-FQ"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
# Hypothesis: ordinary first launch can trigger recovery without video encoding.
# Falsifier: the normal-profile launch survives60s with zero guest/GSP recovery.
# Launch-only diagnostic: no GPU correctness or canonical matrix pass claim.
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" --run-dir "$RUN/edge" \
 --kf3-rev "$REV" --broker "$B" --run-base 1260 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$MEDIA" --apps-json /tmp/kf-edge-default-launch-diagnostic.json \
 --apps edge_default_launch_diagnostic --tier 3 --per-guest 1 --no-isolate
# Hypothesis: managed-memory failure persists in a fresh isolated guest.
# Falsifier: unchanged allocation/work stages finish inside the original180s.
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" --run-dir "$RUN/managed" \
 --kf3-rev "$REV" --broker "$B" --run-base 1261 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$MEDIA" --apps managed_t --tier 3 --per-guest 1 --no-isolate
# Hypothesis: the120s stream timeout is dominated by fresh Python/CuPy startup.
# Falsifier: timed preload/GPU readback completes, then the UNCHANGED120s stream
# test still hangs. Diagnostic has a separate180s budget; neither original
# verdict nor canonical stream timeout is changed.
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" --run-dir "$RUN/startup-stream" \
 --kf3-rev "$REV" --broker "$B" --run-base 1262 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$MEDIA" --apps-json /tmp/kf-startup-stream-diagnostic.json \
 --apps startup_stream_diagnostic,stream_perthread_t --tier 3 --per-guest 2 --no-isolate
# The earlier encoded-frame runs already identify a specific kernel video-method
# refusal. Test authored native semaphore behavior before implementing a decoder.
# Falsifier for each case: wrong GPU-written payload, absent real GPU tail, or Xid.
! pgrep qemu-system
mkdir -p "$RUN/native-video-semaphore"
echo 'VIDEO_SEMAPHORE_SOURCE=35f97433' > "$RUN/native-video-semaphore/probe.log"
echo '81f695d518606def466684ee0cae8d64e0cbb55f66bfde752b9f42bdc029eb94  /tmp/kf-video-semaphore-probe' | sha256sum --check >> "$RUN/native-video-semaphore/probe.log"
for mode in release trap; do
 echo "VIDEO_SEMAPHORE_START mode=$mode at=$(date -Is)" >> "$RUN/native-video-semaphore/probe.log"
 set +e
 timeout 30 setpriv --reuid=65534 --regid=65534 --clear-groups /tmp/kf-video-semaphore-probe "$mode" 1 >> "$RUN/native-video-semaphore/probe.log" 2>&1
 probe_rc=$?
 set -e
 echo "VIDEO_SEMAPHORE_EXIT mode=$mode rc=$probe_rc at=$(date -Is)" >> "$RUN/native-video-semaphore/probe.log"
 test "$probe_rc" = 0
done
