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
# ⊘ `[measured m2a/m2b]` gq's `tr` BLOCK-buffers into a file: show.log stayed empty until the probe
# EXITED, so "wait for SHOWING=A, then screendump" grabbed the frame AFTER the probe restored fbcon —
# while the trace proved every copy of A and B pixel-exact. A line-buffered twin for the one step
# whose output is read while it runs.
gql(){ timeout "${2:-60}" "$G" "$1" 2>&1 | stdbuf -oL tr -d '\r'; }

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

# a host screendump of the kf3 console to $1 (PPM); prints the monitor's last line
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

# 3. set a mode from a known pattern, flip, and grade the host's screendump
HOLD=${DISPLAY_HOLD_S:-20}; FLIPS=${DISPLAY_FLIPS:-120}
( gql "sudo ~/display/kfdisp_probe show $card $HOLD $FLIPS" $((HOLD + 60)) > "$OUT/show.log" ) &
SP=$!
# wait until the guest says it is showing A (or the show fails), then capture
for i in $(seq 1 60); do grep -q '^KFDISP_SHOWING=A\|^KFDISP_FAIL' "$OUT/show.log" 2>/dev/null && break; sleep 1; done
mode=$(sed -n 's/^KFDISP_MODE \([0-9]*\)x\([0-9]*\)@.*/\1 \2/p' "$OUT/show.log")
# ⊘ `screendump … kf0` on a device without a graphic console ABORTS QEMU (`[measured m1b]`
# `Unexpected error in object_property_find_err()`): only when the device registered its console.
HAS_CONSOLE=no; grep -q 'kf3: display console registered' "$BENCH/run_${TAG}_qemu.log" 2>/dev/null && HAS_CONSOLE=yes
if grep -q '^KFDISP_SHOWING=A' "$OUT/show.log" && [ -n "$mode" ] && [ -S "$MON" ] && [ "$HAS_CONSOLE" = yes ]; then
    sleep 1   # one more vblank at least, so the scanout copy of A is the latest frame
    grep -q '^KFDISP_RESTORED' "$OUT/show.log" && say "SCREENDUMP_LATE (the probe already restored its CRTC)"
    shot "$OUT/screendump.ppm"
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

