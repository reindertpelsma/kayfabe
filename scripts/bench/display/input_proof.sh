#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# input_proof.sh <tag> — does broker input reach the guest? (docs/design/V3_DISPLAY.md §8.17)
#
# Boots the interactive.sh guest with the REAL broker (nvkvm-pv, /opt/nvkvm-broker) on its
# display-less `test` backend, whose input is scripted line by line (nb_session_test.c: k/b/a/r/w/f/p)
# and goes through the broker's own input core (focus gating, the CTRL+ALT+G hotkey, grab rules)
# onto the wire, into kf3's relay and QEMU's input layer. The guest records what arrives with evtest
# on each virtio input device and xdotool in its X session. Then, optionally, the measurement
# steps (PROOF_UNLOAD=1: nvidia unload; PROOF_REBOOTS=N: N clean reboots in the same QEMU;
# PROOF_REATTACH=1: QEMU restarted against the still-running broker).
#
# Every grade is one PROOF_* line in $OUT/proof.log with what was injected and what arrived.
# Falsifiers (stated before the run, §8.17): ABS — the guest pointer more than 2 px from
# x*Sw/W, y*Sh/H at any of the positions, for either window size; REL — the evdev REL sum on the
# QEMU Virtio Mouse differs from the injected sum, or the X pointer does not move by it under the
# flat profile; KEY — any injected edge missing on the QEMU Virtio Keyboard, or an extra one; grub —
# no editor text on the serial console after `e`.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
TAG=${1:?usage: input_proof.sh <tag>}
BENCH=${BENCH_DIR:-/workspace/bench}
WORK=${KF_INTERACTIVE_DIR:-/var/lib/kf-windows-20261005/broker-interactive}
OUT=$WORK/proof-$TAG; rm -rf "$OUT"; mkdir -p "$OUT"
LOG=$OUT/proof.log
P(){ echo "PROOF_$*" | tee -a "$LOG"; }
G=(ssh -i "$BENCH/guest_key" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 -o ServerAliveInterval=5 ubuntu@192.168.77.2)
gq(){ timeout "${2:-60}" "${G[@]}" "$1" 2>&1 | tr -d '\r'; }
GX='sudo -u ubuntu env DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority'
FIFO=$OUT/broker.in
P "START tag=$TAG checkout=$(git -C "$HERE" rev-parse --short=8 HEAD) broker=$(cat /opt/nvkvm-broker/REV) $(date -Is)"

# the session user owns the FIFO (the broker runs as that user); fd 7 keeps a writer open so the
# test backend never sees EOF between lines
SU=$(loginctl show-session "$(loginctl show-seat seat0 -p ActiveSession --value)" -p Name --value)
mkfifo -m 0600 "$FIFO"; chown "$SU" "$FIFO"; chmod 0711 "$OUT"
exec 7<>"$FIFO"
send(){ for l in "$@"; do printf '%s\n' "$l" >&7; done; }
key(){ send "k $1 1" "k $1 0"; }

start_vm(){   # $1 = window size for the test backend, $2 = run dir
    KF_BROKER_BACKEND=test KF_BROKER_FIFO=$FIFO BROKER_ARGS="--size $1" KF_RUN_DIR=$2 \
        KF_VNC=0 "$HERE/interactive.sh" run > "$2.out" 2>&1 &
    VMPID=$!
}
mon(){ printf '%s\n' "$1" | timeout 10 socat - "UNIX-CONNECT:$RUN/qemu.mon" >/dev/null 2>&1; }
shot(){ mon "screendump $OUT/$1.ppm kf0"; sleep 1.5; [ -s "$OUT/$1.ppm" ] && convert "$OUT/$1.ppm" "$OUT/$1.png" 2>/dev/null && rm -f "$OUT/$1.ppm"; }
qpid(){ pgrep -f "^[^ ]*qemu-system-x86_64 -name kayfabe-interactive" | head -1; }
wait_ssh(){ for _ in $(seq "${1:-100}"); do gq true 8 >/dev/null 2>&1 && return 0; [ -n "$(qpid)" ] || return 2; sleep 3; done; return 1; }

