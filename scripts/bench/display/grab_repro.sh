#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# grab_repro.sh <tag> — mouse-look under the broker's grab, without the game (V3_DISPLAY.md §8.19).
# Boots the interactive.sh guest with the REAL broker's test backend (input scripted on a FIFO),
# builds warpgrab.c in the guest (GLFW's disabled-cursor mode: grab, blank cursor, warp to centre,
# deltas from core motion and from XI2 RawMotion), and for each case runs it while CTRL+ALT+G is
# held and REL packets are injected; evtest records both virtio pointers. One GRAB_* line per case.
#   A  REL only                      B  REL, a click and a wheel tick in the middle, REL
#   C  as B with the tablet disabled in X (xinput disable)   D  REL, an ABS packet, REL
# env: KF3_REV (the kf3 binary), CASES (default "A B C D").
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
TAG=${1:?usage: grab_repro.sh <tag>}
BENCH=${BENCH_DIR:-/workspace/bench}
WORK=${KF_INTERACTIVE_DIR:-/var/lib/kf-windows-20261005/broker-interactive}
OUT=$WORK/grab-$TAG; rm -rf "$OUT"; mkdir -p "$OUT"; chmod 0711 "$OUT"
LOG=$OUT/grab.log
P(){ echo "GRAB_$*" | tee -a "$LOG"; }
G=(ssh -i "$BENCH/guest_key" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 -o ServerAliveInterval=5 ubuntu@192.168.77.2)
gq(){ timeout "${2:-60}" "${G[@]}" "$1" 2>&1 | tr -d '\r'; }
GX='sudo -u ubuntu env DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority'
P "START tag=$TAG checkout=$(git -C "$HERE" rev-parse --short=8 HEAD) $(date -Is)"
SU=$(loginctl show-session "$(loginctl show-seat seat0 -p ActiveSession --value)" -p Name --value)
FIFO=$OUT/broker.in; mkfifo -m 0600 "$FIFO"; chown "$SU" "$FIFO"; exec 7<>"$FIFO"
send(){ for l in "$@"; do printf '%s\n' "$l" >&7; done; }
chord(){ send "k 29 1" "k 56 1" "k 34 1" "k 34 0" "k 56 0" "k 29 0"; }
RUN=$OUT/vm
KF_BROKER_BACKEND=test KF_BROKER_FIFO=$FIFO KF_RUN_DIR=$RUN "$HERE/interactive.sh" run > "$RUN.out" 2>&1 &
VMPID=$!
for _ in $(seq 60); do grep -aq 'accepted uid' "$RUN/broker.log" 2>/dev/null && break; sleep 0.5; done
send "f 1" "p 1"
for _ in $(seq 100); do gq true 8 >/dev/null 2>&1 && break; sleep 3; done
for _ in $(seq 60); do gq "$GX xset q >/dev/null 2>&1 && pgrep -u ubuntu -x cinnamon >/dev/null && echo UP" 10 | grep -q UP && break; sleep 3; done
P "DESKTOP $(gq 'pgrep -u ubuntu -x cinnamon >/dev/null && echo cinnamon_up')"
# the client, built in the guest
gq 'command -v cc >/dev/null && test -e /usr/include/X11/extensions/XInput2.h || (sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q gcc libc6-dev libx11-dev libxi-dev >/tmp/apt.log 2>&1; echo apt=$?)' 600
"${G[@]}" 'cat > /tmp/warpgrab.c' < "$HERE/warpgrab.c"
P "BUILD $(gq 'cc -O2 -o /tmp/warpgrab /tmp/warpgrab.c -lX11 -lXi 2>&1 && echo ok')"
gq "$GX xinput set-prop 'QEMU Virtio Mouse' 'libinput Accel Profile Enabled' 0 1 0; $GX xinput set-prop 'QEMU Virtio Mouse' 'libinput Accel Speed' 0" >/dev/null
gq "$GX xinput list" > "$OUT/xinput_list.txt"
evstart(){ gq 'for n in Tablet Mouse; do ev=$(grep -A5 "Name=\"QEMU Virtio $n\"" /proc/bus/input/devices | grep -o "event[0-9]*" | head -1); sudo pkill -f "evtest /dev/input/$ev"; (sudo stdbuf -oL evtest /dev/input/$ev < /dev/null > /tmp/ev_$n.log 2>&1 &); done'; }
for c in ${CASES:-A B C D}; do
    [ "$c" = C ] && gq "$GX xinput disable 'QEMU Virtio Tablet'; echo disabled" >/dev/null
    evstart
    # the pointer somewhere far from the client's centre first: the tablet's last report
    send "a 1700 900"; sleep 0.5
    gq "$GX /tmp/warpgrab 7 /tmp/wg_$c.log > /tmp/wg_$c.out 2>&1 &" >/dev/null
    sleep 1.5
    chord; sleep 1
    rel(){ for _ in $(seq "$1"); do send "r 10 0"; sleep 0.03; done; }
    case $c in
        A) rel 20 ;;
        B|C) rel 10; send "b 272 1" "b 272 0"; sleep 0.2; send "w 1 0"; sleep 0.2; rel 10 ;;
        D) rel 10; send "a 1700 900"; sleep 0.2; rel 10 ;;
    esac
    sleep 0.5; chord; sleep 4.5
    res=$(gq "cat /tmp/wg_$c.out")
    for n in Tablet Mouse; do gq "sudo cat /tmp/ev_$n.log" > "$OUT/ev_${c}_$n.log"; done
    gq "cat /tmp/wg_$c.log" > "$OUT/wg_$c.log"
    P "CASE $c $res tablet_events=$(grep -ac 'type [13] (EV_\(ABS\|KEY\)), code' "$OUT/ev_${c}_Tablet.log") tablet_btn=$(grep -ac 'type 1 (EV_KEY), code' "$OUT/ev_${c}_Tablet.log") mouse_rel_x=$(awk '/type 2 \(EV_REL\), code 0 \(REL_X\)/{s+=$NF} END{print s+0}' "$OUT/ev_${c}_Mouse.log") mouse_btn=$(grep -ac 'type 1 (EV_KEY), code' "$OUT/ev_${c}_Mouse.log") mouse_wheel=$(grep -ac 'REL_WHEEL)' "$OUT/ev_${c}_Mouse.log") max_core_jump=$(awk '/MOTION/{split($6,d,","); x=d[1]<0?-d[1]:d[1]; if(x>m)m=x} END{print m+0}' "$OUT/wg_$c.log")"
    [ "$c" = C ] && gq "$GX xinput enable 'QEMU Virtio Tablet'" >/dev/null
done
grep -a 'kf3: broker: \(grab\|pointing\|input while grabbed\)' "$RUN/qemu.log" | sed 's/^[^ ]* //' | tail -12 > "$OUT/relay_grab_lines.txt"
[ "${KEEP:-0}" = 1 ] || { "$HERE/interactive.sh" stop >/dev/null 2>&1; wait "$VMPID" 2>/dev/null; }
exec 7>&-
P "EXIT $(date -Is)"