# 4. ★ M3 (DISPLAY_DESKTOP=1): the desktop on the virtual monitor — Xorg with the stock NVIDIA X
#    driver, a Cinnamon session through lightdm's autologin, then GL and Vulkan clients in it; graded
#    by the HOST's screendumps of the kf3 console (what the display engine scanned out), the X
#    server's own screenshot for comparison, and the clients' own words (renderer, device, fps).
if [ "${DISPLAY_DESKTOP:-0}" = 1 ] && [ "$HAS_CONSOLE" = yes ] && [ -S "$MON" ]; then
    bdf=$(gq "lspci -D -d 10de: | awk 'NR==1{print \$1}'")
    IFS=':.' read -r _ b d f <<< "$bdf"
    busid=$(printf 'PCI:%d:%d:%d' "0x${b:-0}" "0x${d:-0}" "0x${f:-0}")
    gq 'ls /usr/share/xsessions/' > "$OUT/xsessions.log"
    session=$(sed -n 's/^\(cinnamon[a-z0-9-]*\)\.desktop$/\1/p' "$OUT/xsessions.log" | head -1)
    session=${DISPLAY_SESSION:-${session:-cinnamon}}
    DESK=$(mktemp -d); trap 'rm -rf "$DESK"' EXIT
    # an experiment's session environment (e.g. __GL_SYNC_TO_VBLANK=0), via the user's ~/.xsessionrc
    if [ -n "${DISPLAY_SESSION_ENV:-}" ]; then echo "export $DISPLAY_SESSION_ENV" > "$DESK/xsessionrc"; else : > "$DESK/xsessionrc"; fi
    sed "s/@BUSID@/$busid/" "$HERE/desktop/xorg.conf.in" > "$DESK/xorg.conf"
    sed "s/@SESSION@/$session/g" "$HERE/desktop/50-kf-autologin.conf.in" > "$DESK/50-kf-autologin.conf"
    tar -C "$DESK" -cf - xorg.conf 50-kf-autologin.conf xsessionrc | $G 'rm -rf ~/desk && mkdir -p ~/desk && tar -xf - -C ~/desk'
    gq 'sudo cp ~/desk/xorg.conf /etc/X11/xorg.conf && sudo mkdir -p /etc/lightdm/lightdm.conf.d && sudo cp ~/desk/50-kf-autologin.conf /etc/lightdm/lightdm.conf.d/ && cp ~/desk/xsessionrc ~/.xsessionrc && echo DESK_CONF_OK' > "$OUT/desk_conf.log"
    say "DESKTOP_CONF busid=$busid session=$session env=[${DISPLAY_SESSION_ENV:-}] $(tr '\n' ' ' < "$OUT/desk_conf.log")"
    gq 'sudo systemctl start lightdm; echo rc=$?' 60 > "$OUT/lightdm_start.log"
    # the session: Xorg up, then a Cinnamon process of the autologin user (≤ 90 s)
    XENV='sudo -u ubuntu env DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority'
    # ⊘ `[measured m3a]` a process check reads a crash-looping X as "up" (lightdm restarts it): the
    # server must ANSWER a client, as the session user, and the session must be running
    up=no
    for i in $(seq 1 45); do
        if gq "$XENV xset q >/dev/null 2>&1 && pgrep -u ubuntu -x cinnamon >/dev/null && echo UP" | grep -q UP; then up=yes; break; fi
        sleep 2
    done
    say "DESKTOP_SESSION=$up ($(tr '\n' ' ' < "$OUT/lightdm_start.log") xorg_starts=$(gq 'sudo grep -c "X.Org X Server" /var/log/Xorg.0.log.old /var/log/Xorg.0.log 2>/dev/null | tr "\n" " "'))"
    sleep 20   # let the session paint (panel, wallpaper) before the first shot
    shot "$OUT/desk_1.ppm"
    gq "$XENV glxinfo -B 2>&1 | head -40" 60 > "$OUT/glxinfo.log"
    say "GLX_RENDERER $(grep -m1 'OpenGL renderer string' "$OUT/glxinfo.log" | cut -d: -f2- | sed 's/^ *//') DIRECT=$(grep -m1 'direct rendering' "$OUT/glxinfo.log" | cut -d: -f2 | tr -d ' ')"
    gq "$XENV timeout 15 glxgears -info 2>&1 | tail -8" 30 > "$OUT/glxgears.log"
    say "GLXGEARS $(grep 'frames in' "$OUT/glxgears.log" | tail -2 | tr '\n' ' ')"
    gq "$XENV __GL_SYNC_TO_VBLANK=0 timeout 12 glxgears 2>&1 | tail -3" 30 > "$OUT/glxgears_novsync.log"
    say "GLXGEARS_NOVSYNC $(grep 'frames in' "$OUT/glxgears_novsync.log" | tail -1)"
    # Vulkan presentation in each present mode (0 IMMEDIATE, 1 MAILBOX, 2 FIFO): which ones the
    # guest's WSI can create and run on the virtual monitor
    for pm in 0 1 2; do
        gq "$XENV timeout 10 vkcube --c 240 --present_mode $pm 2>&1 | tail -4; echo RC=\${PIPESTATUS[0]}" 30 > "$OUT/vkcube_pm$pm.log"
        say "VKCUBE_PM$pm $(grep -m1 -o 'Assertion.*\|Selected GPU[^,]*' "$OUT/vkcube_pm$pm.log" | tail -1 | head -c 120) $(grep -m1 '^RC=' "$OUT/vkcube_pm$pm.log")"
    done
    ( gq "$XENV timeout 25 vkcube --c 1200 2>&1 | tail -20; echo VKCUBE_RC=\${PIPESTATUS[0]}" 45 > "$OUT/vkcube.log" ) &
    VP=$!
    sleep 8
    shot "$OUT/desk_vkcube.ppm"
    wait $VP
    say "VKCUBE $(grep -m1 -i 'selected\|gpu\|device' "$OUT/vkcube.log" | head -c 160) $(grep VKCUBE_RC "$OUT/vkcube.log")"
    sleep 3
    shot "$OUT/desk_2.ppm"
    # the X server's own view of the root window, taken right after the host's second shot
    gq "$XENV import -window root /tmp/xroot.png && echo IMPORT_OK" 60 > "$OUT/xroot.log"
    $G 'cat /tmp/xroot.png' > "$OUT/xroot.png" 2>/dev/null
    gq 'cat /var/log/Xorg.0.log' 60 > "$OUT/Xorg.0.log"
    gq 'tail -120 /home/ubuntu/.xsession-errors 2>&1' 60 > "$OUT/xsession-errors.log"
    gq "$XENV vulkaninfo --summary 2>&1 | head -60" 60 > "$OUT/vulkaninfo.log"
    gq 'sudo journalctl -b -u lightdm --no-pager | tail -60' 60 > "$OUT/lightdm_journal.log"
    say "XORG_LOG errors=$(grep -c '(EE)' "$OUT/Xorg.0.log") nvidia=$(grep -c 'NVIDIA(0)' "$OUT/Xorg.0.log") $(grep -m1 'NVIDIA(0): Setting mode' "$OUT/Xorg.0.log" | cut -c1-120)"
    for f in desk_1 desk_vkcube desk_2; do
        [ -s "$OUT/$f.ppm" ] && say "SHOT $f md5=$(md5sum < "$OUT/$f.ppm" | cut -c1-12) bytes=$(stat -c %s "$OUT/$f.ppm")" || say "SHOT $f absent"
    done
    say "DESKTOP_GPU_PROGRESS_ERRORS=$(gq 'sudo dmesg | grep -c "waiting for GPU progress"') XID=$(gq 'sudo dmesg | grep -c "Xid"')"
    # why a session component died (`[measured m3b-m3d]` Cinnamon falls back right after it starts)
    gq 'sudo dmesg | grep -i "segfault\|traps:\|general protection" | tail -20; coredumpctl --no-pager list 2>/dev/null | tail -10' 60 > "$OUT/crashes.log"
    say "DESKTOP_CRASHES $(grep -c 'segfault\|traps:' "$OUT/crashes.log") $(grep -m1 -o '[a-z-]*\[[0-9]*\]: segfault.* in [^ ]*' "$OUT/crashes.log" | head -c 160)"
    gq 'sudo systemctl stop lightdm; echo rc=$?' 60 > "$OUT/lightdm_stop.log"
