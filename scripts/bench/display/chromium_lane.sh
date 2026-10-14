#!/usr/bin/env bash
# chromium_lane.sh <kf3-rev> <tag> <guest-script> [guest-timeout-s] — boot a throw-away Linux desktop guest on kf3
# (same harness as desktop_smoke.sh / desktop_s3.sh: private lock + run dir, child overlay, pgrep wrapper), run
# <guest-script> in it over ssh (stdin; env CFG_* passed through), then grade the run:
#   guest: dmesg (Xid, NVRM, Flip event timeout, nvAssert, GSP timeout) saved to dmesg.txt
#   host:  kf3 log (qemu.log): unplanned GSP unloads ('GSP phase Running -> Suspending'), USER_RING_REFUSED,
#          USER_RING_SCAN, 'display: STALL', 'REFUSED fn76' (control ids), 'BLACK:'; host Xid delta (journalctl -k)
# The CALLER takes the host's one-guest lock around the whole call:
#   flock /tmp/claude-linux-guest.lock scripts/bench/display/chromium_lane.sh <rev> <tag> <script>
# base image: $CHROMIUM_BASE (default: the persistent chromium image made by tag "setup", else desktop.qcow2);
# tag "setup" writes INTO the base image (packages installed there persist), every other tag uses a child overlay.
# Start marker START, exit line EXIT in the run dir.
set -uo pipefail
REV=${1:?usage: chromium_lane.sh <kf3-rev> <tag> <guest-script> [timeout]}; TAG=${2:?tag}; GS=${3:?guest script}
GT=${4:-1500}
HERE="$(cd "$(dirname "$0")" && pwd)"
W=/var/lib/kf-windows-20261005
ROOT=$W/claude-chromium-$TAG
PERSIST=$W/claude-chromium-base/chromium.qcow2
if [ "$TAG" = setup ]; then
    mkdir -p "$W/claude-chromium-base"
    [ -e "$PERSIST" ] || qemu-img create -q -f qcow2 -F qcow2 -b "$W/broker-interactive/desktop.qcow2" "$PERSIST" || exit 2
    ROOT=$W/claude-chromium-setup-$(date +%H%M%S); IMG=$PERSIST
else
    [ -e "$ROOT" ] && { echo "CHROMIUM_FAIL $ROOT exists"; exit 2; }
    BASE=${CHROMIUM_BASE:-$PERSIST}; [ -e "$BASE" ] || BASE=$W/broker-interactive/desktop.qcow2
    IMG=$ROOT/desktop.qcow2
fi
mkdir -p "$ROOT"
echo "CHROMIUM_START tag=$TAG rev=$REV script=$GS $(date -Is)" | tee "$ROOT/START"
[ -x "/workspace/bench/kf3-bins/$REV/qemu-system-x86_64" ] || ln -sfn "$W/kf3-bins/$REV" "/workspace/bench/kf3-bins/$REV"
[ "$TAG" = setup ] || qemu-img create -q -f qcow2 -F qcow2 -b "$BASE" "$IMG" || exit 2
xid0=$(journalctl -k --no-pager 2>/dev/null | grep -c 'NVRM: Xid')
WRAP=/data/claude-planes-bin
env PATH="$WRAP:$PATH" KF_INTERACTIVE_DIR="$ROOT" KF_RUN_DIR="$ROOT/run" KF_IMG="$IMG" \
    KF3_REV="$REV" KF_LOCK="$ROOT/private.lock" KF_RAM_MB=${KF_RAM_MB:-6144} KF_FB_MB=${KF_FB_MB:-4096} \
    KF_SMP=${KF_SMP:-6} bash "$HERE/interactive.sh" > "$ROOT/launcher.log" 2>&1 < /dev/null &
launcher=$!
G=(ssh -i /workspace/bench/guest_key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
   -o LogLevel=ERROR -o ConnectTimeout=5 -o ServerAliveInterval=15 -o ServerAliveCountMax=8 ubuntu@192.168.77.2)
