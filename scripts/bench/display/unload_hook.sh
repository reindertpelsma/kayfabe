#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# unload_hook.sh <tag> — box test B5 (docs/design/V3_DISPLAY.md §4.11.9, §4.11.13): what the screen
# shows as each display client goes away, on a guest booted with the GOP ROM. A POST_CAPTURE_HOOK for
# lane.sh, selected with DISPLAY_HOOK=unload_hook. It runs on the HOST with the guest up and nvidia.ko
# loaded (no nvidia-drm yet; boot_capture's nvidia-smi has come and gone, so the guest's RM has
# already initialised and torn the adapter down once). One DISPLAY_B5* line per arm; shots in $OUT.
#   (c)  nvidia.ko, no RM client: the firmware console (simpledrm on BAR1 [0, G)) must show new text.
#   (c2) the same with an RM client holding /dev/nvidia0 (RM up, its console mapping at BAR1 VA 0),
#        then (c3) once more after that client closed (RM torn down a second time).
#   (a)  lightdm with no xorg.conf and no nvidia-drm: Xorg runs on simpledrm (modesetting) and the
#        NVIDIA GL/EGL libraries open /dev/nvidia0 — RM up for X's life. After X exits the text
#        console must be back. ⊘ No NVKMS here (no display channel is ever allocated).
#   (a2) the NVIDIA X driver (an xorg.conf naming it), modeset=0: NVKMS imports the console and
#        restores it when X — its last client — closes; the text console must be back.
#   (b)  nvidia-drm modeset=1 fbdev=1, fbcon unbound, nvidia-drm removed (nvidia-modeset stays).
# Every step logs the guest's uptime so it lines up with the guest dmesg and the QEMU log.
# ⊘ No step may hang: every guest command has a deadline. A missing line is reported as missing.
set -uo pipefail
TAG=${1:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"; G="$HERE/../gssh_nv"
BENCH=${BENCH_DIR:-/workspace/bench}
OUT=${DISPLAY_RES_DIR:-$BENCH/display/$TAG}; mkdir -p "$OUT"
MON=$BENCH/run_${TAG}.mon
say(){ echo "DISPLAY_$*"; }
gq(){ timeout "${2:-60}" "$G" "$1" 2>&1 | tr -d '\r'; }
upt(){ gq 'cut -d" " -f1 /proc/uptime' 15; }
shot(){
    python3 - "$MON" "$1" <<'PY'
import socket, sys, time
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.settimeout(10)
time.sleep(0.2); s.recv(65536)
s.sendall(("screendump %s kf0\n" % sys.argv[2]).encode()); time.sleep(2)
try: print(s.recv(65536).decode(errors="replace").strip().splitlines()[-1])
except Exception as e: print("recv:", e)
PY
}
# "<w>x<h> nonblack=<per mille> md5=<12 hex>" for a P6 screendump (QEMU writes no header comments)
stats(){
    python3 - "$1" <<'PY'
import sys, hashlib
try:
    d = open(sys.argv[1], 'rb').read()
except OSError as e:
    print("no-shot (%s)" % e); sys.exit(0)
magic, wh, maxval, px = d.split(b'\n', 3)
w, h = map(int, wh.split())
step = 3 * 101
idx = range(0, len(px) - 2, step)
nb = sum(1 for i in idx if max(px[i:i + 3]) > 16)
print("%dx%d nonblack=%d/1000 md5=%s" % (w, h, nb * 1000 // max(1, len(idx)), hashlib.md5(px).hexdigest()[:12]))
PY
}
changed(){ [ "${1##*md5=}" != "${2##*md5=}" ] && echo yes || echo no; }
# write <n> lines tagged <tag> to tty1, then shoot <name>; prints "before=[..] after=[..] changed=.."
lines_and_shot(){
    local tag=$1 n=$2 name=$3 b a
    shot "$OUT/${name}_before.ppm" >/dev/null
    gq "for i in \$(seq 1 $n); do echo \"KF3-$tag line \$i\"; done | sudo tee /dev/tty1 >/dev/null; echo ok" 30 >/dev/null
    sleep 2; shot "$OUT/${name}_after.ppm" >/dev/null
    b=$(stats "$OUT/${name}_before.ppm"); a=$(stats "$OUT/${name}_after.ppm")
    echo "before=[$b] after=[$a] changed=$(changed "$b" "$a")"
}
HAS_CONSOLE=no; grep -q 'kf3: display console registered' "$BENCH/run_${TAG}_qemu.log" 2>/dev/null && HAS_CONSOLE=yes
say "B5_CONSOLE=$HAS_CONSOLE"
[ "$HAS_CONSOLE" = yes ] && [ -S "$MON" ] || { say "B5_SKIPPED no kf3 graphic console or monitor socket"; exit 0; }
BDF=$(gq "lspci -D -d 10de: | awk 'NR==1{print \$1}'")
IFS=':.' read -r _ _b _d _f <<< "$BDF"
BUSID=$(printf 'PCI:%d:%d:%d' "0x${_b:-0}" "0x${_d:-0}" "0x${_f:-0}")

# (c) nvidia.ko only, no RM client: the console the ROM set up (simpledrm on BAR1 [0, G)) must show
#     40 new lines. On a real card RM returned BAR1 to physical mode when its last client closed.
say "B5C_MODULES $(gq 'lsmod | awk "/^nvidia/{print \$1}" | tr "\n" " "') fb=[$(gq 'cat /proc/fb | tr "\n" " "')]"
gq 'sudo chvt 1; echo ok' 20 >/dev/null
say "B5C_CUDA_ONLY_CONSOLE up=$(upt) $(lines_and_shot B5C 40 b5c)"
# (c2) RM held up by a client: its console mapping at BAR1 VA 0 is live.
gq 'sudo rm -f /tmp/kf3hold.pid; sudo setsid sh -c "sleep 60 </dev/nvidia0 >/dev/null 2>&1 & echo \$! >/tmp/kf3hold.pid" </dev/null >/dev/null 2>&1; sleep 8; echo ok' 30 >/dev/null
say "B5C2_RM_HELD up=$(upt) holder=$(gq 'cat /tmp/kf3hold.pid 2>/dev/null || echo none') $(lines_and_shot B5C2 20 b5c2)"
# (c3) the holder closes: RM tears the adapter down a second time.
gq 'sudo kill "$(cat /tmp/kf3hold.pid)" 2>/dev/null; sleep 6; echo ok' 30 >/dev/null
say "B5C3_AFTER_SECOND_TEARDOWN up=$(upt) $(lines_and_shot B5C3 20 b5c3)"

# (a) lightdm, no xorg.conf, no nvidia-drm: which X driver runs is RECORDED, not assumed.
say "B5A_DRM_MODESET_PARAM $(gq 'cat /sys/module/nvidia_drm/parameters/modeset 2>/dev/null || echo not-loaded') xorg_conf=$(gq 'ls /etc/X11/xorg.conf /etc/X11/xorg.conf.d/ 2>&1 | tr "\n" " "')"
XENV='sudo env DISPLAY=:0 XAUTHORITY=/var/run/lightdm/root/:0'
x_up(){ for i in $(seq 1 45); do
    if gq "$XENV xset q >/dev/null 2>&1 && echo UP" 15 | grep -q UP; then echo yes; return; fi
    sleep 2; done; echo no; }
x_gone(){ for i in $(seq 1 20); do gq 'pgrep -x Xorg >/dev/null && echo RUNNING' 10 | grep -q RUNNING || break; sleep 1; done; }
x_driver(){ gq 'sudo grep -E "LoadModule: \"(nvidia|modesetting|fbdev|vesa)\"|\(II\) (NVIDIA|modeset|FBDEV)\([0-9]\): |\(EE\)|NVIDIA\(0\): Failed" /var/log/Xorg.0.log | head -12 | tr "\n" "|"' 30; }
a_arm(){  # a_arm <label> <shot prefix>
    local label=$1 p=$2 t0 up
    t0=$(upt); gq 'sudo systemctl start lightdm; echo rc=$?' 60 > "$OUT/${p}_start.log"
    up=$(x_up); sleep 10; shot "$OUT/${p}_x.ppm" >/dev/null
    gq 'sudo cat /var/log/Xorg.0.log' 30 > "$OUT/${p}_Xorg.0.log"
    say "${label}_X up=$up start_up=$t0 $(tr '\n' ' ' < "$OUT/${p}_start.log") shot=[$(stats "$OUT/${p}_x.ppm")] driver=[$(x_driver)] modules=[$(gq 'lsmod | awk "/^nvidia/{print \$1}" | tr "\n" " "')]"
    t0=$(upt); gq 'sudo systemctl stop lightdm; echo rc=$?' 60 > "$OUT/${p}_stop.log"; x_gone
    gq 'sudo chvt 1; echo ok' 20 >/dev/null
    sleep 3; shot "$OUT/${p}_after.ppm" >/dev/null
    say "${label}_AFTER_X stop_up=$t0 $(tr '\n' ' ' < "$OUT/${p}_stop.log") shot=[$(stats "$OUT/${p}_after.ppm")] changed_from_x=$(changed "$(stats "$OUT/${p}_x.ppm")" "$(stats "$OUT/${p}_after.ppm")") (expected: the text console)"
    say "${label}_CONSOLE_AFTER_X up=$(upt) $(lines_and_shot "${label}" 10 "${p}_tty")"
}
a_arm B5A b5a
# (a2) the NVIDIA X driver, modeset=0 (nvidia-drm not loaded): NVKMS is X's display path.
printf 'Section "Device"\n    Identifier "kf3-b5"\n    Driver "nvidia"\n    BusID "%s"\nEndSection\n' "$BUSID" \
    | $G 'sudo mkdir -p /etc/X11/xorg.conf.d && sudo tee /etc/X11/xorg.conf.d/90-kf3-b5-nvidia.conf >/dev/null; echo ok' >/dev/null
a_arm B5A2 b5a2
gq 'sudo rm -f /etc/X11/xorg.conf.d/90-kf3-b5-nvidia.conf; echo ok' 20 >/dev/null
gq 'sudo dmesg | grep -i -E "nvidia-modeset|nvkms|console|fbcon" | tail -30' 30 > "$OUT/b5a_dmesg.log"

# (b) modeset=1 fbdev=1, then fbcon unbound and nvidia-drm removed (nvidia-modeset stays loaded).
gq 'sudo rmmod nvidia_drm 2>/dev/null; sudo modprobe nvidia-drm modeset=1 fbdev=1; echo rc=$?' 90 > "$OUT/b5b_load.log"
sleep 4; gq 'sudo chvt 1; echo "KF3-B5B console on nvidia-drm fbdev" | sudo tee /dev/tty1 >/dev/null' 20 >/dev/null
sleep 2; shot "$OUT/b5b_fbcon.ppm" >/dev/null
say "B5B_FBCON up=$(upt) $(tr '\n' ' ' < "$OUT/b5b_load.log") fb=[$(gq 'cat /proc/fb | tr "\n" " "')] shot=[$(stats "$OUT/b5b_fbcon.ppm")]"
t0=$(upt)
gq 'for v in /sys/class/vtconsole/vtcon*/bind; do echo 0 | sudo tee "$v" >/dev/null; done; echo unbound; cat /proc/uptime; sudo rmmod nvidia_drm; echo rc=$?; lsmod | grep -c "^nvidia_drm"' 90 > "$OUT/b5b_unload.log"
sleep 4; shot "$OUT/b5b_after.ppm" >/dev/null
say "B5B_AFTER_RMMOD up=$t0 $(tr '\n' ' ' < "$OUT/b5b_unload.log") shot=[$(stats "$OUT/b5b_after.ppm")] modules=[$(gq 'lsmod | awk "/^nvidia/{print \$1}" | tr "\n" " "')]"
gq 'sudo dmesg | tail -60' 30 > "$OUT/b5_dmesg_tail.log"
# the device's own account of each arm: what the console showed, BAR1's boot range, the teardowns
grep -a -E 'the console shows|BAR1 boot framebuffer|wrote NV_|BAR1_BLOCK|fn 47|boot display seed|physical view|armed its first head|ChannelFreed \{ kind: Core|ChannelAllocated \{ kind: Core' \
    "$BENCH/run_${TAG}_qemu.log" > "$OUT/b5_device.log" 2>/dev/null
say "B5_DEVICE_LINES $(wc -l < "$OUT/b5_device.log")"
say "B5_HOOK_DONE"
