#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# subpixel_proof.sh <tag> — the broker's relative path on the LIVE Wayland session (V3_DISPLAY.md
# §8.19): one guest (interactive.sh run --record, the broker fullscreen so the host pointer is over the
# content), a host uinput test mouse (uinput_mouse.c) at 1000 and 1600 dpi (udev hwdb MOUSE_DPI),
# slow single counts (one per 20 ms) and fast ones (one per 2 ms), first through OLD_BROKER, then
# through NEW_BROKER (restarted on the same socket; QEMU reconnects). The guest's evtest on the
# virtio mouse is summed per step. One SUBPX line per step: sent counts, expected, received.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
TAG=${1:?usage: subpixel_proof.sh <tag>}
WORK=${KF_INTERACTIVE_DIR:-/var/lib/kf-windows-20261005/broker-interactive}
OLD=${OLD_BROKER:-/opt/nvkvm-broker/nvkvm-display-broker}
NEW=${NEW_BROKER:-/opt/nvkvm-broker-next/nvkvm-display-broker}
RUN=$WORK/subpx-$TAG; rm -rf "$RUN"
U=$WORK/uinput_mouse
cc -O2 -o "$U" "$HERE/uinput_mouse.c" || exit 2
printf 'mouse:*:name:kf-test-1600dpi:*\n MOUSE_DPI=1600@1000\n\nmouse:*:name:kf-test-1000dpi:*\n MOUSE_DPI=1000@1000\n' \
    > /etc/udev/hwdb.d/99-kf-broker-test.hwdb && systemd-hwdb update
G=(ssh -i /workspace/bench/guest_key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR ubuntu@192.168.77.2)
sum(){ "${G[@]}" "awk '/type 2 \\(EV_REL\\), code 0 \\(REL_X\\)/{s+=\$NF} END{print s+0}' /tmp/kf-rec/evtest_Mouse.log"; }
P(){ echo "SUBPX_$*" | tee -a "$RUN.log"; }
BROKER_BIN=$OLD BROKER_ARGS=--fullscreen KF_RUN_DIR=$RUN setsid "$HERE/interactive.sh" run --record > "$RUN.out" 2>&1 < /dev/null &
for _ in $(seq 80); do grep -q started "$RUN/record.txt" 2>/dev/null && break; sleep 5; done
P "START tag=$TAG kf3=$(sed -n 's/.*kf3=\([0-9a-f]*\).*/\1/p' "$RUN/marker.txt") old=$(cat "$(dirname "$OLD")/REV") new=$(cat "$(dirname "$NEW")/REV") $(date -Is)"
step(){   # label, dpi, n, us, expected
    local b a; b=$(sum)
    "$U" "kf-test-${2}dpi" "$3" "$4" > /dev/null 2>&1
    sleep 2; a=$(sum)
    P "$1 dpi=$2 counts=$3 every=${4}us expected=$5 received=$((a - b))"
}
step OLD_slow 1600 200 20000 125
step OLD_slow 1000 200 20000 200
BROKER_BIN=$NEW BROKER_ARGS=--fullscreen "$HERE/interactive.sh" broker > /dev/null 2>&1
for _ in $(seq 30); do grep -aq 'kf3: broker: connected' <(tail -n 20 "$RUN/qemu.log") && break; sleep 1; done
sleep 3
step NEW_slow 1600 200 20000 125
step NEW_slow 1000 200 20000 200
step NEW_fast 1000 600 2000 600
step NEW_fast 1600 600 2000 375
grep -a 'input while grabbed\|grab O\|grab o' "$RUN/qemu.log" | sed 's/^[^ ]* //' > "$RUN.relay.txt"
rm -f /etc/udev/hwdb.d/99-kf-broker-test.hwdb && systemd-hwdb update
P "EXIT $(date -Is) (the guest keeps running; stop it with interactive.sh stop)"