RUN=$OUT/vm1; start_vm 1280x720 "$RUN"; T0=$(date +%s)
for _ in $(seq 60); do grep -aq 'accepted uid' "$RUN/broker.log" 2>/dev/null && break; sleep 0.5; done
P "BROKER_CONNECTED +$(( $(date +%s) - T0 ))s $(grep -a -m1 'accepted uid' "$RUN/broker.log" | cut -c1-160)"
# the boot display before nvidia, as the console's framebuffer readback: one shot a second for 25 s
# — only from the broker's connection on: ⊘ [measured, run proof-p3] a console readback in the first
# ~2 s after QEMU starts stalls kf3's first scanout copy for good (V3_DISPLAY.md §8.17)
( for i in $(seq -w 1 25); do shot "boot_t$i"; sleep 1; done ) &
SHOTS=$!
send "f 1" "p 1"
# grub (GRUB_TERMINAL=console also writes its menu to the serial port): a DOWN key each second from
# the broker's connection until the menu is on the serial port — the first one grub reads stops its
# 10 s countdown — then 'e' opens the entry editor, whose help text is the evidence
for _ in $(seq 25); do grep -aq 'GNU GRUB' "$RUN/serial.log" 2>/dev/null && break; key 108; sleep 1; done
P "GRUB_MENU +$(( $(date +%s) - T0 ))s serial=$(grep -ac 'GNU GRUB' "$RUN/serial.log") countdown=[$(grep -ao 'automatically in [0-9]*s' "$RUN/serial.log" | tr '\n' ' ')]"
sleep 1; shot grub_menu
key 18
for _ in $(seq 10); do grep -aq 'Minimum Emacs-like screen editing' "$RUN/serial.log" && break; sleep 1; done
shot grub_editor
ed=$(grep -ac 'Minimum Emacs-like screen editing' "$RUN/serial.log")
P "GRUB_KEY $([ "$ed" -gt 0 ] && echo PASS || echo FAIL) broker e -> editor_on_serial=$ed countdown_left=$(grep -ao 'automatically in [0-9]s' "$RUN/serial.log" | tail -1)"
if [ "$ed" -gt 0 ]; then
    key 1; sleep 1; key 102; sleep 0.5; key 28   # Escape to the menu, Home (the first entry), Enter
else
    # the control: the same key through QEMU's monitor (PS/2) — tells "grub takes no key" from
    # "the broker's key never reached the PS/2 keyboard"
    mon "sendkey e"; sleep 2
    P "GRUB_MONITOR_KEY sendkey e -> editor_on_serial=$(grep -ac 'Minimum Emacs-like screen editing' "$RUN/serial.log")"
    mon "sendkey esc"; sleep 1; mon "sendkey home"; mon "sendkey ret"
