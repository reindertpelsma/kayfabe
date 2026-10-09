#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# broker_lane.sh — the display BROKER lane on a box with a host desktop (docs/design/V3_DISPLAY.md
# §8.9 item 5, §8.11 E0-E5, §8.12 the hover cursor). Two subcommands, each run from this checkout:
#
#   broker_lane.sh prep <nvkvm-pv-rev>
#       E0 and the host side: apt deps for the broker's X11/Wayland backends and the evidence tools;
#       nvkvm-pv cloned from GitHub at <rev> into /root/nvkvm-pv and the broker built; nvidia-drm
#       reloaded with modeset=1 when it is not (the GPU-copy rung needs it; the host desktop must be
#       down); the host display manager started; waits for the session. Prints PREP_* lines.
#   broker_lane.sh run <tag>
#       the broker as the desktop session's user (X11 backend, --persist --verbose) on
#       /run/user/<uid>/nvkvm/display.sock; then boot_capture.sh with display=on,display-broker=…,
#       display-broker-uid=<uid> (+ $BRK_KF3_EXTRA), virtio keyboard/tablet/mouse, and
#       broker_hook.sh as the POST_CAPTURE_HOOK. Evidence: /workspace/bench/brk/<tag>/.
#
# env (run): BROKER_ARGS (extra broker flags, e.g. --present-mode=shm), BRK_KF3_EXTRA (extra kf3
#   properties, e.g. display-broker-vram=off), KF3_VRAM_ATTRS (s0|s1|s2, E1), BRK_DESKTOP (1: bring
#   up the guest's Cinnamon session; default 1), BRK_CURSOR (1: the cursor experiments; default 1),
#   BRK_VNC (1: QEMU's VNC for the console cursor, §8.13), BRK_DRI3 (1: the DRI3 refusal client
#   before the VMM connects, §8.15), BRK_RESILIENCE (1: E3), BRK_HOLD (seconds the hook holds the
#   guest up for manual checks; `touch <out>/release` ends it).
# Every claim cites the kf3 binary's revision (boot_capture's run_<tag>_rev.txt) and the broker's
# (PREP_BROKER_REV, /root/nvkvm-pv). A start marker and an EXIT line are written by this script.
set -uo pipefail
# The lane greps per-statement log lines (display fps, broker connects); production is quiet by default
# (docs/design/V3_NONSTALL_THREADS.md). KF3_LOG_VERBOSE=0 in the environment turns it off here too.
export KF3_LOG_VERBOSE="${KF3_LOG_VERBOSE-1}"
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../../.." && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
NVPV=${NVPV_DIR:-/root/nvkvm-pv}
# ⊘ [2026-10-03, box 54032077, run brkA] the broker runs as the desktop's user, who cannot
# traverse /root (mode 0700): "env: .../nvkvm-display-broker: Permission denied", and the relay
# retried an absent listener. prep installs the build world-readable here.
B=${BROKER_BIN:-/opt/nvkvm-broker/nvkvm-display-broker}
DMASRC=${DMASRC_BIN:-/opt/nvkvm-broker/nvkvm-broker-dmabuf-src}
cmd=${1:?usage: broker_lane.sh prep <nvkvm-pv-rev> | run <tag>}
echo "BRK_LANE_START $cmd ${2:-} kf=$(git -C "$REPO" rev-parse --short=8 HEAD) $(date -Is)"

# The desktop session's X display, cookie and user — from a process inside it (plasmashell, else
# the X server itself). Never exported into this shell: handed to the commands that need it.
session() {
    local p
    p=$(pgrep -o -x plasmashell 2>/dev/null)
    if [ -n "$p" ]; then
        sv() { tr '\0' '\n' < "/proc/$p/environ" | sed -n "s/^$1=//p"; }
        XD=$(sv DISPLAY); XA=$(sv XAUTHORITY); SU=$(stat -c %U "/proc/$p"); SUID=$(stat -c %u "/proc/$p")
        return 0
    fi
    return 1
}

