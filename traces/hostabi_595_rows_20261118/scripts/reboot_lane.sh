#!/bin/bash
# reboot_lane.sh <label> <srcdir (a sweep worktree at the revision)> <guest-driver> [guestimg]
# one cold boot with the hook; never retried.
LABEL=$1; SRC=$2; GD=$3; IMG=${4:-}
export BENCH_DIR=/workspace/bench KF_DEVICE=kf3 PATH=$HOME/.cargo/bin:$PATH
cd "$SRC" || exit 2
REV=$(git rev-parse --short=8 HEAD); T=rb_${LABEL}_r${REV}
exec 9>/tmp/kayfabe-fastguest.lock; flock 9
for _w in $(seq 1 60); do u=$(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>/dev/null | head -1 | tr -dc 0-9); [ -n "$u" ] && [ "$u" -le 512 ] && break; sleep 0.5; done
echo "RBLANE_START $(date -Is) label=$LABEL rev=$REV host=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader) guest_driver=$GD img=${IMG:-bench image}"
env KF_REPO="$SRC" KF3_DEV_EXTRA="guest-driver=$GD" ${IMG:+KF_GUEST_IMG=$IMG} KF_DEVICE=kf3 POST_CAPTURE_HOOK=/root/reboot_hook.sh GQ_TIMEOUT=600 \
  timeout 1500 bash scripts/bench/boot_capture.sh "$T" > /workspace/bench/${T}_driver.log 2>&1
echo "RBLANE_EXIT rc=$? tag=$T $(date -Is)"
sed -n '/=== POST_CAPTURE_HOOK/,/^HOOK_RC/p' /workspace/bench/run_${T}_probe.log > /workspace/bench/rbres_${T}.txt
