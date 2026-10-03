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
# DISPLAY_HOOK selects the hook (default hook.sh; unload_hook = box test B5)
export POST_CAPTURE_HOOK="$HERE/${DISPLAY_HOOK:-hook}.sh"
# ★ Leave the guest's filesystems clean before a poweroff the display teardown may wedge
# (coordinator 2026-09-30: an unclean shutdown leaves the image's journal dirty and the next fast-guest
# build cannot mount it): sync, then the kernel's emergency sync + remount read-only.
export PRE_POWEROFF_GUEST_CMD=${PRE_POWEROFF_GUEST_CMD:-"sudo sync; echo s | sudo tee /proc/sysrq-trigger >/dev/null; sleep 1; echo u | sudo tee /proc/sysrq-trigger >/dev/null; sleep 1"}
exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9
echo "DISPLAY_LANE_START tag=$TAG rev=$(git -C "$REPO" rev-parse --short=8 HEAD) $(date -Is)"
bash "$REPO/scripts/bench/boot_capture.sh" "$TAG" -- -vga none -vnc "${DISPLAY_VNC:-127.0.0.1:0}"
# ⊘ boot_capture's status, taken HERE: `[measured m1b, m1c]` it used to be read after the grep below,
# whose `-c` exits 1 when it counts ZERO errors — so a clean boot reported `DISPLAY_LANE_EXIT rc=1`.
rc=$?
# the whole guest dmesg at the end of the boot — every display error, not only the hook's slice
if [ -s "/workspace/bench/run_${TAG}_dmesg_after.log" ]; then
    echo "DISPLAY_GPU_PROGRESS_ERRORS_AFTER=$(grep -c 'waiting for GPU progress' "/workspace/bench/run_${TAG}_dmesg_after.log")"
    echo "DISPLAY_FLIP_EVENT_TIMEOUTS_AFTER=$(grep -c 'Flip event timeout' "/workspace/bench/run_${TAG}_dmesg_after.log")"
    echo "DISPLAY_DRM_WARNS_AFTER=$(grep -c 'cut here' "/workspace/bench/run_${TAG}_dmesg_after.log")"
fi
# ★ the HOST's view: a guest channel whose methods the host RM cannot service shows here as an Xid
# (`[measured m3c]` 186 x Xid 32 while the guest's display-SW object was offered)
echo "DISPLAY_HOST_XID=$(grep -c 'Xid' "/workspace/bench/run_${TAG}_hostdmesg.log" 2>/dev/null) $(grep -o 'Xid ([^)]*): [0-9]*' "/workspace/bench/run_${TAG}_hostdmesg.log" 2>/dev/null | awk '{print $NF}' | sort | uniq -c | tr '\n' ' ')"
grep -a '^DISPLAY_' "/workspace/bench/run_${TAG}_probe.log" 2>/dev/null
echo "DISPLAY_LANE_EXIT rc=$rc $(date -Is)"
