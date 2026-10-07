#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# unload_hook.sh <tag> — box test B5 (docs/design/V3_DISPLAY.md §4.11.9, §4.11.13): what the screen
# shows as each display client goes away, on a guest booted with the GOP ROM. A POST_CAPTURE_HOOK for
# lane.sh, selected with DISPLAY_HOOK=unload_hook. It runs on the HOST with the guest up and nvidia.ko
# loaded (no nvidia-drm yet; boot_capture's nvidia-smi has come and gone, so the guest's RM has
# already initialised and torn the adapter down once). One DISPLAY_B5* line per arm; shots in $OUT.
#   (c)  nvidia.ko, no RM client: the firmware console (simpledrm on BAR1 [0, G)) must show new text
#        — the guest's RM gave BAR1 back to physical mode when its last client closed.
#   (c2) the same with an RM client holding /dev/nvidia0 (RM up, its console mapping at BAR1 VA 0),
#        then (c3) once more after that client closed (RM torn down a second time). (c2) is the
#        positive control: it passed with the defect too (box run d1, 2026-10-03).
#   (a)  a KMS compositor on the firmware framebuffer — Cinnamon on Wayland (muffin) on simpledrm,
#        nvidia-drm not loaded, NVIDIA's EGL holding RM up. ⊘ No Xorg and no NVKMS (no display
#        channel is ever allocated). After the session the text console must be back.
#   (a2) the NVIDIA X driver (an xorg.conf.d snippet naming it), modeset=0: X11 Cinnamon on Xorg on
#        NVKMS, which imports the firmware console and restores it when X — its last client —
#        closes; the text console must be back and keep updating (PRESERVE_HW).
#   (b)  nvidia-drm modeset=1 fbdev=1, fbcon unbound, nvidia-drm removed (nvidia-modeset stays):
#        black, as on bare metal — fbdev=1 made NVKMS drop the console surface
#        (`nvRmUnmapFbConsoleMemory`), so its restore has nothing to show and shuts the heads down.
# Every step logs the guest's uptime so it lines up with the guest dmesg and the QEMU log.
# ⊘ No step may hang: every guest command has a deadline. A missing line is reported as missing.
# ★ VERDICT (2026-10-03, the review of v3-gop-unload: "the B5 lane cannot fail"): each arm's observation
# is compared with its expectation by `judge`; DISPLAY_B5_VERDICT PASS|FAIL lists every arm, and the
# hook exits 1 on any mismatch, a skipped arm, or a missing line — lane.sh turns that into its rc.
# ⊘ STALE EVIDENCE (the same review; the class that misread the first (a) run): /var/log/Xorg.0.log
# survives a session, so an arm that started no X read the previous arm's log. Each X arm now moves
# the old log aside first and reads the file only if it is NEWER than the arm's start (mtime vs the
# guest clock), and every device-log check reads only the QEMU log lines written after the arm began.
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
nonblack(){ sed -n 's/.*nonblack=\([0-9]*\)\/1000.*/\1/p' <<<"$1" | head -1; }
# ★ judge <arm> <expected> <observed> <ok: yes|no> — one row of the verdict
VERDICTS=(); FAILS=0
judge(){
    local r=PASS; [ "$4" = yes ] || { r=FAIL; FAILS=$((FAILS + 1)); }
    VERDICTS+=("$1=$r")
    say "B5_JUDGE $1 $r expected=[$2] observed=[$3]"
}
finish(){
    say "B5_VERDICT $([ "$FAILS" -eq 0 ] && echo PASS || echo FAIL) arms=${#VERDICTS[@]} failed=$FAILS ${VERDICTS[*]}"
    [ "$FAILS" -eq 0 ]; exit $?
}
QLOG=$BENCH/run_${TAG}_qemu.log
qlines(){ wc -l < "$QLOG" 2>/dev/null || echo 0; }
# the device's own lines written since QEMU log line <n>
qsince(){ tail -n +"$(( $1 + 1 ))" "$QLOG" 2>/dev/null; }
# how many of them match <ERE>. ⊘ Never `qsince | grep -q` (measured 2026-10-03, box run b5g at
# 445367a8): under pipefail grep -q exits at the first match, tail takes SIGPIPE, and the pipeline
# FAILS on a match — (a2)'s PRESERVED line was present and judged absent. Count instead: grep -c reads
# everything.
qcount(){ local n; n=$(qsince "$1" | grep -a -c -E "$2"); echo "${n:-0}"; }
# write <n> lines tagged <tag> to tty1, then shoot <name>; prints "before=[..] after=[..] changed=.."
lines_and_shot(){
    local tag=$1 n=$2 name=$3 b a
    shot "$OUT/${name}_before.ppm" >/dev/null
    gq "for i in \$(seq 1 $n); do echo \"KF3-$tag line \$i\"; done | sudo tee /dev/tty1 >/dev/null; echo ok" 30 >/dev/null
    sleep 2; shot "$OUT/${name}_after.ppm" >/dev/null
    b=$(stats "$OUT/${name}_before.ppm"); a=$(stats "$OUT/${name}_after.ppm")
    echo "before=[$b] after=[$a] changed=$(changed "$b" "$a")"
}
HAS_CONSOLE=no; grep -q 'kf3: display console registered' "$QLOG" 2>/dev/null && HAS_CONSOLE=yes
say "B5_CONSOLE=$HAS_CONSOLE"
if [ "$HAS_CONSOLE" != yes ] || [ ! -S "$MON" ]; then
    say "B5_SKIPPED no kf3 graphic console or monitor socket"
    judge B5_CONSOLE "a kf3 graphic console and a monitor socket" "console=$HAS_CONSOLE mon=$([ -S "$MON" ] && echo yes || echo no)" no
    finish
