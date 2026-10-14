#!/usr/bin/env bash
# Trusted host only; caller holds global GPU lock. Disposable baseline child.
set -euo pipefail
REV=${1:?full tested source revision required}
[[ "$REV" =~ ^[0-9a-f]{40}$ ]]
SHORT=${REV:0:8}
W=/var/lib/kf-windows-20261005
TREE=/data/kf-memory-access-host-${REV:0:9}
ROOT=$W/codex-vulkan-default-windows-${REV:0:9}
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
POST=/tmp/kf-fifo-refusal-post-b081f705.sh
BASE=${2:?unused first run number required}
[[ "$BASE" =~ ^[0-9]+$ ]]
test "$(git -C "$TREE" rev-parse HEAD)" = "$REV"
test -z "$(git -C "$TREE" status --porcelain --untracked-files=no)"
test "$(cat "$W/codex-memory-access-linux-${REV:0:9}/EXIT")" = 0
test "$(cat "$W/codex-linux-desktop-${REV:0:9}/EXIT")" = 0
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 107374182400
test ! -e "$ROOT"
test ! -e "$W/boundary-kayfabe-$BASE"
test -f "$W/owner-state/manual1380/KEEP.json"
mkdir -p "$ROOT"
date -Is > "$ROOT/START"
cleanup() {
    rc=$?; trap - EXIT
    # Four app rows plus up to three boot failures; only our reserved range.
    for ((n=BASE;n<BASE+8;n++)); do
        test -e "$W/windows-broker-run$n.state" || continue
        bash "$B" stop "$n" || true
    done
    bash "$POST" || true
    echo "$rc" > "$ROOT/EXIT"
    echo "USER_WORKER_WINDOWS_EXIT=$rc at=$(date -Is) source=$REV (harness status; app verdict separate)"
}
trap cleanup EXIT
if [ ! -e "$W/kf3-bins/$SHORT" ]; then
    ln -s "$W/readtrace/kf3-bins/$SHORT" "$W/kf3-bins/$SHORT"
fi
test -x "$W/kf3-bins/$SHORT/qemu-system-x86_64"
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export KF_LOCKED=1
export WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_DROP=KF3_TIMER_MAP WIN_DISABLE_MPO=1
export PROD_EXTRA_FLAGS= WIN_FLAGS=
export WIN_PRE_GUEST_CMD="test \$(df -B1 --output=avail /var/lib | tail -1) -gt 107374182400 && bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash $POST"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
cd "$TREE"
echo 'HYPOTHESIS source-zero migration at the intercepted USER stream position preserves the original GravityMark Vulkan workload.'
echo 'FALSIFIER clean refusal, lost progress, wrong output, device loss, hostXid32 or guest141/4101.'
echo 'DEFAULT_ROUTE no relay enabling flag; unchanged production flags, original app media/oracles. Video/D3D12/CUDA smoke plus original600s Vulkan oracle.'
echo 'SUCCESS normal benchmark exit, original image/timings/GPU activity, zero recovery; same-GPU VFIO reference183s.'
start=$(date -u '+%Y-%m-%d %H:%M:%S UTC')
bash scripts/bench/windows/appmatrix/run_windows_apps.sh --run-dir "$ROOT/relay" \
    --kf3-rev "$SHORT" --broker "$B" --run-base "$BASE" \
    --override-dir "$W/codex-matrix-focus-tools-20261011" \
    --iso "$W/codex-supplement-20261011/full-image-a61af1c3/kfapps_supplement.img" \
    --apps edge_video_h264,gravitymark_d3d12,vectorAdd_demo,gravitymark_vk --tier 3 --per-guest 4 --no-isolate \
    > "$ROOT/relay-console.log" 2>&1
python3 /tmp/kf-capture-xids.py "$ROOT/relay" "$start"
