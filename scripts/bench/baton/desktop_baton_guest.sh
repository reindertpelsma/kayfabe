#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# desktop_baton_guest.sh - the guest half of desktop_baton.sh; runs INSIDE the Linux desktop guest
# (as the desktop user, over ssh), against the live Cinnamon/Xorg session.
#
#   desktop_baton_guest.sh info                        what the desktop is (renderer, compositor, screen)
#   desktop_baton_guest.sh wait-desktop [SECONDS]      wait for cinnamon + a usable X display
#   desktop_baton_guest.sh build                       gcc glwin.c -> ./glwin (next to this script)
#   desktop_baton_guest.sh calibrate K_MS              print the layer count for ~K_MS ms frames
#   desktop_baton_guest.sh run MODE DURATION OUTDIR LAYERS [HEAVY_LAYERS]
#        MODE: solo (A) | pair (A,B) | canary (A,B + small vsynced window C) | heavylight (A light, B heavy)
#
# Windows are placed side by side (A left half, B right half of the screen, C bottom-left corner), never
# overlapping, so a compositor cannot skip an occluded one.  Everything the phase writes goes to OUTDIR:
# <label>.log/.csv/.err (glwin format), windows.txt (xwininfo), nvsmi.csv (1 Hz), dmesg.txt.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export DISPLAY=${DISPLAY:-:0} XAUTHORITY=${XAUTHORITY:-$HOME/.Xauthority}
GL="$HERE/glwin"
cmd=${1:-}; shift || true

screen_wh() { xdpyinfo 2>/dev/null | sed -n 's/^ *dimensions: *\([0-9]*\)x\([0-9]*\) pixels.*/\1 \2/p' | head -1; }

case "$cmd" in
info)
    echo "kernel=$(uname -r) driver=$(cat /proc/driver/nvidia/version 2>/dev/null | head -1 | cut -c1-120)"
    echo "screen=$(screen_wh)"
    glxinfo -B 2>&1 | grep -E 'OpenGL (vendor|renderer|version)' || echo "glxinfo: none"
    echo "composite_ext=$(xdpyinfo -ext all 2>/dev/null | grep -ci composite)"
    echo "cm_selection=$(xprop -root _NET_WM_CM_S0 2>&1 | head -1)"
    echo "wm=$(xprop -root _NET_SUPPORTING_WM_CHECK 2>&1 | head -1)"
    pgrep -a -x cinnamon | head -2; pgrep -a -x muffin | head -2
    pgrep -a -x Xorg | head -1 | cut -c1-120
    ;;
wait-desktop)
    secs=${1:-180}
    for _ in $(seq "$secs"); do
        if pgrep -x cinnamon >/dev/null && xdpyinfo >/dev/null 2>&1; then sleep 5; echo "DESKTOP_UP screen=$(screen_wh)"; exit 0; fi
        sleep 1
    done
    echo "DESKTOP_NOT_UP after ${secs}s"; exit 1
    ;;
build)
    gcc -O2 -Wall -o "$GL" "$HERE/glwin.c" -ldl -lm && echo "BUILD_OK $(gcc --version | head -1)"
    ;;
calibrate)
    read -r SW SH < <(screen_wh)
    "$GL" --calibrate-only --target-ms "${1:-5}" --w $(( (${SW:-1920} - 30) / 2 )) --h 700 --require-renderer NVIDIA
    ;;