fi
BDF=$(gq "lspci -D -d 10de: | awk 'NR==1{print \$1}'")
IFS=':.' read -r _ _b _d _f <<< "$BDF"
BUSID=$(printf 'PCI:%d:%d:%d' "0x${_b:-0}" "0x${_d:-0}" "0x${_f:-0}")

# (c) nvidia.ko only, no RM client: the console the ROM set up (simpledrm on BAR1 [0, G)) must show
#     40 new lines. On a real card RM returned BAR1 to physical mode when its last client closed.
say "B5C_MODULES $(gq 'lsmod | awk "/^nvidia/{print \$1}" | tr "\n" " "') fb=[$(gq 'cat /proc/fb | tr "\n" " "')]"
gq 'sudo chvt 1; echo ok' 20 >/dev/null
r=$(lines_and_shot B5C 40 b5c)
say "B5C_CUDA_ONLY_CONSOLE up=$(upt) $r (expected: changed=yes)"
judge B5C "changed=yes" "changed=${r##*changed=}" "$([ "${r##*changed=}" = yes ] && echo yes || echo no)"
# (c2) RM held up by a client: its console mapping at BAR1 VA 0 is live.
gq 'sudo rm -f /tmp/kf3hold.pid; sudo setsid sh -c "sleep 60 </dev/nvidia0 >/dev/null 2>&1 & echo \$! >/tmp/kf3hold.pid" </dev/null >/dev/null 2>&1; sleep 8; echo ok' 30 >/dev/null
holder=$(gq 'cat /tmp/kf3hold.pid 2>/dev/null || echo none'); r=$(lines_and_shot B5C2 20 b5c2)
say "B5C2_RM_HELD up=$(upt) holder=$holder $r (expected: changed=yes)"
judge B5C2 "a holder, changed=yes" "holder=$holder changed=${r##*changed=}" \
    "$([ "${r##*changed=}" = yes ] && [ -n "$holder" ] && [ "$holder" != none ] && echo yes || echo no)"
# (c3) the holder closes: RM tears the adapter down a second time.
gq 'sudo kill "$(cat /tmp/kf3hold.pid)" 2>/dev/null; sleep 6; echo ok' 30 >/dev/null
r=$(lines_and_shot B5C3 20 b5c3)
say "B5C3_AFTER_SECOND_TEARDOWN up=$(upt) $r (expected: changed=yes)"
judge B5C3 "changed=yes" "changed=${r##*changed=}" "$([ "${r##*changed=}" = yes ] && echo yes || echo no)"

# (a)/(a2) need a known session: the image's lightdm autologin conf is whatever the last lane left
#     (`[measured d1, 2026-10-03]` B3's Cinnamon-Wayland arm had left `cinnamon-wayland`, so the first
#     B5 run's "X" was muffin on simpledrm: no Xorg, no NVKMS). Saved here, restored at the end.
AUTOLOGIN=/etc/lightdm/lightdm.conf.d/50-kf-autologin.conf
gq "sudo rm -f /tmp/kf3-b5-autologin.bak; [ -e $AUTOLOGIN ] && sudo cp -a $AUTOLOGIN /tmp/kf3-b5-autologin.bak; echo ok" 20 >/dev/null
set_session(){
    sed "s/@SESSION@/$1/g" "$HERE/desktop/50-kf-autologin.conf.in" \
        | $G "sudo mkdir -p /etc/lightdm/lightdm.conf.d && sudo tee $AUTOLOGIN >/dev/null; echo ok" >/dev/null
}
WSESS=$(gq 'ls /usr/share/wayland-sessions/ 2>/dev/null' | sed -n 's/^\(cinnamon[a-z0-9-]*\)\.desktop$/\1/p' | head -1)
XSESS=$(gq 'ls /usr/share/xsessions/ 2>/dev/null' | sed -n 's/^\(cinnamon[a-z0-9-]*\)\.desktop$/\1/p' | head -1)
say "B5A_SESSIONS wayland=[${WSESS:-none}] x11=[${XSESS:-none}] drm_modeset=$(gq 'cat /sys/module/nvidia_drm/parameters/modeset 2>/dev/null || echo not-loaded') xorg_conf=[$(gq 'ls /etc/X11/xorg.conf /etc/X11/xorg.conf.d/ 2>&1 | tr "\n" " "')]"
XENV='sudo env DISPLAY=:0 XAUTHORITY=/var/run/lightdm/root/:0'
up_x11(){ for _ in $(seq 1 45); do
    if gq "pgrep -x Xorg >/dev/null && $XENV xset q >/dev/null 2>&1 && echo UP" 15 | grep -q UP; then echo yes; return; fi
    sleep 2; done; echo no; }
up_wayland(){ for _ in $(seq 1 45); do
    if gq 'pgrep -u ubuntu -x cinnamon >/dev/null && ls /run/user/1000/wayland-* >/dev/null 2>&1 && echo UP' 15 | grep -q UP; then echo yes; return; fi
    sleep 2; done; echo no; }
gone(){ for _ in $(seq 1 20); do gq 'pgrep -x Xorg >/dev/null || pgrep -u ubuntu -x cinnamon >/dev/null && echo RUNNING' 10 | grep -q RUNNING || break; sleep 1; done; }
a_arm(){  # a_arm <label> <shot prefix> <up check> <expect X: yes|no>
    local label=$1 p=$2 upcheck=$3 want_x=$4 t0 up epoch xlog xorg xdrv q0 r after cs ok n
    # ⊘ the previous arm's (or boot's) Xorg log goes aside; only a log newer than this arm is read
    epoch=$(gq 'date +%s' 15)
    gq "sudo mv -f /var/log/Xorg.0.log /var/log/Xorg.0.log.kf3-before-$p 2>/dev/null; echo ok" 20 >/dev/null
    q0=$(qlines)
    t0=$(upt); gq 'sudo systemctl start lightdm; echo rc=$?' 60 > "$OUT/${p}_start.log"
    up=$($upcheck); sleep 10; shot "$OUT/${p}_x.ppm" >/dev/null
    xlog=$(gq "f=/var/log/Xorg.0.log; if [ ! -e \$f ]; then echo absent; elif [ \$(stat -c %Y \$f) -ge $epoch ]; then echo fresh; else echo STALE; fi" 15)
    xdrv=""
    # nothing of an earlier session is copied as this arm's: no fresh log, no ${p}_Xorg.0.log
    if [ "$xlog" = fresh ]; then
        gq 'sudo cat /var/log/Xorg.0.log 2>/dev/null' 30 > "$OUT/${p}_Xorg.0.log"
        xdrv=$(grep -o -m3 -E '\((II|EE)\) (NVIDIA|modeset)\(0\): [^,]{0,60}' "$OUT/${p}_Xorg.0.log" | tr '\n' '|')
    fi
    xorg=$(gq 'pgrep -a -x Xorg | head -1')
    say "${label}_SESSION up=$up start_up=$t0 arm_epoch=$epoch $(tr '\n' ' ' < "$OUT/${p}_start.log") shot=[$(stats "$OUT/${p}_x.ppm")] xorg=[$xorg] xorg_log=$xlog x_driver=[$xdrv] modules=[$(gq 'lsmod | awk "/^nvidia/{print \$1}" | tr "\n" " "')]"
    if [ "$want_x" = yes ]; then
        ok=no; [ "$up" = yes ] && [ -n "$xorg" ] && [ "$xlog" = fresh ] && grep -q 'NVIDIA(0)' <<<"$xdrv" && ok=yes
        judge "${label}_SESSION" "up on Xorg with the NVIDIA X driver (a fresh Xorg.0.log)" "up=$up xorg=${xorg:+running} xorg_log=$xlog nvidia_driver=$(grep -q 'NVIDIA(0)' <<<"$xdrv" && echo yes || echo no)" "$ok"
    else
        ok=no; [ "$up" = yes ] && [ -z "$xorg" ] && [ "$xlog" != fresh ] && ok=yes
        judge "${label}_SESSION" "the Wayland session up, no Xorg (no fresh Xorg.0.log)" "up=$up xorg=${xorg:+running} xorg_log=$xlog" "$ok"
    fi
    t0=$(upt); gq 'sudo systemctl stop lightdm; echo rc=$?' 60 > "$OUT/${p}_stop.log"; gone
    gq 'sudo chvt 1; echo ok' 20 >/dev/null
    sleep 3; shot "$OUT/${p}_after.ppm" >/dev/null
    after=$(changed "$(stats "$OUT/${p}_x.ppm")" "$(stats "$OUT/${p}_after.ppm")")
    say "${label}_AFTER_SESSION stop_up=$t0 $(tr '\n' ' ' < "$OUT/${p}_stop.log") shot=[$(stats "$OUT/${p}_after.ppm")] changed_from_session=$after (expected: yes — the text console)"
    judge "${label}_AFTER_SESSION" "changed_from_session=yes (the text console is back)" "changed_from_session=$after" "$([ "$after" = yes ] && echo yes || echo no)"
    r=$(lines_and_shot "${label}" 10 "${p}_tty")
    say "${label}_CONSOLE_AFTER_SESSION up=$(upt) $r (expected: changed=yes)"
    judge "${label}_CONSOLE" "changed=yes (the console keeps updating after the session)" "changed=${r##*changed=}" "$([ "${r##*changed=}" = yes ] && echo yes || echo no)"
    # the device's own account of this arm: only the QEMU log lines written since it began
    cs=$(qsince "$q0" | grep -a 'the console shows' | sed 's/^kf3: display: //' | cut -c1-140 | tr '\n' '|' | cut -c1-900)
    say "${label}_DEVICE console_shows=[$cs]"
    if [ "$want_x" = yes ]; then
        # NVKMS restored the console and freed its channels with PRESERVE_HW: the scanout stays
        n=$(qcount "$q0" 'the console shows the PRESERVED scanout')
        judge "${label}_PRESERVED" "kf3 kept the console NVKMS restored (the PRESERVED scanout)" "lines=$n" "$([ "$n" -gt 0 ] && echo yes || echo no)"
    fi
}
# (a) a KMS compositor on the firmware framebuffer: Cinnamon on Wayland (muffin) drives simpledrm —
#     nvidia-drm is not loaded — and NVIDIA's EGL opens /dev/nvidia0 (RM up for the session's life).
if [ -n "$WSESS" ]; then
    gq 'sudo rm -f /etc/X11/xorg.conf; echo ok' 20 >/dev/null
    set_session "$WSESS"
    a_arm B5A b5a up_wayland no
else
    say "B5A_SKIPPED no cinnamon Wayland session on this image"
    judge B5A "a cinnamon Wayland session" "none on this image" no
fi
# (a2) the NVIDIA X driver, modeset=0 (nvidia-drm not loaded): X11 Cinnamon on Xorg on NVKMS. NVKMS
#      imports the firmware console and restores it when X — its last client — closes.
if [ -n "$XSESS" ]; then
    printf 'Section "Device"\n    Identifier "kf3-b5"\n    Driver "nvidia"\n    BusID "%s"\nEndSection\n' "$BUSID" \
        | $G 'sudo mkdir -p /etc/X11/xorg.conf.d && sudo tee /etc/X11/xorg.conf.d/90-kf3-b5-nvidia.conf >/dev/null; echo ok' >/dev/null
    set_session "$XSESS"
    a_arm B5A2 b5a2 up_x11 yes
    gq 'sudo rm -f /etc/X11/xorg.conf.d/90-kf3-b5-nvidia.conf; echo ok' 20 >/dev/null
else
    say "B5A2_SKIPPED no cinnamon X11 session on this image"
    judge B5A2 "a cinnamon X11 session" "none on this image" no
fi
gq "if [ -e /tmp/kf3-b5-autologin.bak ]; then sudo cp -a /tmp/kf3-b5-autologin.bak $AUTOLOGIN; else sudo rm -f $AUTOLOGIN; fi; echo ok" 20 >/dev/null
gq 'sudo dmesg | grep -i -E "nvidia-modeset|nvkms|console|fbcon" | tail -30' 30 > "$OUT/b5a_dmesg.log"

# (b) modeset=1 fbdev=1, then fbcon unbound and nvidia-drm removed (nvidia-modeset stays loaded).
gq 'sudo rmmod nvidia_drm 2>/dev/null; sudo modprobe nvidia-drm modeset=1 fbdev=1; echo rc=$?' 90 > "$OUT/b5b_load.log"
sleep 4; gq 'sudo chvt 1; echo "KF3-B5B console on nvidia-drm fbdev" | sudo tee /dev/tty1 >/dev/null' 20 >/dev/null
sleep 2; shot "$OUT/b5b_fbcon.ppm" >/dev/null
fb=$(gq 'cat /proc/fb | tr "\n" " "'); sf=$(stats "$OUT/b5b_fbcon.ppm")
say "B5B_FBCON up=$(upt) $(tr '\n' ' ' < "$OUT/b5b_load.log") fb=[$fb] shot=[$sf]"
ok=no; grep -q 'rc=0' "$OUT/b5b_load.log" && grep -q 'nvidia-drm' <<<"$fb" && [ "$(nonblack "$sf")" -gt 0 ] 2>/dev/null && ok=yes
judge B5B_FBCON "nvidia-drm fbdev loaded (rc=0, nvidia-drmdrmfb), fbcon text shown (nonblack>0)" "$(tr '\n' ' ' < "$OUT/b5b_load.log")fb=[$fb] nonblack=$(nonblack "$sf")" "$ok"
t0=$(upt); q0=$(qlines)
gq 'for v in /sys/class/vtconsole/vtcon*/bind; do echo 0 | sudo tee "$v" >/dev/null; done; echo unbound; cat /proc/uptime; sudo rmmod nvidia_drm; echo rc=$?; lsmod | grep -c "^nvidia_drm"' 90 > "$OUT/b5b_unload.log"
sleep 4; shot "$OUT/b5b_after.ppm" >/dev/null
sa=$(stats "$OUT/b5b_after.ppm")
say "B5B_AFTER_RMMOD up=$t0 $(tr '\n' ' ' < "$OUT/b5b_unload.log") shot=[$sa] modules=[$(gq 'lsmod | awk "/^nvidia/{print \$1}" | tr "\n" " "')] (expected: nonblack=0 — black, as bare metal)"
ok=no; grep -q '^rc=0' "$OUT/b5b_unload.log" && [ "$(nonblack "$sa")" = 0 ] && ok=yes
judge B5B_AFTER_RMMOD "rmmod rc=0, black (nonblack=0)" "$(grep -o '^rc=[0-9]*' "$OUT/b5b_unload.log" | head -1) nonblack=$(nonblack "$sa")" "$ok"
n=$(qcount "$q0" 'the console shows BLACK')
judge B5B_DEVICE "kf3 chose black (the console shows BLACK)" "lines=$n" "$([ "$n" -gt 0 ] && echo yes || echo no)"
gq 'sudo dmesg | tail -60' 30 > "$OUT/b5_dmesg_tail.log"
# the device's own account of each arm: what the console showed, BAR1's boot range, the teardowns
grep -a -E 'the console shows|BAR1 boot framebuffer|wrote NV_|BAR1_BLOCK|fn 47|boot display seed|physical view|armed its first head|ChannelFreed \{ kind: Core|ChannelAllocated \{ kind: Core|lines logged' \
    "$QLOG" > "$OUT/b5_device.log" 2>/dev/null
say "B5_DEVICE_LINES $(wc -l < "$OUT/b5_device.log")"
# ★ every RM teardown returned BAR1 [0, G) to its physical view: after each PHYSICAL write of the
#   BAR1-mode register, a "back to its physical view" or "already shows" line before the next write
#   (a first trigger "NOT restored" may be rescued by fn 47's), and no REFUSED
read -r nw nok nbad < <(awk '
    /MODE PHYSICAL/ { if (p) bad++; p = 1; w++ }
    /BAR1 back to its physical view|already shows its physical view/ { if (p) { ok++; p = 0 } }
    /physical view REFUSED/ { bad++ }
    END { if (p) bad++; print w + 0, ok + 0, bad + 0 }' "$OUT/b5_device.log")
ok=no; [ "$nw" -gt 0 ] && [ "$nok" -eq "$nw" ] && [ "$nbad" -eq 0 ] && ok=yes
judge B5_TEARDOWNS "each PHYSICAL write followed by BAR1 [0, G) shown again, none refused" "physical_writes=$nw restored=$nok unrestored_or_refused=$nbad" "$ok"
say "B5_HOOK_DONE"
finish
