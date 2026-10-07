#!/usr/bin/env bash
# max_fps_hook.sh <tag> — DISPLAY_HOOK=max_fps_hook for lane.sh (the v3-maxfps box stage, V3_DISPLAY.md §8.16):
# the stock hook.sh first (env passes through: FPS_BOUND, DISPLAY_ASYNC, DISPLAY_WESTON, ...), then
#   W. Weston on the DRM backend again: vkcube-wayland FIFO / IMMEDIATE / MAILBOX (400 frames, timed),
#      weston-simple-egl with vsync and with -b (swap interval 0), the fps fragment during each
#   U. D2 with NOBODY watching (no broker in this lane, no VNC client, no screendump): fbcon front-buffer
#      writes while the fps fragment is watched, then two screendumps that must show the newest picture
# Every result is one MF_* line on stdout (boot_capture appends it to the probe log). No step may hang.
set -uo pipefail
TAG=${1:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"; G="$HERE/../gssh_nv"
BENCH=${BENCH_DIR:-/workspace/bench}
OUT=${DISPLAY_RES_DIR:-$BENCH/display/$TAG}; mkdir -p "$OUT"
MON=$BENCH/run_${TAG}.mon; Q=$BENCH/run_${TAG}_qemu.log
say(){ echo "MF_$* [$(date +%T)]"; }
gq(){ timeout "${2:-60}" "$G" "$1" 2>&1 | tr -d '\r'; }
fpsl(){ grep -a 'kf3: display fps\[' "$Q" | tail -1 | sed 's/^.*kf3: display //'; }
# E. the EDID the guest reads (no broker in this lane: the boot monitor), after nvidia-drm is loaded
gq 'sudo modprobe nvidia-drm modeset=1 fbdev=1; sleep 3; for e in /sys/class/drm/card*-*/edid; do echo "EDID_FILE $e bytes=$(cat $e | wc -c) status=$(cat $(dirname $e)/status) sha256=$(sha256sum < $e | cut -c1-64)"; done' 90 > "$OUT/mf_edid_list.txt"
sed 's/^/MF_/' "$OUT/mf_edid_list.txt"
for e in $(awk '$1 == "EDID_FILE" && $3 != "bytes=0" {print $2}' "$OUT/mf_edid_list.txt"); do
    n=$(basename "$(dirname "$e")"); gq "xxd -p $e" > "$OUT/mf_edid_$n.hex"
    say "EDID_HEX $n $(tr -d '\n' < "$OUT/mf_edid_$n.hex")"
done
grep -a 'kf3: display: monitor\|EDID fnv' "$Q" | head -4 | cut -c1-240 | sed 's/^/MF_QLOG /'
bash "$HERE/hook.sh" "$TAG"
say "HOOK_SH_RC=$?"
[ "${MF_STEPS:-1}" = 1 ] || { grep -a 'kf3: display fps\[' "$Q" | tail -20 | sed 's/^.*kf3: display /MF_FPSLINE /'; say "HOOK_DONE (MF_STEPS=0)"; exit 0; }
shot(){ python3 "$HERE/refresh_probe.py" qmp "${MON%.mon}.qmp" "$1"; }
# W. Weston again, every present mode timed
gq 'sudo systemctl stop lightdm 2>/dev/null; sudo pkill -x weston; sleep 2; echo ok' 30 > /dev/null
gq "sudo rm -rf /run/kfw && sudo mkdir -m 700 /run/kfw && sudo sh -c 'XDG_RUNTIME_DIR=/run/kfw LIBSEAT_BACKEND=builtin nohup weston --backend=drm --continue-without-input --socket=kfw --log=/tmp/weston2.log >/dev/null 2>&1 &' && echo started" 30 > "$OUT/mf_weston_start.log"
sleep 10
WENV='sudo env XDG_RUNTIME_DIR=/run/kfw WAYLAND_DISPLAY=kfw'
say "WESTON $(tr '\n' ' ' < "$OUT/mf_weston_start.log") alive=$(gq 'pgrep -x weston >/dev/null && echo yes || echo no')"
for spec in 2:400:8 0:3000:4 1:3000:4; do
    pm=${spec%%:*}; rest=${spec#*:}; n=${rest%%:*}; at=${rest#*:}
    ( gq "t0=\$(date +%s%N); $WENV timeout 60 vkcube-wayland --c $n --present_mode $pm 2>&1 | tail -4; echo RC=\${PIPESTATUS[0]} WALL_MS=\$(( (\$(date +%s%N) - t0) / 1000000 ))" 90 > "$OUT/mf_vkcube_wl_pm$pm.log" ) &
    VP=$!; sleep "$at"; f=$(fpsl); wait $VP
    say "VKCUBE_WAYLAND_PM$pm frames=$n $(grep -m1 -o 'Assertion.*\|Selected GPU[^,]*\|not supported[^.]*' "$OUT/mf_vkcube_wl_pm$pm.log" | tail -1 | head -c 100) $(grep -m1 '^RC=' "$OUT/mf_vkcube_wl_pm$pm.log") fps_during=[$f]"
done
gq 'sudo pkill -x weston; sleep 2; echo ok' 30 > /dev/null
# U. D2 unwatched: fbcon writes (a front buffer, no flip) while nobody watches, then screendumps
gq 'sudo chvt 1; sleep 1; echo ok' 30 > /dev/null
sleep 4; fa=$(fpsl)
( gq "sudo sh -c 'for i in \$(seq 120); do date +%T.%N > /dev/tty1; sleep 0.05; done'" 40 ) &
WP=$!; sleep 6; fb=$(fpsl); wait $WP; sleep 3; fc=$(fpsl)
say "D2_UNWATCHED before=[$fa] during_fbcon_writes=[$fb] after=[$fc]"
for c in '41:red' '44:blue'; do
    code=${c%%:*}; name=${c#*:}
    gq "sudo sh -c 'printf \"\\033[${code}m\\033[2J\" > /dev/tty1'" 20 > /dev/null; sleep 4; fq=$(fpsl)
    r=$(shot "$OUT/mf_unwatched_$name.ppm")
    px=$(python3 -c "
d=open('$OUT/mf_unwatched_$name.ppm','rb').read().split(b'\n',3); w,h=map(int,d[1].split()); b=d[3]
from collections import Counter
c=Counter('%02x%02x%02x'%(b[i],b[i+1],b[i+2]) for i in range(0,len(b),3*97)).most_common(2)
print('size=%dx%d dominant=%s'%(w,h,c))
" 2>&1)
    say "D2_SCREENDUMP $name $r $px fps_before=[$fq]"
    convert "$OUT/mf_unwatched_$name.ppm" -resize 640x "$OUT/mf_unwatched_$name.png" 2>/dev/null && rm -f "$OUT/mf_unwatched_$name.ppm"
done
sleep 3; say "D2_AFTER fps=[$(fpsl)]"
gq "sudo sh -c 'printf \"\\033[0m\\033[2J\" > /dev/tty1'" 20 > /dev/null
grep -a 'kf3: display fps\[' "$Q" | tail -40 | sed 's/^.*kf3: display /MF_FPSLINE /'
say "HOOK_DONE"
