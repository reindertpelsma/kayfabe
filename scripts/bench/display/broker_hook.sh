#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# broker_hook.sh <tag> — POST_CAPTURE_HOOK of broker_lane.sh (docs/design/V3_DISPLAY.md §8.11-§8.12).
# Runs on the HOST with the guest up, `nvidia` loaded in it, and the broker in the host's desktop.
#   1. guest KMS (nvidia-drm modeset=1 fbdev=1) — the relay's first frames
#   2. the guest's Cinnamon session (Xorg + the stock NVIDIA X driver, lightdm autologin), as hook.sh
#   3. the broker window fullscreen (1:1 with the guest's 1920x1080, so cursor images compare 1:1)
#   4. HOVER: the host pointer over the window; the host's cursor image (XFixes) against the guest
#      X server's own cursor image; host and guest screenshots
#   5. HIDE: the guest hides its cursor (XFixesHideCursor) — the host's must go blank, then return
#   6. GRAB: CTRL+ALT+G — the host's cursor blank, the cursor COMPOSED into the frame (the console's
#      screendump around the guest pointer differs from hover's); a relative move; CTRL+ALT+G again
# Every result is one BRK_* line (boot_capture appends them to the probe log); artefacts go to
# $BRK_OUT. No step may hang: every guest and host command has a deadline.
set -uo pipefail
TAG=${1:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"; G="$HERE/../gssh_nv"
BENCH=${BENCH_DIR:-/workspace/bench}
OUT=${BRK_OUT:-$BENCH/brk/$TAG}; mkdir -p "$OUT"
MON=$BENCH/run_${TAG}.mon; Q=$BENCH/run_${TAG}_qemu.log
XC="$HERE/xcursor.py"
say(){ echo "BRK_$*"; }
gq(){ timeout "${2:-60}" "$G" "$1" 2>&1 | tr -d '\r'; }
HX(){ timeout "${HXT:-20}" env DISPLAY="$BRK_XD" XAUTHORITY="$BRK_XA" "$@"; }
GX='sudo -u ubuntu env DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority'
# the kf3 console's frame (QEMU screendump) as PNG
shot(){
    python3 - "$MON" "$OUT/$1.ppm" <<'PY'
import socket, sys, time
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.settimeout(10)
time.sleep(0.2); s.recv(65536)
s.sendall(("screendump %s kf0\n" % sys.argv[2]).encode()); time.sleep(2)
PY
    [ -s "$OUT/$1.ppm" ] && convert "$OUT/$1.ppm" "$OUT/$1.png" 2>/dev/null && rm -f "$OUT/$1.ppm"
}
# the host's root window (what the user sees, without the pointer) as PNG
hshot(){ HX import -window root "$OUT/$1.png" 2>/dev/null; }
# pixels that differ between two PNGs inside the 96x96 box at (x, y)
boxdiff(){
    python3 - "$OUT/$1.png" "$OUT/$2.png" "$3" "$4" <<'PY'
import sys
from PIL import Image, ImageChops
a, b = Image.open(sys.argv[1]).convert("RGB"), Image.open(sys.argv[2]).convert("RGB")
x, y = int(sys.argv[3]), int(sys.argv[4])
box = (max(0, x - 16), max(0, y - 16), min(a.width, x + 80), min(a.height, y + 80))
d = ImageChops.difference(a.crop(box), b.crop(box))
print(sum(1 for p in d.getdata() if max(p) > 24))
PY
}
# the fraction of pixels (in 1/10000) that differ between two same-size PNGs — the broker's picture
# (the host's root window with the window fullscreen) against the guest's own frame
fulldiff(){
    python3 - "$OUT/$1.png" "$OUT/$2.png" <<'PY'
import sys
from PIL import Image, ImageChops
a, b = Image.open(sys.argv[1]).convert("RGB"), Image.open(sys.argv[2]).convert("RGB")
if a.size != b.size:
    print("size_mismatch %dx%d vs %dx%d" % (a.size + b.size)); sys.exit(0)
d = ImageChops.difference(a, b)
n = sum(1 for p in d.getdata() if max(p) > 24)
print("differing_per_10000=%d of %dx%d" % (n * 10000 // (a.width * a.height), a.width, a.height))
PY
}
qline(){ wc -l < "$Q"; }
since(){ tail -n +"$(( $1 + 1 ))" "$Q"; }

say "HOOK_START $(date -Is)"
tar -C "$HERE" -cf - xcursor.py | "$G" 'mkdir -p ~/display && tar -xf - -C ~/display'

# 1. guest KMS
gq 'sudo modprobe nvidia-drm modeset=1 fbdev=1; echo rc=$?' 90 > "$OUT/modprobe.log"
sleep 6
say "GUEST_KMS $(tr '\n' ' ' < "$OUT/modprobe.log") nodes=[$(gq 'ls /dev/dri | tr "\n" " "')]"
say "RELAY_CONNECTED $(grep -ac 'kf3: broker: connected to' "$Q") GPU_COPY_PROBE=[$(grep -a 'GPU-copy rung' "$Q" | head -1 | cut -c1-200)]"
say "EV_DEVICE broker=[$(grep -a 'renders on DRM device\|EV_DEVICE will' "${BRK_BROKER_LOG:-/dev/null}" | head -1 | cut -c1-160)] relay=[$(grep -a 'the compositor' "$Q" | head -1 | cut -c1-160)] host_nodes=[$(stat -c '%n=%t:%T' /dev/dri/card* /dev/dri/renderD* 2>/dev/null | tr '\n' ' ')]"

# 2. the guest's desktop (Cinnamon on Xorg, as hook.sh's M3)
bdf=$(gq "lspci -D -d 10de: | awk 'NR==1{print \$1}'")
IFS=':.' read -r _ b d f <<< "$bdf"
busid=$(printf 'PCI:%d:%d:%d' "0x${b:-0}" "0x${d:-0}" "0x${f:-0}")
session=$(gq 'ls /usr/share/xsessions/' | sed -n 's/^\(cinnamon[a-z0-9-]*\)\.desktop$/\1/p' | head -1)
DESK=$(mktemp -d)
sed "s/@BUSID@/$busid/" "$HERE/desktop/xorg.conf.in" > "$DESK/xorg.conf"
sed "s/@SESSION@/${session:-cinnamon}/g" "$HERE/desktop/50-kf-autologin.conf.in" > "$DESK/50-kf-autologin.conf"
tar -C "$DESK" -cf - xorg.conf 50-kf-autologin.conf | "$G" 'rm -rf ~/desk && mkdir -p ~/desk && tar -xf - -C ~/desk'
rm -rf "$DESK"
gq 'sudo cp ~/desk/xorg.conf /etc/X11/xorg.conf && sudo mkdir -p /etc/lightdm/lightdm.conf.d && sudo cp ~/desk/50-kf-autologin.conf /etc/lightdm/lightdm.conf.d/ && : > ~/.xsessionrc && sudo systemctl start lightdm; echo rc=$?' 60 > "$OUT/lightdm.log"
up=no
for _ in $(seq 1 45); do
    if gq "$GX xset q >/dev/null 2>&1 && pgrep -u ubuntu -x cinnamon >/dev/null && echo UP" | grep -q UP; then up=yes; break; fi
    sleep 2
done
say "GUEST_DESKTOP=$up session=${session:-cinnamon} busid=$busid ($(tr '\n' ' ' < "$OUT/lightdm.log"))"
sleep 20
shot guest_desktop
say "RUNGS $(grep -ao 'frames go as [^(;—]*' "$Q" | sort | uniq -c | tr '\n' ' ')"
grep -aE 'kf3: broker: (the compositor|the display (CAN|CANNOT|imported)|GPU-copy frames are not)' "$Q" | cut -c1-200 | head -6 | sed 's/^/BRK_RUNG_LINE /'

# 3. the broker window, fullscreen (1:1 with the guest)
W=$(HX xdotool search --name '^nvkvm' 2>/dev/null | head -1)
say "WINDOW id=[${W:-none}]"
if [ -z "$W" ]; then say "HOOK_DONE (no broker window)"; exit 0; fi
HX xdotool windowactivate --sync "$W" >/dev/null 2>&1
HX xdotool key --clearmodifiers ctrl+alt+f >/dev/null 2>&1
sleep 4
eval "$(HX xdotool getwindowgeometry --shell "$W" 2>/dev/null)"
say "WINDOW geometry=${WIDTH:-?}x${HEIGHT:-?}+${X:-?}+${Y:-?}"
hshot host_desktop

[ "${BRK_CURSOR:-1}" = 1 ] || { say "HOOK_DONE (no cursor experiments)"; exit 0; }
# 4. HOVER — the host pointer over the picture
HX xdotool windowactivate --sync "$W" >/dev/null 2>&1
HX xdotool mousemove --window "$W" 700 400 >/dev/null 2>&1; sleep 1
HX xdotool mousemove --window "$W" 720 410 >/dev/null 2>&1; sleep 3
say "MODE_LINES $(grep -a 'kf3: broker: guest cursor:' "$Q" | cut -d: -f4- | tr '\n' '|' | cut -c1-300)"
HX python3 "$XC" image "$OUT/host_hover.pam" | sed 's/^/BRK_HOST_HOVER /'
gpos=$(gq "$GX python3 ~/display/xcursor.py pointer" | sed -n 's/^POINTER //p')
say "GUEST_POINTER $gpos"
gq "$GX python3 ~/display/xcursor.py image /tmp/guest_hover.pam" | sed 's/^/BRK_GUEST_CURSOR /'
"$G" 'cat /tmp/guest_hover.pam' > "$OUT/guest_hover.pam" 2>/dev/null
python3 "$XC" compare "$OUT/guest_hover.pam" "$OUT/host_hover.pam" | sed 's/^/BRK_HOVER_/'
shot hover
hshot host_hover
say "HOST_VS_GUEST hover $(fulldiff hover host_hover) (the broker fullscreen against the guest frame; the host shot has no pointer, the frame no cursor in hover)"
set -- $gpos; gx=${1:-0}; gy=${2:-0}

# 5. HIDE — the guest hides its cursor for 10 s
m=$(qline)
( gq "$GX python3 ~/display/xcursor.py hide 10" 30 > "$OUT/guest_hide.log" ) &
HP=$!
sleep 4
HX python3 "$XC" image "$OUT/host_hidden.pam" | sed 's/^/BRK_HOST_HIDDEN /'
shot hidden
wait $HP
sleep 3
HX python3 "$XC" image "$OUT/host_after_hide.pam" | sed 's/^/BRK_HOST_AFTER_HIDE /'
say "HIDE_GUEST $(tr '\n' ' ' < "$OUT/guest_hide.log")"
say "HIDE_RELAY $(since "$m" | grep -ac 'kf3: broker:') relay lines, host_cursor_refusals=$(since "$m" | grep -ac 'host cursor REFUSED')"

# 6. GRAB — CTRL+ALT+G, the composed cursor, a relative move, CTRL+ALT+G
m=$(qline)
HX xdotool windowactivate --sync "$W" >/dev/null 2>&1
HX xdotool key --clearmodifiers ctrl+alt+g >/dev/null 2>&1
sleep 3
say "GRAB_ON $(since "$m" | grep -aE 'kf3: broker: (grab|guest cursor)' | cut -d: -f4- | tr '\n' '|' | cut -c1-240)"
HX python3 "$XC" image "$OUT/host_grab.pam" | sed 's/^/BRK_HOST_GRAB /'
shot grab
say "GRAB_FRAME_DIFF hover_vs_grab_px=$(boxdiff hover grab "$gx" "$gy") (the composed cursor near the guest pointer $gx,$gy)"
HX xdotool mousemove_relative -- 60 40 >/dev/null 2>&1; sleep 2
gpos2=$(gq "$GX python3 ~/display/xcursor.py pointer" | sed -n 's/^POINTER //p')
shot grab_moved
set -- $gpos2
say "GRAB_MOVE guest_pointer=$gpos -> $gpos2 moved_cursor_px=$(boxdiff grab grab_moved "${1:-0}" "${2:-0}")"
m=$(qline)
HX xdotool key --clearmodifiers ctrl+alt+g >/dev/null 2>&1
sleep 3
say "GRAB_OFF $(since "$m" | grep -aE 'kf3: broker: (grab|guest cursor)' | cut -d: -f4- | tr '\n' '|' | cut -c1-240)"
HX xdotool mousemove --window "$W" 720 410 >/dev/null 2>&1; sleep 2
HX python3 "$XC" image "$OUT/host_after_grab.pam" | sed 's/^/BRK_HOST_AFTER_GRAB /'
shot after_grab

# E5 (part): what QEMU holds — descriptors by kind, its VRAM
pid=${KF_QEMU_PID:-}
if [ -n "$pid" ] && [ -d "/proc/$pid/fd" ]; then
    say "E5_FDS $(ls -l "/proc/$pid/fd" 2>/dev/null | awk '{print $NF}' | sed 's/[0-9]\+/N/g' | sort | uniq -c | sort -rn | head -12 | tr -s ' ' | tr '\n' ';')"
fi
say "E5_VRAM $(nvidia-smi --query-compute-apps=pid,used_memory --format=csv,noheader 2>/dev/null | tr '\n' ';')"
for f in "$OUT"/*.pam; do [ -s "$f" ] && convert "$f" "${f%.pam}.png" 2>/dev/null; done
say "HOOK_DONE $(date -Is)"