# ⊘ [box 54032077, runs brkA2/brkA3, 2026-10-03] the KDE session locked itself while idle
# (kscreenlocker_greet, LockedHint=yes): the locker holds the keyboard and pointer, so CTRL+ALT+F/G
# and the pointer never reached the broker, and no ABS reached the guest — two runs that graded
# nothing. Unlocked, the broker's own test client saw every key and the grab toggle.
unlock() {
    local s
    for s in $(loginctl list-sessions --no-legend 2>/dev/null | awk -v u="${1:-user}" '$3 == u {print $1}'); do
        if [ "$(loginctl show-session "$s" -p LockedHint --value 2>/dev/null)" = yes ]; then
            loginctl unlock-session "$s"; echo "BRK_SESSION session=$s was LOCKED: unlocked"
        fi
    done
}
# every broker on <socket> — the runuser wrapper ignores SIGTERM ([run brkA3]: it outlived the lane)
brokers_down() {
    pkill -9 -f "^runuser -u [a-z0-9_-]* -- env .*nvkvm-display-broker --socket $1( |\$)" 2>/dev/null
    pkill -f "^[^ ]*nvkvm-display-broker --socket $1( |\$)" 2>/dev/null
    sleep 1
}

if [ "$cmd" = prep ]; then
    REV=${2:?prep needs the nvkvm-pv revision}
    export DEBIAN_FRONTEND=noninteractive
    apt-get install -y -q pkg-config libwayland-dev wayland-protocols libxcb1-dev libxcb-dri3-dev \
        libxcb-present-dev libxcb-render0-dev libxcb-xinput-dev libgbm-dev xdotool x11-apps \
        x11-utils imagemagick socat > "$BENCH/brk_prep_apt.log" 2>&1
    echo "PREP_APT_RC=$?"
    if [ ! -d "$NVPV/.git" ]; then
        git clone -q https://github.com/reindertpelsma/nvkvm-pv.git "$NVPV" || { echo "BRK_LANE_EXIT rc=10 clone"; exit 10; }
    fi
    git -C "$NVPV" fetch -q origin && git -C "$NVPV" checkout -q --detach "$REV" \
        || { echo "BRK_LANE_EXIT rc=11 checkout"; exit 11; }
    echo "PREP_BROKER_REV $(git -C "$NVPV" rev-parse HEAD) dirty=[$(git -C "$NVPV" status --porcelain --untracked-files=no | wc -l)]"
    make -C "$NVPV/src/broker" clean > /dev/null 2>&1
    make -C "$NVPV/src/broker" report nvkvm-display-broker nvkvm-broker-testclient > "$BENCH/brk_prep_make.log" 2>&1
    echo "PREP_BROKER_MAKE_RC=$? $(grep -A4 'backends:' "$BENCH/brk_prep_make.log" | tr -s ' ' | tr '\n' ' ' | cut -c1-300)"
    install -D -m 0755 "$NVPV/src/broker/nvkvm-display-broker" "$B" \
        || { echo "BRK_LANE_EXIT rc=12 no broker binary"; exit 12; }
    echo "PREP_BROKER_BIN $B sha256=$(sha256sum "$B" | cut -c1-16)"
    # nvkvm-pv's dma-buf test source (links libgbm; test tooling, never the broker) for BRK_DRI3
    make -C "$NVPV/src/broker" nvkvm-broker-dmabuf-src >> "$BENCH/brk_prep_make.log" 2>&1 \
        && install -D -m 0755 "$NVPV/src/broker/nvkvm-broker-dmabuf-src" "$DMASRC"
    echo "PREP_DMABUF_SRC rc=$? $([ -x "$DMASRC" ] && sha256sum "$DMASRC" | cut -c1-16)"
    # E0: the host driver, nvidia-drm's modeset, the render node
    echo "PREP_E0 driver=$(cat /sys/module/nvidia/version) drm=$(cat /sys/module/nvidia_drm/version) modeset=$(cat /sys/module/nvidia_drm/parameters/modeset) fbdev=$(cat /sys/module/nvidia_drm/parameters/fbdev 2>/dev/null)"
    echo "PREP_E0 nodes $(stat -c '%A %U %G %t:%T %n' /dev/dri/card* /dev/dri/renderD* 2>/dev/null | tr '\n' ';')"
    echo "PREP_E0 render_acl $(getfacl -p /dev/dri/renderD* 2>/dev/null | grep -v '^#' | tr '\n' ' ')"
    if [ "$(cat /sys/module/nvidia_drm/parameters/modeset)" != Y ]; then
        if pgrep -x Xorg >/dev/null; then
            echo "PREP_MODESET refused: Xorg holds nvidia_drm; stop the display manager first"
        else
            modprobe -r nvidia_drm && modprobe nvidia_drm modeset=1 fbdev=1
            echo "PREP_MODESET reloaded rc=$? modeset=$(cat /sys/module/nvidia_drm/parameters/modeset)"
        fi
    fi
    # the host desktop (§8.9 5a): start the real display-manager unit, never enable it
    dm=$(sort -u /root/prov/host_dm_stopped 2>/dev/null | head -1)
    [ "$dm" = display-manager ] || [ -z "$dm" ] && dm=$(basename "$(cat /etc/X11/default-display-manager 2>/dev/null)")
    systemctl start "$dm"; echo "PREP_DM $dm start_rc=$?"
    for _ in $(seq 120); do session && break; sleep 1; done
    if session; then
        echo "PREP_SESSION user=$SU uid=$SUID display=$XD xauth=$([ -n "$XA" ] && echo set || echo none)"
        # no screen lock and no blanking for the lane's session (see unlock above)
        db=$(tr '\0' '\n' < "/proc/$(pgrep -o -x plasmashell)/environ" | sed -n 's/^DBUS_SESSION_BUS_ADDRESS=//p')
        for k in Autolock LockOnResume; do
            runuser -u "$SU" -- env DBUS_SESSION_BUS_ADDRESS="$db" kwriteconfig5 --file kscreenlockerrc --group Daemon --key "$k" false
        done
        runuser -u "$SU" -- env DBUS_SESSION_BUS_ADDRESS="$db" qdbus org.freedesktop.ScreenSaver /ScreenSaver configure >/dev/null 2>&1
        env DISPLAY="$XD" XAUTHORITY="$XA" xset s off -dpms
        unlock "$SU"
        echo "PREP_SESSION_LOCK autolock=off $(grep -h Autolock "/home/$SU/.config/kscreenlockerrc" 2>/dev/null)"
        echo "PREP_XORG $(grep -aE 'NVIDIA GLX Module' /var/log/Xorg.0.log | tail -1 | cut -c1-80)"
        env DISPLAY="$XD" XAUTHORITY="$XA" xdpyinfo 2>/dev/null | grep -E 'dimensions|DRI3|Present' | head -4 | sed 's/^/PREP_XDPY /'
    else
        echo "PREP_SESSION none after 120 s (no plasmashell)"
    fi
    echo "BRK_LANE_EXIT rc=0 $(date -Is)"
    exit 0