fi

# 4b. ★ M3 Cinnamon on Wayland (DISPLAY_CINNAMON_WAYLAND=1): the Mint desktop's own compositor
#     (muffin) driving KMS directly — no X driver, so no display-SW object — through lightdm's
#     autologin into the `cinnamon-wayland` session, then a Vulkan client in it; host screendumps
if [ "${DISPLAY_CINNAMON_WAYLAND:-0}" = 1 ] && [ "$HAS_CONSOLE" = yes ] && [ -S "$MON" ]; then
    gq 'ls /usr/share/wayland-sessions/ 2>&1' > "$OUT/wayland_sessions.log"
    wsession=$(sed -n 's/^\(cinnamon[a-z0-9-]*\)\.desktop$/\1/p' "$OUT/wayland_sessions.log" | head -1)
    say "CINNAMON_WAYLAND_SESSION=[${wsession:-none}] ($(tr '\n' ' ' < "$OUT/wayland_sessions.log"))"
    if [ -n "$wsession" ]; then
        CW=$(mktemp -d)
        sed "s/@SESSION@/$wsession/g" "$HERE/desktop/50-kf-autologin.conf.in" > "$CW/50-kf-autologin.conf"
        tar -C "$CW" -cf - 50-kf-autologin.conf | $G 'rm -rf ~/cw && mkdir -p ~/cw && tar -xf - -C ~/cw'
        rm -rf "$CW"
        gq 'sudo systemctl stop lightdm 2>/dev/null; sudo rm -f /etc/X11/xorg.conf; sudo mkdir -p /etc/lightdm/lightdm.conf.d && sudo cp ~/cw/50-kf-autologin.conf /etc/lightdm/lightdm.conf.d/ && sudo systemctl start lightdm; echo rc=$?' 60 > "$OUT/cw_start.log"
        up=no
        for i in $(seq 1 45); do
            if gq 'pgrep -u ubuntu -x cinnamon >/dev/null && ls /run/user/1000/wayland-* >/dev/null 2>&1 && echo UP' | grep -q UP; then up=yes; break; fi
            sleep 2
        done
        sleep 15
        wd=$(gq 'ls /run/user/1000/ | grep -m1 "^wayland-[0-9]*$"')
        say "CINNAMON_WAYLAND up=$up socket=[${wd}] $(tr '\n' ' ' < "$OUT/cw_start.log")"
        shot "$OUT/cw_1.ppm"
        CWENV="sudo -u ubuntu env XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=${wd:-wayland-0}"
        ( gq "$CWENV timeout 12 vkcube-wayland --c 400 2>&1 | tail -6; echo RC=\${PIPESTATUS[0]}" 40 > "$OUT/cw_vkcube.log" ) &
        VP=$!; sleep 6; shot "$OUT/cw_vkcube.ppm"; wait $VP
        say "CINNAMON_WAYLAND_VKCUBE $(grep -m1 -o 'Assertion.*\|Selected GPU[^,]*' "$OUT/cw_vkcube.log" | tail -1 | head -c 120) $(grep -m1 '^RC=' "$OUT/cw_vkcube.log")"
        gq 'sudo dmesg | grep -i "segfault\|traps:" | tail -10; tail -60 /home/ubuntu/.xsession-errors 2>/dev/null; sudo journalctl -b -u lightdm --no-pager | tail -30' 60 > "$OUT/cw_errors.log"
        say "CINNAMON_WAYLAND_CRASHES $(grep -c 'segfault\|traps:' "$OUT/cw_errors.log")"
        for f in cw_1 cw_vkcube; do
            [ -s "$OUT/$f.ppm" ] && say "SHOT $f md5=$(md5sum < "$OUT/$f.ppm" | cut -c1-12)" || say "SHOT $f absent"
        done
        gq 'sudo systemctl stop lightdm; echo rc=$?' 60 > /dev/null
    fi
