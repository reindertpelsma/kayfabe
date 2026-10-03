#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# unload_hook.sh <tag> — box test B5 (docs/design/V3_DISPLAY.md §4.11.9): what the screen shows as each
# display client goes away, on a guest booted with the GOP ROM. A POST_CAPTURE_HOOK for lane.sh,
# selected with DISPLAY_HOOK=unload_hook. It runs on the HOST with the guest up and nvidia.ko loaded
# (no nvidia-drm yet). One DISPLAY_B5* line per arm; the shots go to $OUT.
#   (c) a CUDA-only guest (nvidia.ko, no nvidia-drm) keeps its firmware console alive;
#   (a) modeset=0: Xorg starts and stops; the console must come back (NVKMS console restore);
#   (b) modeset=1 fbdev=1: unbinding fbcon and removing nvidia-drm leaves the screen black, which is
#       what bare metal does (sysfb_disable never re-registers the firmware framebuffer).
# ⊘ No step may hang: every guest command has a deadline. A missing line is reported as missing.
set -uo pipefail
TAG=${1:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"; G="$HERE/../gssh_nv"
BENCH=${BENCH_DIR:-/workspace/bench}
OUT=${DISPLAY_RES_DIR:-$BENCH/display/$TAG}; mkdir -p "$OUT"
MON=$BENCH/run_${TAG}.mon
say(){ echo "DISPLAY_$*"; }
gq(){ timeout "${2:-60}" "$G" "$1" 2>&1 | tr -d '\r'; }
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
HAS_CONSOLE=no; grep -q 'kf3: display console registered' "$BENCH/run_${TAG}_qemu.log" 2>/dev/null && HAS_CONSOLE=yes
say "B5_CONSOLE=$HAS_CONSOLE"
[ "$HAS_CONSOLE" = yes ] && [ -S "$MON" ] || { say "B5_SKIPPED no kf3 graphic console or monitor socket"; exit 0; }

# (c) nvidia.ko only: the console the ROM set up must still be live through BAR1 VA 0 after RM
#     took BAR1 into virtual mode. Two shots around 40 lines of new text: if the console is alive the
#     second differs from the first.
say "B5C_MODULES $(gq 'lsmod | awk "/^nvidia/{print \$1}" | tr "\n" " "')"
gq 'sudo chvt 1; echo ok' 20 >/dev/null
shot "$OUT/b5c_before.ppm" >/dev/null
gq 'for i in $(seq 1 40); do echo "KF3-B5C console still alive with nvidia.ko loaded: line $i"; done | sudo tee /dev/tty1 >/dev/null; echo ok' 30 >/dev/null
sleep 2; shot "$OUT/b5c_after.ppm" >/dev/null
b=$(stats "$OUT/b5c_before.ppm"); a=$(stats "$OUT/b5c_after.ppm")
say "B5C_CUDA_ONLY_CONSOLE before=[$b] after=[$a] changed=$([ "${b##*md5=}" != "${a##*md5=}" ] && echo yes || echo no) fb=[$(gq 'cat /proc/fb | tr "\n" " "')]"

# (a) modeset=0 + X: lightdm starts Xorg on the NVIDIA X driver (NVKMS, no nvidia-drm KMS); stopping
#     it is NVKMS's last client going away, which restores the console it imported at start.
say "B5A_DRM_MODESET_PARAM $(gq 'cat /sys/module/nvidia_drm/parameters/modeset 2>/dev/null || echo not-loaded')"
gq 'sudo systemctl start lightdm; echo rc=$?' 60 > "$OUT/b5a_start.log"
XENV='sudo -u ubuntu env DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority'
up=no
for i in $(seq 1 45); do
    if gq "$XENV xset q >/dev/null 2>&1 && echo UP" 15 | grep -q UP; then up=yes; break; fi
    sleep 2
done
sleep 10; shot "$OUT/b5a_x.ppm" >/dev/null
say "B5A_X up=$up $(tr '\n' ' ' < "$OUT/b5a_start.log") shot=[$(stats "$OUT/b5a_x.ppm")] drm_modeset=$(gq 'cat /sys/module/nvidia_drm/parameters/modeset 2>/dev/null || echo not-loaded')"
gq 'sudo systemctl stop lightdm; echo rc=$?' 60 > "$OUT/b5a_stop.log"
for i in $(seq 1 20); do gq 'pgrep -x Xorg >/dev/null && echo RUNNING' 10 | grep -q RUNNING || break; sleep 1; done
gq 'sudo chvt 1; echo "KF3-B5A console after X exit" | sudo tee /dev/tty1 >/dev/null' 20 >/dev/null
sleep 3; shot "$OUT/b5a_after.ppm" >/dev/null
gq 'sudo dmesg | grep -i -E "nvidia-modeset|nvkms|console" | tail -20' 30 > "$OUT/b5a_dmesg.log"
say "B5A_AFTER_X $(tr '\n' ' ' < "$OUT/b5a_stop.log") shot=[$(stats "$OUT/b5a_after.ppm")] (expected: the text console, not black)"

# (b) modeset=1 fbdev=1, then remove nvidia-drm: expected black, as on bare metal.
gq 'sudo rmmod nvidia_drm 2>/dev/null; sudo modprobe nvidia-drm modeset=1 fbdev=1; echo rc=$?' 90 > "$OUT/b5b_load.log"
sleep 4; gq 'sudo chvt 1; echo "KF3-B5B console on nvidia-drm fbdev" | sudo tee /dev/tty1 >/dev/null' 20 >/dev/null
sleep 2; shot "$OUT/b5b_fbcon.ppm" >/dev/null
say "B5B_FBCON $(tr '\n' ' ' < "$OUT/b5b_load.log") fb=[$(gq 'cat /proc/fb | tr "\n" " "')] shot=[$(stats "$OUT/b5b_fbcon.ppm")]"
gq 'for v in /sys/class/vtconsole/vtcon*/bind; do echo 0 | sudo tee "$v" >/dev/null; done; sudo rmmod nvidia_drm; echo rc=$?; lsmod | grep -c "^nvidia_drm"' 90 > "$OUT/b5b_unload.log"
sleep 4; shot "$OUT/b5b_after.ppm" >/dev/null
say "B5B_AFTER_RMMOD $(tr '\n' ' ' < "$OUT/b5b_unload.log") shot=[$(stats "$OUT/b5b_after.ppm")] (expected: black)"
gq 'sudo dmesg | tail -40' 30 > "$OUT/b5_dmesg_tail.log"
say "B5_HOOK_DONE"