fi

[ "$cmd" = run ] || { echo "BRK_LANE_EXIT rc=2 unknown subcommand $cmd"; exit 2; }
TAG=${2:?run needs a tag}
OUT=$BENCH/brk/$TAG; rm -rf "$OUT"; mkdir -p "$OUT"
session || { echo "BRK_LANE_EXIT rc=20 no desktop session (run prep)"; exit 20; }
[ -x "$B" ] || { echo "BRK_LANE_EXIT rc=21 no broker at $B (run prep)"; exit 21; }
echo "BRK_REVS kf3=$(git -C "$REPO" rev-parse HEAD) broker=$(git -C "$NVPV" rev-parse HEAD)"
SOCKD=/run/user/$SUID/nvkvm; SOCK=$SOCKD/display.sock
unlock "$SU"
echo "BRK_SESSION_LOCKED=$(loginctl show-session "$(loginctl list-sessions --no-legend | awk -v u="$SU" '$3 == u && $0 !~ /closing/ {print $1}' | tail -1)" -p LockedHint --value 2>/dev/null)"
brokers_down "$SOCK"
install -d -o "$SU" -m 0700 "$SOCKD"; rm -f "$SOCK"
# shellcheck disable=SC2086  # BROKER_ARGS is a flag list
runuser -u "$SU" -- env DISPLAY="$XD" XAUTHORITY="$XA" "$B" --socket "$SOCK" --backend x11 --persist \
    --verbose ${BROKER_ARGS:-} > "$OUT/broker.log" 2>&1 &
