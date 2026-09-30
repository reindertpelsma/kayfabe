#!/usr/bin/env bash
# hook.sh <tag> — POST_CAPTURE_HOOK for the display lane (boot_capture.sh, KF_DEVICE=kf3,
# docs/design/V3_DISPLAY.md §5). Runs on the HOST with the guest up and `nvidia` loaded.
#   1. loads nvidia-drm modeset=1 (fbdev on) and records what the guest's KMS device looks like
#   2. kfdisp_probe list  → the connectors / planes the guest's stock driver built
#   3. kfdisp_probe show  → a mode set from a dumb buffer holding a pure-function pattern, flips timed;
#      while the guest holds pattern A the HOST takes a QEMU `screendump` and compares it with the
#      reference PPM the same source emits on the host (pixel-exact or it is a FAIL)
# Every result is one DISPLAY_* line on stdout (boot_capture.sh appends it to the probe log); the
# artefacts go to $DISPLAY_RES_DIR (default /workspace/bench/display/<tag>/).
# ⊘ No step may hang: every guest command has a deadline, and a missing line is reported as missing.
set -uo pipefail
TAG=${1:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"; G="$HERE/../gssh_nv"
BENCH=${BENCH_DIR:-/workspace/bench}
OUT=${DISPLAY_RES_DIR:-$BENCH/display/$TAG}; mkdir -p "$OUT"
MON=$BENCH/run_${TAG}.mon
say(){ echo "DISPLAY_$*"; }
gq(){ timeout "${2:-60}" "$G" "$1" 2>&1 | tr -d '\r'; }

# the probe ships at run time (a harness fix never needs a re-provisioned image)
tar -C "$HERE" -cf - kfdisp_probe.c | $G 'mkdir -p ~/display && tar -xf - -C ~/display' \
  && gq 'gcc -O2 -Wall -o ~/display/kfdisp_probe ~/display/kfdisp_probe.c $(pkg-config --cflags --libs libdrm) && echo PROBE_BUILD_OK' 120 > "$OUT/probe_build.log"
grep -q PROBE_BUILD_OK "$OUT/probe_build.log" && say "PROBE_BUILD=ok" || say "PROBE_BUILD=FAIL"

# 1. nvidia-drm with modeset=1 (the display plane's consumer); dmesg slice kept
d0=$(gq 'sudo dmesg | wc -l'); d0=${d0:-0}
gq 'sudo modprobe nvidia-drm modeset=1 fbdev=1; echo rc=$?' 90 > "$OUT/modprobe.log"
sleep 3
gq "sudo dmesg | tail -n +$((d0 + 1))" > "$OUT/drm_dmesg.log"
say "DRM_MODPROBE $(tr '\n' ' ' < "$OUT/modprobe.log")"
say "DRM_NODES $(gq 'ls /dev/dri 2>&1 | tr "\n" " "')"
say "DRM_DMESG_LINES=$(wc -l < "$OUT/drm_dmesg.log") displayless=$(grep -c -i 'displayless\|No display hardware\|Cannot find any crtc' "$OUT/drm_dmesg.log")"

# 2. the KMS device the stock driver built
card=$(gq 'for c in /sys/class/drm/card[0-9]*; do [ -e "$c/device/driver" ] && basename "$(readlink -f $c/device/driver)" | grep -qx nvidia && { echo /dev/dri/$(basename $c); break; }; done')
card=${card:-/dev/dri/card0}
gq "sudo ~/display/kfdisp_probe list $card" > "$OUT/list.log"
grep '^KFDISP_' "$OUT/list.log" | sed 's/^/DISPLAY_/'
# ★ M1 evidence (V3_DISPLAY.md §5): the stock tools' own view of the KMS device, and the guest's
# display state as nvidia-smi reports it; the GPU-progress wait errors counted (the m1 grade needs 0)
gq "sudo modetest -M nvidia-drm -c 2>&1 | head -60" 60 > "$OUT/modetest_c.log"
gq "sudo modetest -M nvidia-drm -p 2>&1 | head -80" 60 > "$OUT/modetest_p.log"
gq "nvidia-smi -q 2>&1 | grep -iA2 'display' | head -20" 60 > "$OUT/smi_display.log"
say "MODETEST_CONNECTED=$(grep -c '[[:space:]]connected[[:space:]]' "$OUT/modetest_c.log") modes_1080p=$(grep -c '1920x1080' "$OUT/modetest_c.log")"
say "SMI_DISPLAY $(tr '\n' ' ' < "$OUT/smi_display.log" | tr -s ' ' | head -c 300)"
say "GPU_PROGRESS_ERRORS=$(gq 'sudo dmesg | grep -c "waiting for GPU progress"')"
grep -q '^KFDISP_SUMMARY connected=[1-9]' "$OUT/list.log" && say "CONNECTED=yes card=$card" || { say "CONNECTED=no card=$card"; exit 0; }

# 3. set a mode from a known pattern, flip, and grade the host's screendump
HOLD=${DISPLAY_HOLD_S:-20}; FLIPS=${DISPLAY_FLIPS:-120}
( gq "sudo ~/display/kfdisp_probe show $card $HOLD $FLIPS" $((HOLD + 60)) > "$OUT/show.log" ) &
SP=$!
# wait until the guest says it is showing A (or the show fails), then capture
for i in $(seq 1 60); do grep -q '^KFDISP_SHOWING=A\|^KFDISP_FAIL' "$OUT/show.log" 2>/dev/null && break; sleep 1; done
mode=$(sed -n 's/^KFDISP_MODE \([0-9]*\)x\([0-9]*\)@.*/\1 \2/p' "$OUT/show.log")
# ⊘ `screendump … kf0` on a device without a graphic console ABORTS QEMU (`[measured m1b]`
# `Unexpected error in object_property_find_err()`): only when the device registered its console.
HAS_CONSOLE=no; grep -q 'kf3: display console registered' "$BENCH/run_${TAG}_qemu.log" 2>/dev/null && HAS_CONSOLE=yes
if grep -q '^KFDISP_SHOWING=A' "$OUT/show.log" && [ -n "$mode" ] && [ -S "$MON" ] && [ "$HAS_CONSOLE" = yes ]; then
    sleep 1   # one more vblank at least, so the scanout copy of A is the latest frame
    python3 - "$MON" "$OUT/screendump.ppm" <<'PY'
import socket, sys, time
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.settimeout(10)
time.sleep(0.2); s.recv(65536)
s.sendall(("screendump %s kf0\n" % sys.argv[2]).encode()); time.sleep(2)
try: print(s.recv(65536).decode(errors="replace").strip().splitlines()[-1])
except Exception as e: print("recv:", e)
PY
    set -- $mode
    cc -O2 -DKFDISP_NO_DRM -o "$OUT/kfdisp_ppm" "$HERE/kfdisp_probe.c" 2>/dev/null \
      && "$OUT/kfdisp_ppm" ppm "$1" "$2" a > "$OUT/reference_a.ppm"
    if [ -s "$OUT/screendump.ppm" ] && [ -s "$OUT/reference_a.ppm" ]; then
        if cmp -s "$OUT/screendump.ppm" "$OUT/reference_a.ppm"; then say "PATTERN_MATCH=yes (pixel-exact ${1}x${2})"
        else say "PATTERN_MATCH=no screendump=$(head -c 20 "$OUT/screendump.ppm" | tr '\n' ' ') ref=$(md5sum < "$OUT/reference_a.ppm" | cut -c1-12) got=$(md5sum < "$OUT/screendump.ppm" | cut -c1-12)"; fi
    else say "PATTERN_MATCH=absent (screendump=$(stat -c %s "$OUT/screendump.ppm" 2>/dev/null || echo none))"; fi
else
    say "PATTERN_MATCH=not-run (showing=$(grep -c '^KFDISP_SHOWING=A' "$OUT/show.log") mode=[$mode] mon=$([ -S "$MON" ] && echo yes || echo no) console=$HAS_CONSOLE)"
fi
wait $SP
grep '^KFDISP_' "$OUT/show.log" | sed 's/^/DISPLAY_/'
# ★ after the probe exited (its restore + close are inside that exit): nvidia-drm's own complaints.
# A missing flip event is a timeout here; an event nobody expected is a WARN (`cut here`).
say "FLIP_EVENT_TIMEOUTS=$(gq 'sudo dmesg | grep -c "Flip event timeout"') DRM_WARNS=$(gq 'sudo dmesg | grep -c "cut here"')"
say "HOOK_DONE"
