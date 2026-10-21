#!/usr/bin/env bash
# desktop_smoke.sh <kf3-rev> <tag> — the Linux desktop + display-broker retest (V3_DISPLAY.md §8.17,
# `feedback`: retest the Linux guest with the broker after display changes), runnable BESIDE another VM:
# interactive.sh on a throw-away child of the provisioned desktop image, a private lock and run dir,
# KF_FB_MB (default 4096) so the store fits next to the other VM's, and a pgrep wrapper that lets
# interactive.sh's "no QEMU may run" precondition pass only when no running QEMU uses this image chain.
# Graded in the guest: Cinnamon + Xorg up, glxinfo renderer NVIDIA, glxgears frames for 20 s, vkcube
# 10 s when installed, guest dmesg (Flip event timeouts, nvAssert, Xid); on the host: kf3 display
# STALL/Exception lines, the broker's frames, host Xid delta. Start marker and DESKTOP_SMOKE_EXIT line.
set -uo pipefail
REV=${1:?usage: desktop_smoke.sh <kf3-rev> <tag>}; TAG=${2:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"
W=/var/lib/kf-windows-20261005
ROOT=$W/claude-desktop-$TAG
[ -e "$ROOT" ] && { echo "DESKTOP_SMOKE_FAIL $ROOT exists"; exit 2; }
mkdir -p "$ROOT"
START=$(date -Is); echo "DESKTOP_SMOKE_START tag=$TAG rev=$REV $START" | tee "$ROOT/START"
[ -x "/workspace/bench/kf3-bins/$REV/qemu-system-x86_64" ] || ln -sfn "$W/kf3-bins/$REV" "/workspace/bench/kf3-bins/$REV"
qemu-img create -q -f qcow2 -F qcow2 -b "$W/broker-interactive/desktop.qcow2" "$ROOT/desktop.qcow2" || exit 2
xid0=$(journalctl -k --no-pager 2>/dev/null | grep -c 'NVRM: Xid')
WRAP=${PGREP_WRAPPER_DIR:-/data/claude-planes-bin}
env PATH="$WRAP:$PATH" KF_INTERACTIVE_DIR="$ROOT" KF_RUN_DIR="$ROOT/run" KF_IMG="$ROOT/desktop.qcow2" \
    KF3_REV="$REV" KF_LOCK="$ROOT/private.lock" KF_RAM_MB=${KF_RAM_MB:-4096} KF_FB_MB=${KF_FB_MB:-4096} \
    bash "$HERE/interactive.sh" > "$ROOT/launcher.log" 2>&1 < /dev/null &
launcher=$!
G=(ssh -i /workspace/bench/guest_key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
   -o LogLevel=ERROR -o ConnectTimeout=5 ubuntu@192.168.77.2)
finish() {
    rc=$?
    env PATH="$WRAP:$PATH" KF_INTERACTIVE_DIR="$ROOT" bash "$HERE/interactive.sh" stop > "$ROOT/stop.log" 2>&1 < /dev/null
    for _ in $(seq 30); do kill -0 "$launcher" 2>/dev/null || break; sleep 1; done
    kill -0 "$launcher" 2>/dev/null && kill "$launcher"
    xid1=$(journalctl -k --no-pager 2>/dev/null | grep -c 'NVRM: Xid')
    echo "DESKTOP_HOST_XID delta=$((xid1 - xid0))"
    Q="$ROOT/run/qemu.log"
    echo "DESKTOP_KF3 stall=$(grep -ac 'display: STALL' "$Q" 2>/dev/null) exception=$(grep -ac 'Exception' "$Q" 2>/dev/null) scanout_refused=$(grep -ac 'scanout REFUSED' "$Q" 2>/dev/null) cursor_refused=$(grep -ac 'cursor REFUSED' "$Q" 2>/dev/null) broker_lines=$(grep -ac 'kf3: broker' "$Q" 2>/dev/null)"
    grep -a 'kf3: broker' "$Q" 2>/dev/null | tail -3 | cut -c1-200 | sed 's/^/DESKTOP_BROKER /'
    echo "DESKTOP_SMOKE_EXIT rc=$rc $(date -Is)" | tee "$ROOT/EXIT"
}
trap finish EXIT
ready=0
for _ in $(seq 1 100); do
    kill -0 "$launcher" 2>/dev/null || { echo "DESKTOP_SMOKE_FAIL launcher exited"; tail -5 "$ROOT/launcher.log"; exit 3; }
    if timeout 8 "${G[@]}" true 2>/dev/null; then ready=1; break; fi
    sleep 3
done
[ "$ready" = 1 ] || { echo "DESKTOP_SMOKE_FAIL guest ssh never came up"; exit 3; }
timeout 150 "${G[@]}" 'bash -s' > "$ROOT/guest-smoke.log" 2>&1 <<'GUEST'
set -u
export DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority
for _ in $(seq 1 40); do pgrep -x cinnamon >/dev/null && break; sleep 2; done
echo "CINNAMON $(pgrep -c -x cinnamon) XORG $(pgrep -c -x Xorg)"
glxinfo -B 2>&1 | grep -E 'OpenGL renderer|OpenGL vendor' | sed 's/^/GLX /'
timeout 20 stdbuf -oL glxgears > /tmp/kf-gears.log 2>&1
sed 's/^/GEARS /' /tmp/kf-gears.log | tail -4
if command -v vkcube >/dev/null; then
    timeout 10 vkcube > /tmp/kf-vkcube.log 2>&1; echo "VKCUBE rc=$? $(tail -1 /tmp/kf-vkcube.log | cut -c1-120)"
else
    echo "VKCUBE not installed"
fi
echo "GUEST_DMESG flip_timeouts=$(sudo dmesg | grep -c 'Flip event timeout') nvassert=$(sudo dmesg | grep -ci 'nvAssert') xid=$(sudo dmesg | grep -c 'Xid')"
echo GUEST_DONE
GUEST
sed 's/^/DESKTOP_/' "$ROOT/guest-smoke.log"
grep -q '^GUEST_DONE' "$ROOT/guest-smoke.log" || exit 4
grep -q 'GEARS .*frames in' "$ROOT/guest-smoke.log" || exit 5
grep -qi 'GLX OpenGL renderer.*NVIDIA' "$ROOT/guest-smoke.log" || exit 6
exit 0
