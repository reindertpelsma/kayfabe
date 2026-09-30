#!/usr/bin/env bash
# lane.sh <tag> — the display lane, one boot (docs/design/V3_DISPLAY.md §5).
#   Boots the fat guest on the kf3 device built from THIS checkout with the display plane ON
#   (`display=on`), no emulated VGA (the kf3 head is the only graphic console), a localhost-only
#   VNC server, and runs hook.sh while the guest is up. Every claim cites the binary's revision
#   (boot_capture.sh's `kf3-bin-rev`).
# env: DISPLAY_VNC (default 127.0.0.1:0 → port 5900), NVKVM_RAM_MB (default 8192), KF_SMP (6),
#      DISPLAY_KF3_EXTRA (appended to the device line), DISPLAY_HOLD_S / DISPLAY_FLIPS (hook.sh)
# output: /workspace/bench/run_<tag>_{probe,dmesg,qemu,hostdmesg}.log (boot_capture.sh) and
#         /workspace/bench/display/<tag>/ (hook.sh); the verdict lines are DISPLAY_* in the probe log.
set -uo pipefail
TAG=${1:?usage: lane.sh <tag>}
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../../.." && pwd)"
export NVKVM_RAM_MB=${NVKVM_RAM_MB:-8192} KF_SMP=${KF_SMP:-6}
export KF3_DEV_EXTRA="display=on${DISPLAY_KF3_EXTRA:+,$DISPLAY_KF3_EXTRA}"
export POST_CAPTURE_HOOK="$HERE/hook.sh"
exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9
echo "DISPLAY_LANE_START tag=$TAG rev=$(git -C "$REPO" rev-parse --short=8 HEAD) $(date -Is)"
bash "$REPO/scripts/bench/boot_capture.sh" "$TAG" -- -vga none -vnc "${DISPLAY_VNC:-127.0.0.1:0}"
# the whole guest dmesg at the end of the boot — every display error, not only the hook's slice
[ -s "/workspace/bench/run_${TAG}_dmesg_after.log" ] && grep -c 'waiting for GPU progress' "/workspace/bench/run_${TAG}_dmesg_after.log" | sed 's/^/DISPLAY_GPU_PROGRESS_ERRORS_AFTER=/'
rc=$?
grep -a '^DISPLAY_' "/workspace/bench/run_${TAG}_probe.log" 2>/dev/null
echo "DISPLAY_LANE_EXIT rc=$rc $(date -Is)"