BPID=$!
for _ in $(seq 50); do [ -S "$SOCK" ] && break; sleep 0.2; done
echo "BRK_BROKER pid=$BPID socket=$([ -S "$SOCK" ] && echo up || echo MISSING) args=[${BROKER_ARGS:-}] bin=$B sha256=$(sha256sum "$B" | cut -c1-16)"
[ -S "$SOCK" ] || { echo "BRK_BROKER_LOG $(tail -3 "$OUT/broker.log" | tr '\n' ' ')"; echo "BRK_LANE_EXIT rc=22 no broker socket"; exit 22; }
# §8.15 (2026-10-04, opt-in BRK_DRI3=1): before the VMM connects, a real GPU dma-buf (nvkvm-pv's
# dmabuf-src, block-linear 0x0300000000606014 — the relay's own GPU-copy pair) is attached under
# malformed descriptors, one connection each, so the X server's DRI3 import refuses one — the
# broker's unsolicited EV_FORMAT x=0 for both alpha twins, then a new connection told yes again.
# The VMM connects after it, to the same --persist broker: its yes for that pair is the refusal
# not outliving its connection, end to end.
if [ "${BRK_DRI3:-0}" = 1 ]; then
    if [ -x "$DMASRC" ]; then
        SRCS=/run/kf-dri3-src.sock; rm -f "$SRCS"
        "$DMASRC" --serve "$SRCS" --size 512x512 --modifier 0x0300000000606014 > "$OUT/dri3_source.log" 2>&1 &
        SPID=$!
        for _ in $(seq 50); do [ -S "$SRCS" ] && break; sleep 0.2; done
        bl=$(wc -l < "$OUT/broker.log")
        timeout 120 python3 "$HERE/dri3_refusal.py" --broker "$SOCK" --source "$SRCS" --udmabuf \
            > "$OUT/dri3.log" 2>&1
        echo "BRK_DRI3_RC=$? source=[$(head -c 400 "$OUT/dri3_source.log" | tr '\n' ' ')]"
        sed 's/^/BRK_/' "$OUT/dri3.log"
        tail -n +"$(( bl + 1 ))" "$OUT/broker.log" | grep -aE 'DRI3|EV_FORMAT|refused|REJECTED|accepted uid|detached' \
            | cut -c1-260 | head -40 | sed 's/^/BRK_DRI3_BROKER /'
        kill "$SPID" 2>/dev/null; wait "$SPID" 2>/dev/null; rm -f "$SRCS"
    else
        echo "BRK_DRI3 SKIPPED: no $DMASRC (run prep)"
    fi
fi
export BRK_XD="$XD" BRK_XA="$XA" BRK_OUT="$OUT" BRK_BROKER_LOG="$OUT/broker.log"
# for the hook's E3 resilience step (BRK_RESILIENCE=1): who runs the broker, and how to start it again
export BRK_SU="$SU" BRK_SOCK="$SOCK" BRK_BIN="$B" BRK_BROKER_ARGS="${BROKER_ARGS:-}"
export NVKVM_RAM_MB=${NVKVM_RAM_MB:-8192} KF_SMP=${KF_SMP:-6}
export KF3_DEV_EXTRA="display=on,display-broker=$SOCK,display-broker-uid=$SUID${BRK_KF3_EXTRA:+,$BRK_KF3_EXTRA}"
export POST_CAPTURE_HOOK="$HERE/broker_hook.sh"
export PRE_POWEROFF_GUEST_CMD=${PRE_POWEROFF_GUEST_CMD:-"sudo systemctl stop lightdm; sudo sync; echo s | sudo tee /proc/sysrq-trigger >/dev/null; sleep 1; echo u | sudo tee /proc/sysrq-trigger >/dev/null; sleep 1"}
# §8.13 (2026-10-04, opt-in): BRK_VNC=1 gives QEMU a VNC server on 127.0.0.1:5907 for the hook's
# console-cursor checks (vnc_cursor.py); unset, the QEMU command line is as before
VNC_ARGS=()
if [ "${BRK_VNC:-}" = 1 ]; then
    VNC_ARGS=(-vnc 127.0.0.1:7); export BRK_VNC=127.0.0.1:5907
else
    unset BRK_VNC
fi
exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9
bash "$REPO/scripts/bench/boot_capture.sh" "$TAG" -- -vga none -device virtio-keyboard-pci \
    -device virtio-tablet-pci,display=kf0,head=0 -device virtio-mouse-pci "${VNC_ARGS[@]}"
rc=$?
kill "$BPID" 2>/dev/null
brokers_down "$SOCK"
echo "BRK_BROKERS_LEFT=$(pgrep -c -f "[n]vkvm-display-broker --socket $SOCK")"
for f in qemu probe dmesg dmesg_after hostdmesg rev; do
    s=$BENCH/run_${TAG}_$f.log; [ "$f" = rev ] && s=$BENCH/run_${TAG}_rev.txt
    [ -s "$s" ] && cp "$s" "$OUT/"