Q="$ROOT/run/qemu.log"
grade_host() {
    xid1=$(journalctl -k --no-pager 2>/dev/null | grep -c 'NVRM: Xid')
    echo "HOST_XID delta=$((xid1 - xid0))"
    echo "HOST_KF3 unplanned_gsp_unload=$(grep -ac 'GSP phase Running -> Suspending' "$Q") user_ring_refused=$(grep -ac 'USER_RING_REFUSED' "$Q") user_ring_scan=$(grep -ac 'USER_RING_SCAN' "$Q") display_stall=$(grep -ac 'display: STALL' "$Q") refused_fn76=$(grep -ac 'REFUSED fn76' "$Q") black=$(grep -ac 'BLACK:' "$Q")"
    grep -a 'REFUSED fn76' "$Q" | sed 's/.*REFUSED fn76/fn76/' | cut -c1-120 | sort | uniq -c | sort -rn | head -8 | sed 's/^/HOST_FN76 /'
}
finish() {
    rc=$?
    grade_host | tee "$ROOT/host-grade.txt"
    env PATH="$WRAP:$PATH" KF_INTERACTIVE_DIR="$ROOT" bash "$HERE/interactive.sh" stop > "$ROOT/stop.log" 2>&1 < /dev/null
    for _ in $(seq 40); do kill -0 "$launcher" 2>/dev/null || break; sleep 1; done
    kill -0 "$launcher" 2>/dev/null && kill "$launcher"
    journalctl -k --no-pager 2>/dev/null | grep 'NVRM' | tail -40 > "$ROOT/host-nvrm-tail.txt"
    echo "CHROMIUM_EXIT rc=$rc $(date -Is)" | tee "$ROOT/EXIT"
}
trap finish EXIT
ready=0
for _ in $(seq 1 100); do
    kill -0 "$launcher" 2>/dev/null || { echo "CHROMIUM_FAIL launcher exited"; tail -5 "$ROOT/launcher.log"; exit 3; }
    if timeout 8 "${G[@]}" true 2>/dev/null; then ready=1; break; fi
    sleep 3
done
[ "$ready" = 1 ] || { echo "CHROMIUM_FAIL guest ssh never came up"; exit 3; }
echo "CHROMIUM_GUEST_UP $(date -Is)"
# PUSH_DIR: its contents are unpacked into the guest's ~/kf first (pages, driver, and for "setup" the downloads)
if [ -n "${PUSH_DIR:-}" ]; then
    tar -C "$PUSH_DIR" -cf - . | "${G[@]}" 'mkdir -p ~/kf && tar -C ~/kf -xf -' && echo "CHROMIUM_PUSHED $PUSH_DIR"
fi
# CFG_* variables of this environment go to the guest script
CFG=$(env | grep '^CFG_' | sed "s/'/'\\\\''/g; s/^\([^=]*\)=\(.*\)$/export \1='\2'/")
{ echo "$CFG"; cat "$GS"; } | timeout "$GT" "${G[@]}" 'bash -s' > "$ROOT/guest.log" 2>&1
echo "CHROMIUM_GUEST_SCRIPT_RC=$? $(date -Is)"
tail -40 "$ROOT/guest.log" | cut -c1-240 | sed 's/^/G /'
mkdir -p "$ROOT/guest-out"; timeout 60 "${G[@]}" 'tar -C ~/out -cf - . 2>/dev/null' | tar -C "$ROOT/guest-out" -xf - 2>/dev/null
timeout 40 "${G[@]}" 'sudo dmesg' > "$ROOT/dmesg.txt" 2>&1
echo "GUEST_DMESG xid=$(grep -c 'Xid' "$ROOT/dmesg.txt") nvrm=$(grep -c 'NVRM' "$ROOT/dmesg.txt") flip_timeout=$(grep -c 'Flip event timeout' "$ROOT/dmesg.txt") nvassert=$(grep -ci 'nvAssert' "$ROOT/dmesg.txt") gsp_timeout=$(grep -ci 'GSP.*timeout\|timeout.*GSP' "$ROOT/dmesg.txt") dmesg_lines=$(wc -l < "$ROOT/dmesg.txt")" | tee "$ROOT/guest-grade.txt"
grep -E 'Xid|NVRM|Flip event|nvAssert|timeout' "$ROOT/dmesg.txt" | tail -15 | cut -c1-200 | sed 's/^/GD /'
exit 0
