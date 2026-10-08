#!/usr/bin/env bash
# subpixel_swap.sh — run s3 (V3_DISPLAY.md §8.19): against a RUNNING interactive.sh guest, the broker swapped per phase
set -u
K=/var/lib/kf-windows-20261005/kayfabe-broker-interactive/scripts/bench/display
W=/var/lib/kf-windows-20261005/broker-interactive
U=$W/uinput_mouse
G=(ssh -i /workspace/bench/guest_key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR ubuntu@192.168.77.2)
cat > /etc/udev/rules.d/99-kf-broker-test.rules <<'EOF'
ACTION=="add|change", KERNEL=="event*", ATTRS{name}=="kf-test-1600dpi", ENV{MOUSE_DPI}="1600@1000"
ACTION=="add|change", KERNEL=="event*", ATTRS{name}=="kf-test-1000dpi", ENV{MOUSE_DPI}="1000@1000"
EOF
udevadm control --reload
"${G[@]}" 'mkdir -p /tmp/kf-rec; ev=$(grep -A5 "Name=\"QEMU Virtio Mouse\"" /proc/bus/input/devices | grep -o "event[0-9]*" | head -1); sudo pkill -f "evtest /dev/input/$ev"; (sudo stdbuf -oL evtest /dev/input/$ev < /dev/null > /tmp/kf-rec/evtest_Mouse.log 2>&1 &); echo ok'
sum(){ "${G[@]}" "awk '/type 2 \\(EV_REL\\), code 0 \\(REL_X\\)/{s+=\$NF} END{print s+0}' /tmp/kf-rec/evtest_Mouse.log"; }
P(){ echo "SUBPX_$*" | tee -a "$W/subpx-s3.log"; }
step(){ local b a; b=$(sum); "$U" "kf-test-${2}dpi" "$3" "$4" > /dev/null 2>&1; sleep 2; a=$(sum); P "$1 dpi=$2 counts=$3 every=${4}us expected=$5 received=$((a - b))"; }
swap(){ BROKER_BIN=$1 BROKER_ARGS=$2 "$K/interactive.sh" broker > /dev/null 2>&1; sleep 6; }
P "START s3 guest=$(pgrep -f '^/workspace/bench/kf3-bins/[0-9a-f]*/qemu-system-x86_64 -name kayfabe-interactive' | head -1) $(date -Is)"
swap /opt/nvkvm-broker/nvkvm-display-broker --fullscreen
P "BROKER old=$(cat /opt/nvkvm-broker/REV)"
step OLD_slow 1600 200 20000 125
step OLD_slow 1000 200 20000 200
step OLD_fast 1000 600 2000 600
swap /opt/nvkvm-broker-next/nvkvm-display-broker --fullscreen
P "BROKER new=$(cat /opt/nvkvm-broker-next/REV)"
step NEW_slow 1600 200 20000 125
step NEW_slow 1000 200 20000 200
step NEW_fast 1000 600 2000 600
step NEW_fast 1600 600 2000 375
rm -f /etc/udev/rules.d/99-kf-broker-test.rules && udevadm control --reload
swap /opt/nvkvm-broker-next/nvkvm-display-broker ""
P "EXIT $(date -Is) demo left with the new broker, windowed"