run)
    MODE=${1:?mode}; DUR=${2:?duration}; OUT=${3:?outdir}; LAY=${4:?layers}; HLAY=${5:-$(( ${4} * 10 ))}
    mkdir -p "$OUT"
    read -r SW SH < <(screen_wh); SW=${SW:-1920}; SH=${SH:-1080}
    WW=$(( (SW - 30) / 2 )); WH=700; [ $(( SH - 200 )) -lt $WH ] && WH=$(( SH - 200 ))
    START=$(python3 -c 'import time; print("%.3f" % (time.time() + 8))')
    XID0=$(sudo dmesg 2>/dev/null | grep -c 'Xid'); FLIP0=$(sudo dmesg 2>/dev/null | grep -c 'Flip event timeout')
    PIDS=()
    start() {  # label x y w h layers [extra args]
        local l=$1 x=$2 y=$3 w=$4 h=$5 n=$6; shift 6
        "$GL" --label "$l" --duration "$DUR" --start-at "$START" --x "$x" --y "$y" --w "$w" --h "$h" \
              --layers "$n" --vsync 0 --require-renderer NVIDIA --dump-all "$OUT/$l.csv" "$@" \
              > "$OUT/$l.log" 2> "$OUT/$l.err" &
        PIDS+=($!)
    }
    case "$MODE" in
    solo)       start A 10 40 "$WW" "$WH" "$LAY" ;;
    pair)       start A 10 40 "$WW" "$WH" "$LAY"; start B $(( WW + 20 )) 40 "$WW" "$WH" "$LAY" ;;
    canary)     start A 10 40 "$WW" "$WH" "$LAY"; start B $(( WW + 20 )) 40 "$WW" "$WH" "$LAY"
                start C 10 $(( WH + 60 )) 240 160 1 --canary ;;
    heavylight) start A 10 40 "$WW" "$WH" "$LAY"; start B $(( WW + 20 )) 40 "$WW" "$WH" "$HLAY" ;;
    *) echo "bad mode $MODE" >&2; exit 2 ;;
    esac
    echo "PHASE_START mode=$MODE dur=$DUR layers=$LAY screen=${SW}x${SH} win=${WW}x${WH} start_at=$START pids=${PIDS[*]} $(date -Is)"
    if command -v nvidia-smi >/dev/null; then
        nvidia-smi --query-gpu=timestamp,utilization.gpu,clocks.sm,temperature.gpu --format=csv,noheader -l 1 \
            > "$OUT/nvsmi.csv" 2>/dev/null &
        SMI=$!
    fi
    sleep 14   # windows mapped and exposed (start-at is +8 s, frames flow after)
    { xwininfo -root -tree 2>&1 | grep -E 'glwin|kayfabe' ; xprop -root _NET_ACTIVE_WINDOW 2>&1; } > "$OUT/windows.txt"
    cat "$OUT/windows.txt" | sed 's/^/WINDOWS /'
    # a bound, as run_baton.sh: a client that hangs (a real stall) must not hang the phase forever
    end=$(( $(date +%s) + DUR + 60 )); ABORT=0
    while :; do
        alive=0; for p in "${PIDS[@]}"; do kill -0 "$p" 2>/dev/null && alive=1; done
        [ $alive = 0 ] && break
        if [ "$(date +%s)" -gt "$end" ]; then
            echo "PHASE_DEADLINE: SIGTERM to own glwin pids ${PIDS[*]}"; kill -TERM "${PIDS[@]}" 2>/dev/null; ABORT=1; sleep 5; break
        fi
        sleep 1
    done
    wait "${PIDS[@]}" 2>/dev/null
    [ -n "${SMI:-}" ] && kill "$SMI" 2>/dev/null
    sudo dmesg 2>/dev/null | grep -E 'Xid|Flip event timeout|nvAssert' | tail -20 > "$OUT/dmesg.txt"
    echo "PHASE_END mode=$MODE abort=$ABORT guest_xid_delta=$(( $(sudo dmesg 2>/dev/null | grep -c 'Xid') - XID0 )) flip_timeout_delta=$(( $(sudo dmesg 2>/dev/null | grep -c 'Flip event timeout') - FLIP0 )) $(date -Is)"
    for f in "$OUT"/*.err; do [ -s "$f" ] && { echo "ERR $f:"; head -3 "$f"; }; done
    ;;
*) echo "usage: $0 info|wait-desktop|build|calibrate|run" >&2; exit 2 ;;
esac