done
Q=$OUT/run_${TAG}_qemu.log
echo "BRK_RELAY $(grep -ac 'kf3: broker:' "$Q" 2>/dev/null) lines; rungs: $(grep -ao 'frames go as [^(;]*' "$Q" 2>/dev/null | sort | uniq -c | tr '\n' ' ')"
grep -aE 'kf3: broker: (GPU-copy rung|connected|the compositor|the display (CAN|CANNOT|imported)|guest cursor|grab|frames go)' "$Q" 2>/dev/null | cut -c1-230 | head -40 | sed 's/^/BRK_RELAY_LINE /'
grep -ao 'broker\[[^]]*\]' "$Q" 2>/dev/null | tail -1 | sed 's/^/BRK_STATUS /'
# §8.15 (the review of 2026-10-04): a GPU-copy run must refuse no descriptor and trip no dma-buf
# back-off. A status line WITHOUT the counters (a binary before them) is UNMEASURED, never a zero.
brk_st=$(grep -ao 'broker\[[^]]*\]' "$Q" 2>/dev/null | tail -1)
brk_n() { printf '%s\n' "$brk_st" | grep -o " $1=[0-9]*" | head -1 | cut -d= -f2; }
brk_gc=$(brk_n gpucopy); brk_cr=$(brk_n carrier_refused); brk_dt=$(brk_n dmabuf_trips)
if [ -z "$brk_st" ] || [ -z "$brk_cr" ] || [ -z "$brk_dt" ]; then
    echo "BRK_COUNTER_GRADE UNMEASURED (no relay status line, or one without carrier_refused/dmabuf_trips)"
elif [ "${brk_gc:-0}" -gt 0 ]; then
    if [ "$brk_cr" = 0 ] && [ "$brk_dt" = 0 ]; then brk_v=PASS; else brk_v=FAIL; fi
    echo "BRK_COUNTER_GRADE $brk_v gpucopy=$brk_gc carrier_refused=$brk_cr dmabuf_trips=$brk_dt carrier_unchecked=$(brk_n carrier_unchecked) formats_unasked=$(brk_n formats_unasked)"
else
    echo "BRK_COUNTER_GRADE NOT_GRADED (no GPU-copy frame: gpucopy=${brk_gc:-?}) carrier_refused=$brk_cr dmabuf_trips=$brk_dt"
fi
grep -ao 'host_cursor_reads=[0-9]* host_cursor_refused=[0-9]*\|scanout_d2h=[0-9]* scanout_pack=[0-9]* pack_skipped=[0-9]* display_vram_mib=[0-9]*' "$Q" 2>/dev/null | tail -2 | sed 's/^/BRK_DISP /'
grep -aE 'REFUSED|refused' "$Q" 2>/dev/null | grep -a broker | cut -c1-200 | head -8 | sed 's/^/BRK_REFUSED_LINE /'
# §8.14: the bring-up self-tests on the GPU (the compose kernel's includes the XOR pixel)
grep -aE 'kf3: (display|broker): .*self-test' "$Q" 2>/dev/null | cut -c1-200 | sort | uniq -c | sed 's/^/BRK_SELFTEST /'
# §8.15: every format verdict the relay recorded, and what the broker asked and answered per
# connection (a "no" must be asked again on each new connection: the refusal is connection state)
grep -aE 'kf3: broker: (the display (CAN|CANNOT) show|unasked EV_FORMAT|reclaimed frame slot|dma-buf frames are not|REFUSED frame slot|sent unchecked)' "$Q" 2>/dev/null \
    | cut -c1-230 | head -20 | sed 's/^/BRK_FORMAT_LINE /'
awk '/accepted uid/{n++} /QUERY_FORMAT|DRI3 pixmap import refused|EV_FORMAT x=0|is not advertised/{print "conn" n ": " $0}' "$OUT/broker.log" 2>/dev/null \
    | cut -c1-230 | head -30 | sed 's/^/BRK_BROKER_FORMAT /'
grep -a '^BRK_' "$BENCH/run_${TAG}_probe.log" 2>/dev/null
echo "BRK_LANE_EXIT rc=$rc $(date -Is)"
exit "$rc"
