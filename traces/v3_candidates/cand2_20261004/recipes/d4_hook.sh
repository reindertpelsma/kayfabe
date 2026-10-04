#!/usr/bin/env bash
# d4_hook.sh <tag> — DISPLAY_HOOK=d4_hook for lane.sh on the SCRATCH merge (v3-maxfps + v3-dispsw-exp),
# the D4 question (OWNER_RULINGS §M; V3_DISPLAY.md §8.16): does display-max-fps bound X11 vsync?
# hook.sh first (DISPLAY_DESKTOP=1 Cinnamon on Xorg, DISPLAY_X11_BARE=1 bare Xorg, FPS_BOUND), then on a
# bare Xorg (no compositor), every client timed and the fps fragment read while it runs:
#   B1 glxgears windowed, vsync (20 s)         B2 glxgears -fullscreen, vsync (the flip path)
#   B3 vkcube FIFO 480 frames, timed           B4 glxgears -fullscreen, __GL_SYNC_TO_VBLANK=0 (D1, async)
#   B5 vkcube IMMEDIATE 480 frames, timed
#   R1B (D4_R1B=1): the same B1-B3 with a 1080p 60 Hz CEA mode forced past the EDID (ModeValidation
#       overrides) — the raster 60 Hz, so only the clamp can hold the tick at the cap
set -uo pipefail
TAG=${1:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"; G="$HERE/../gssh_nv"
BENCH=${BENCH_DIR:-/workspace/bench}
OUT=${DISPLAY_RES_DIR:-$BENCH/display/$TAG}; mkdir -p "$OUT"
Q=$BENCH/run_${TAG}_qemu.log
say(){ echo "D4_$* [$(date +%T)]"; }
gq(){ timeout "${2:-60}" "$G" "$1" 2>&1 | tr -d '\r'; }
fpsl(){ grep -a 'kf3: display fps\[' "$Q" | tail -1 | sed 's/^.*kf3: display //'; }
bash "$HERE/hook_q.sh" "$TAG"
say "HOOK_SH_RC=$?"
BX='sudo -u ubuntu env DISPLAY=:0'
xup(){  # $1 = extra Xorg args, $2 = label
    gq 'sudo systemctl stop lightdm 2>/dev/null; sudo pkill -x Xorg; sleep 3; echo ok' 40 > /dev/null
    gq "sudo sh -c 'nohup Xorg :0 -nolisten tcp -noreset -ac $1 -logfile /var/log/Xorg.7.log > /tmp/xd4.log 2>&1 &' && echo started" 30 > /dev/null
    up=no; for _ in $(seq 30); do gq "$BX xset q >/dev/null 2>&1 && echo UP" | grep -q UP && { up=yes; break; }; sleep 2; done
    sleep 3
    gq "$BX xrandr 2>&1 | head -8" > "$OUT/d4_xrandr_$2.log"
    say "X_$2 up=$up mode=[$(grep '\*' "$OUT/d4_xrandr_$2.log" | tr -s ' ' | head -2 | tr '\n' '|')] $(grep -a 'period [0-9]* ns' "$Q" | tail -1 | sed 's/^.*kf3: display: //' | cut -c1-160)"
}
runset(){  # $1 = label
    L=$1
    ( gq "$BX timeout 21 glxgears 2>&1 | grep 'frames in'" 40 > "$OUT/d4_${L}_glxgears.log" ) &
    VP=$!; sleep 14; f=$(fpsl); wait $VP
    say "${L}_B1_GLXGEARS_WINDOWED_VSYNC [$(tr '\n' ' ' < "$OUT/d4_${L}_glxgears.log")] fps=[$f]"
    ( gq "$BX timeout 16 glxgears -fullscreen 2>&1 | grep 'frames in'" 40 > "$OUT/d4_${L}_glxgears_fs.log" ) &
    VP=$!; sleep 12; f=$(fpsl); wait $VP
    say "${L}_B2_GLXGEARS_FULLSCREEN_VSYNC [$(tr '\n' ' ' < "$OUT/d4_${L}_glxgears_fs.log")] fps=[$f]"
    ( gq "t0=\$(date +%s%N); $BX timeout 60 vkcube --c 480 --present_mode 2 2>&1 | tail -3; echo RC=\${PIPESTATUS[0]} WALL_MS=\$(( (\$(date +%s%N) - t0) / 1000000 ))" 90 > "$OUT/d4_${L}_vkcube_fifo.log" ) &
    VP=$!; sleep 10; f=$(fpsl); wait $VP
    say "${L}_B3_VKCUBE_FIFO_480 $(grep -m1 '^RC=' "$OUT/d4_${L}_vkcube_fifo.log") $(grep -m1 -o 'Assertion.*' "$OUT/d4_${L}_vkcube_fifo.log" | head -c 100) fps=[$f]"
    [ "${2:-}" = short ] && return
    ( gq "$BX __GL_SYNC_TO_VBLANK=0 timeout 16 glxgears -fullscreen 2>&1 | grep 'frames in'" 40 > "$OUT/d4_${L}_glxgears_fs_novsync.log" ) &
    VP=$!; sleep 12; f=$(fpsl); wait $VP
    say "${L}_B4_GLXGEARS_FULLSCREEN_NOVSYNC [$(tr '\n' ' ' < "$OUT/d4_${L}_glxgears_fs_novsync.log")] fps=[$f]"
    ( gq "t0=\$(date +%s%N); $BX timeout 60 vkcube --c 480 --present_mode 0 2>&1 | tail -3; echo RC=\${PIPESTATUS[0]} WALL_MS=\$(( (\$(date +%s%N) - t0) / 1000000 ))" 90 > "$OUT/d4_${L}_vkcube_imm.log" ) &
    VP=$!; sleep 5; f=$(fpsl); wait $VP
    say "${L}_B5_VKCUBE_IMMEDIATE_480 $(grep -m1 '^RC=' "$OUT/d4_${L}_vkcube_imm.log") $(grep -m1 -o 'Assertion.*' "$OUT/d4_${L}_vkcube_imm.log" | head -c 100) fps=[$f]"
}
xup "" edid
runset bare
gq 'sudo cat /var/log/Xorg.7.log' 30 > "$OUT/d4_Xorg_edid.log"
if [ "${D4_R1B:-0}" = 1 ]; then
    cat > /tmp/kf-r1b.conf.$$ <<'XC'
Section "Monitor"
    Identifier "Mon0"
    Modeline "1920x1080_60cea" 148.50 1920 2008 2052 2200 1080 1084 1089 1125 +hsync +vsync
EndSection
Section "Device"
    Identifier "Dev0"
    Driver "nvidia"
    Option "ModeValidation" "AllowNonEdidModes, NoVertRefreshCheck, NoHorizSyncCheck, NoMaxPClkCheck, NoEdidMaxPClkCheck, NoEdidModes"
EndSection
Section "Screen"
    Identifier "Scr0"
    Device "Dev0"
    Monitor "Mon0"
    Option "MetaModes" "1920x1080_60cea +0+0"
EndSection
XC
    timeout 30 "$G" 'sudo tee /etc/X11/kf-r1b.conf >/dev/null' < /tmp/kf-r1b.conf.$$; rm -f /tmp/kf-r1b.conf.$$
    xup "-config kf-r1b.conf" r1b
    gq 'sudo grep -a -i "1920x1080_60cea\|ModeValidation\|validated\|refresh\|(EE)" /var/log/Xorg.7.log | head -20' 30 > "$OUT/d4_r1b_xorg_modes.log"
    say "R1B_XORG $(grep -c . "$OUT/d4_r1b_xorg_modes.log") lines: $(grep -a -m2 -i '1920x1080_60cea' "$OUT/d4_r1b_xorg_modes.log" | tr '\n' '|' | cut -c1-240)"
    runset r1b short
    gq 'sudo cat /var/log/Xorg.7.log' 30 > "$OUT/d4_Xorg_r1b.log"
    gq 'sudo rm -f /etc/X11/kf-r1b.conf; echo ok' 20 > /dev/null
fi
gq 'sudo pkill -x Xorg; echo ok' 30 > /dev/null
grep -a 'kf3: display fps\[' "$Q" | tail -60 | sed 's/^.*kf3: display /D4_FPSLINE /'
grep -a 'period [0-9]* ns' "$Q" | tail -12 | sed 's/^.*kf3: display: /D4_PERIOD /' | cut -c1-200
say "HOOK_DONE"