fi
wait "$SHOTS" 2>/dev/null
wait_ssh 100 || { P "FAIL guest never answered ssh"; }
for _ in $(seq 60); do gq "$GX xset q >/dev/null 2>&1 && pgrep -u ubuntu -x cinnamon >/dev/null && echo UP" 10 | grep -q UP && break; sleep 3; done
P "DESKTOP $(gq "pgrep -u ubuntu -x cinnamon >/dev/null && echo cinnamon_up; $GX xdpyinfo | grep -m1 dimensions")"
sleep 5; shot desktop
gq 'cat /proc/bus/input/devices' > "$OUT/input_devices.txt"
gq "nvidia-smi --query-gpu=name,driver_version --format=csv,noheader; uname -r" > "$OUT/guest_versions.txt"
# evtest on each virtio input device, in the background, for the whole input test
gq 'for n in Keyboard Tablet Mouse; do ev=$(grep -A5 "Name=\"QEMU Virtio $n\"" /proc/bus/input/devices | grep -o "event[0-9]*" | head -1); echo "$n=$ev"; sudo pkill -f "evtest /dev/input/$ev"; (sudo stdbuf -oL evtest /dev/input/$ev < /dev/null > /tmp/ev_$n.log 2>&1 &); done' > "$OUT/evtest_devices.txt"
# the flat acceleration profile makes the X pointer move exactly by the relative deltas
gq "$GX xinput set-prop 'QEMU Virtio Mouse' 'libinput Accel Profile Enabled' 0 1 0; $GX xinput set-prop 'QEMU Virtio Mouse' 'libinput Accel Speed' 0; $GX xinput list-props 'QEMU Virtio Mouse' | grep -i accel" > "$OUT/accel.txt"
sleep 2
ptr(){ gq "$GX xdotool getmouselocation" | sed -n 's/^x:\([0-9]*\) y:\([0-9]*\).*/\1 \2/p'; }
read -r SW SH < <(gq "$GX xdpyinfo" | sed -n 's/.*dimensions: *\([0-9]*\)x\([0-9]*\) pixels.*/\1 \2/p')
# ⊘ [run p4, 2026-10-08] the test backend's window takes the guest's size from the relay's WINDOW
# (1920x1080 there), so `--size` is only its size before the first frame and the ABS range is the
# guest's frame: the evtest values were x*32767/1920 exactly. The checks pass that range.
abs_check(){   # $1 $2 = window position, $3 $4 = the ABS range (the broker window = the guest frame)
    send "a $1 $2"; sleep 1
    local want_x=$(( $1 * SW / $3 )) want_y=$(( $2 * SH / $4 )) got gx gy v=PASS
    got=$(ptr); read -r gx gy <<< "$got"
    { [ -z "$gx" ] || [ $(( gx > want_x ? gx - want_x : want_x - gx )) -gt 2 ] || [ $(( gy > want_y ? gy - want_y : want_y - gy )) -gt 2 ]; } && v=FAIL
    P "ABS $v window=$3x$4 inject=$1,$2 want=$want_x,$want_y got=${gx:-?},${gy:-?} screen=${SW}x$SH"
}
abs_check 100 100 "$SW" "$SH"
abs_check 1000 600 "$SW" "$SH"
abs_check 640 360 "$SW" "$SH"
abs_check "$((SW - 1))" "$((SH - 1))" "$SW" "$SH"
# keys: shift+a, ctrl+l, and plain a
m0=$(grep -ac 'grab' "$RUN/broker.log")
send "k 42 1" "k 30 1" "k 30 0" "k 42 0" "k 29 1" "k 38 1" "k 38 0" "k 29 0" "k 30 1" "k 30 0"
sleep 1
# buttons (left click on the desktop background at the last position) and a wheel detent
send "a 640 300"; sleep 0.5
send "b 272 1" "b 272 0" "w 1 0" "w -1 0"; sleep 1
# GRAB by the broker's own hotkey chord, CTRL+ALT+G (consumed by the broker, never forwarded)
send "k 29 1" "k 56 1" "k 34 1" "k 34 0" "k 56 0" "k 29 0"; sleep 1.5
P "GRAB_ON broker=[$(tail -n +"$((m0 + 1))" "$RUN/broker.log" | grep -a 'grab' | tr '\n' '|' | cut -c1-160)] kf3=[$(grep -a 'pointing device ->' "$RUN/qemu.log" | tail -1 | sed 's/.*kf3: broker: //')]"
read -r bx by <<< "$(ptr)"
# relative: 10x(10,0), 5x(0,10), (-30,-20) => (70,30); then an ABS while grabbed, which the broker must drop
for _ in $(seq 10); do send "r 10 0"; sleep 0.05; done
for _ in $(seq 5); do send "r 0 10"; sleep 0.05; done
send "r -30 -20"; sleep 1
read -r ax ay <<< "$(ptr)"
P "REL_X11 $([ "$((ax - bx))" = 70 ] && [ "$((ay - by))" = 30 ] && echo PASS || echo FAIL) inject_sum=70,30 pointer=$bx,$by->$ax,$ay delta=$((ax - bx)),$((ay - by)) (flat profile)"
send "a 10 10"; sleep 1
read -r cx cy <<< "$(ptr)"
P "ABS_UNDER_GRAB $([ "$cx,$cy" = "$ax,$ay" ] && echo PASS_DROPPED || echo FAIL_MOVED) pointer=$ax,$ay->$cx,$cy"
send "b 273 1" "b 273 0"; sleep 0.5; key 1    # a right click under grab (closes its menu with Escape)
send "k 29 1" "k 56 1" "k 34 1" "k 34 0" "k 56 0" "k 29 0"; sleep 1.5
P "GRAB_OFF kf3=[$(grep -a 'pointing device ->' "$RUN/qemu.log" | tail -1 | sed 's/.*kf3: broker: //')]"
abs_check 200 150 "$SW" "$SH"
# the broker restarted (QEMU reconnects and re-sends its geometry and last frame)
# ⊘ [run p4] the socket path was read AFTER the kill had removed it: the restart got an empty
# --socket and this step never ran. The path is read first now.
sock=$(ls /run/user/*/nvkvm/display.sock 2>/dev/null | head -1)
pkill -f "^[^ ]*nvkvm-display-broker --socket /run/user/[0-9]*/nvkvm/display.sock" ; sleep 1
rm -f "$sock"
runuser -u "$SU" -- /opt/nvkvm-broker/nvkvm-display-broker --socket "$sock" --backend test --persist --verbose --size 800x600 < "$FIFO" > "$RUN/broker2.log" 2>&1 &
for _ in $(seq 40); do grep -aq 'accepted uid' "$RUN/broker2.log" 2>/dev/null && break; sleep 0.5; done
send "f 1" "p 1"; sleep 1
P "BROKER2 $(grep -a -m1 'accepted uid' "$RUN/broker2.log" | cut -c1-120) relay=[$(grep -a 'kf3: broker: \(connected\|re-sent\|the display broker closed\)' "$RUN/qemu.log" | tail -2 | sed 's/.*kf3: broker: //' | tr '\n' '|' | cut -c1-200)]"
abs_check 400 300 "$SW" "$SH"
abs_check 1500 900 "$SW" "$SH"
abs_check 0 0 "$SW" "$SH"
sleep 1
for n in Keyboard Tablet Mouse; do gq "sudo cat /tmp/ev_$n.log" > "$OUT/ev_$n.log"; done
# evdev sums and edges
P "EVDEV_MOUSE rel_x_sum=$(awk '/type 2 \(EV_REL\), code 0 \(REL_X\)/{s+=$NF} END{print s+0}' "$OUT/ev_Mouse.log") rel_y_sum=$(awk '/type 2 \(EV_REL\), code 1 \(REL_Y\)/{s+=$NF} END{print s+0}' "$OUT/ev_Mouse.log") btn=[$(grep -a 'type 1 (EV_KEY), code' "$OUT/ev_Mouse.log" | sed 's/.*code [0-9]* (\(.*\)), value \(.\)/\1=\2/' | tr '\n' ' ')] wheel=[$(grep -ac 'code 8 (REL_WHEEL)' "$OUT/ev_Mouse.log")]"
P "EVDEV_TABLET abs_events=$(grep -ac 'type 3 (EV_ABS), code' "$OUT/ev_Tablet.log") btn=[$(grep -a 'type 1 (EV_KEY), code' "$OUT/ev_Tablet.log" | sed 's/.*code [0-9]* (\(.*\)), value \(.\)/\1=\2/' | tr '\n' ' ')] wheel=$(grep -ac 'code 8 (REL_WHEEL)' "$OUT/ev_Tablet.log")"
P "EVDEV_KEYBOARD edges=[$(grep -a 'type 1 (EV_KEY), code' "$OUT/ev_Keyboard.log" | sed 's/.*code [0-9]* (\(.*\)), value \(.\)/\1=\2/' | tr '\n' ' ')]"
want_keys='KEY_LEFTSHIFT=1 KEY_A=1 KEY_A=0 KEY_LEFTSHIFT=0 KEY_LEFTCTRL=1 KEY_L=1 KEY_L=0 KEY_LEFTCTRL=0 KEY_A=1 KEY_A=0 '
got_keys=$(grep -a 'type 1 (EV_KEY), code' "$OUT/ev_Keyboard.log" | sed 's/.*code [0-9]* (\(.*\)), value \(.\)/\1=\2/' | tr '\n' ' ' | sed 's/KEY_ESC=1 KEY_ESC=0 //g; s/KEY_LEFTCTRL=1 KEY_LEFTALT=1 KEY_LEFTALT=0 KEY_LEFTCTRL=0 //g')
P "KEYS $([ "${got_keys:0:${#want_keys}}" = "$want_keys" ] && echo PASS || echo FAIL) want=[$want_keys] got=[$got_keys]"
grep -a 'kf3: broker:' "$RUN/qemu.log" | sed 's/^[^ ]* //' | cut -c1-220 > "$OUT/relay_lines.txt"
grep -ao 'broker\[[^]]*\]' "$RUN/qemu.log" | tail -1 | sed 's/^/PROOF_STATUS /' | tee -a "$LOG"

if [ "${PROOF_UNLOAD:-0}" = 1 ]; then
    gq 'sudo systemctl stop lightdm; sleep 3; sudo pkill -9 Xorg; sleep 2; lsmod | grep -q ^nvidia_uvm && sudo modprobe -r nvidia_uvm; sudo modprobe -r nvidia_drm nvidia_modeset nvidia; echo rc=$?; lsmod | grep -c nvidia' 90 > "$OUT/unload.txt"
    sleep 5; shot after_unload_5s; sleep 10; shot after_unload_15s
    P "UNLOAD $(tr '\n' ' ' < "$OUT/unload.txt") relay=[$(grep -a 'kf3: \(display\|broker\):' "$RUN/qemu.log" | tail -4 | sed 's/.*kf3: //' | tr '\n' '|' | cut -c1-400)]"
fi
for i in $(seq 1 "${PROOF_REBOOTS:-0}"); do
    q=$(qpid); sl=$(wc -l < "$RUN/serial.log")
    gq 'sudo systemctl reboot' 20 >/dev/null
    sleep 20
    r=down; wait_ssh 80 && r=ssh_up
    sleep 30
    shot reboot${i}_after
    gq 'sudo dmesg' > "$OUT/reboot${i}_dmesg.log"
    P "REBOOT$i qemu_pid=$q alive=$([ -n "$(qpid)" ] && echo yes || echo NO) guest=$r login_prompt=$(tail -n +"$sl" "$RUN/serial.log" | grep -ac 'login:') grub=$(tail -n +"$sl" "$RUN/serial.log" | grep -ac 'GNU GRUB') nvrm=$(grep -c NVRM "$OUT/reboot${i}_dmesg.log") xid=$(grep -c 'Xid' "$OUT/reboot${i}_dmesg.log") wpr=$(grep -ci 'wpr' "$OUT/reboot${i}_dmesg.log") rminit_fail=$(grep -c 'RmInitAdapter failed' "$OUT/reboot${i}_dmesg.log") smi=[$(gq 'nvidia-smi -L 2>&1 | head -2' 30 | tr '\n' ' ')] desktop=$(gq 'pgrep -u ubuntu -x cinnamon >/dev/null && echo up || echo no' 10)"
    P "REBOOT${i}_KF3 $(tail -n 400 "$RUN/qemu.log" | grep -a -i -E 'reset|refus|wpr|kf3: broker: (connected|re-sent)|guest armed' | tail -6 | sed 's/^[^ ]* //' | tr '\n' '|' | cut -c1-600)"
done
if [ "${PROOF_REATTACH:-0}" = 1 ]; then
    # QEMU restarted while the broker keeps running: the new relay must connect to the same broker
    bp=$(pgrep -n -f "^[^ ]*nvkvm-display-broker --socket"); q=$(qpid)
    mon quit; for _ in $(seq 30); do [ -n "$(qpid)" ] || break; sleep 1; done
    wait "$VMPID" 2>/dev/null
    RUN=$OUT/vm2
    KF_REUSE_BROKER=1 KF_RUN_DIR=$RUN "$HERE/interactive.sh" run > "$RUN.out" 2>&1 &
    VMPID=$!
    for _ in $(seq 60); do grep -aq 'kf3: broker: connected' "$RUN/qemu.log" 2>/dev/null && break; sleep 1; done
    sleep 15
    P "REATTACH_DISPLAY stalled=$(grep -ac 'did not complete' "$RUN/qemu.log") $(grep -ao 'display fps\[[^]]*\]' "$RUN/qemu.log" | tail -1) (predicted from ab-D: stalled, gop=on with the broker connected at start)"
    P "REATTACH broker_pid_before=$bp after=$(pgrep -n -f "^[^ ]*nvkvm-display-broker --socket") qemu=$q->$(qpid) launcher=[$(grep -a 'reusing' "$RUN.out" | cut -c1-120)] relay=[$(grep -a 'kf3: broker: connected' "$RUN/qemu.log" | head -1 | sed 's/.*kf3: broker: //' | cut -c1-160)] broker_accepts=$(grep -ac 'accepted uid' "$OUT/vm1/broker2.log")"
fi
# stop the VM (and the broker the run started)
mon system_powerdown; for _ in $(seq 60); do [ -n "$(qpid)" ] || break; sleep 1; done
[ -n "$(qpid)" ] && kill -9 "$(qpid)"
wait "$VMPID" 2>/dev/null
pkill -f "^[^ ]*nvkvm-display-broker --socket /run/user/[0-9]*/nvkvm/display.sock"
exec 7>&-
P "EXIT $(date -Is) qemu_left=$(pgrep -c -x qemu-system-x86)"