fi

# 5. ★ M3 Wayland (DISPLAY_WESTON=1): with no X server holding the head —
#    weston on the DRM backend (nvidia-drm KMS -> the virtual engine)
#    with Vulkan and EGL clients presenting through it; graded by host screendumps
if [ "${DISPLAY_WESTON:-0}" = 1 ] && [ "$HAS_CONSOLE" = yes ] && [ -S "$MON" ]; then
    gq 'sudo systemctl stop lightdm 2>/dev/null; sleep 2; echo ok' 30 > /dev/null
    # (Ubuntu's vkcube is built without VK_KHR_display — `[measured m3f]` it has no --wsi option)
    gq "sudo rm -rf /run/kfw && sudo mkdir -m 700 /run/kfw && sudo sh -c 'XDG_RUNTIME_DIR=/run/kfw LIBSEAT_BACKEND=builtin nohup weston --backend=drm --continue-without-input --socket=kfw --log=/tmp/weston.log >/dev/null 2>&1 &' && echo started" 30 > "$OUT/weston_start.log"
    sleep 10
    WENV='sudo env XDG_RUNTIME_DIR=/run/kfw WAYLAND_DISPLAY=kfw'
    say "WESTON $(tr '\n' ' ' < "$OUT/weston_start.log") alive=$(gq 'pgrep -x weston >/dev/null && echo yes || echo no')"
    shot "$OUT/weston_1.ppm"
    ( gq "$WENV timeout 12 vkcube-wayland --c 400 2>&1 | tail -6; echo RC=\${PIPESTATUS[0]}" 40 > "$OUT/vkcube_wayland.log" ) &
    VP=$!; sleep 6; shot "$OUT/weston_vkcube.ppm"; wait $VP
    say "VKCUBE_WAYLAND $(grep -m1 -o 'Assertion.*\|Selected GPU[^,]*' "$OUT/vkcube_wayland.log" | tail -1 | head -c 120) $(grep -m1 '^RC=' "$OUT/vkcube_wayland.log")"
    ( gq "$WENV timeout 10 weston-simple-egl 2>&1 | tail -4; echo RC=\${PIPESTATUS[0]}" 30 > "$OUT/simple_egl.log" ) &
    VP=$!; sleep 5; shot "$OUT/weston_egl.ppm"; wait $VP
    say "SIMPLE_EGL $(tail -2 "$OUT/simple_egl.log" | tr '\n' ' ')"
    gq 'sudo tail -120 /tmp/weston.log' 30 > "$OUT/weston.log"
    gq 'sudo pkill -x weston; echo ok' 30 > /dev/null
    for f in weston_1 weston_vkcube weston_egl; do
        [ -s "$OUT/$f.ppm" ] && say "SHOT $f md5=$(md5sum < "$OUT/$f.ppm" | cut -c1-12)" || say "SHOT $f absent"
    done
    say "WESTON_GPU_PROGRESS_ERRORS=$(gq 'sudo dmesg | grep -c "waiting for GPU progress"')"
fi
say "HOOK_DONE"
