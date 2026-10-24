#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# cursor_lane.sh — the owner's manual Wayland-cursor check, automated (owner ruling 2026-10-17,
# OWNER_RULINGS §AJ: the BROKER owns all cursors including the true XOR inversion; kayfabe composes
# none). On the trusted GPU host's live GNOME (Wayland) session:
#
#   leg xor    the broker (Wayland backend, fullscreen) and a SYNTHETIC VMM (nvkvm-broker-testclient
#              --present 320x240 --shm --cursor-xor 9x16): the host pointer is moved over the frame
#              and the host screencast must show the TRUE inversion of the frame under the I-beam (no
#              black/white approximation, no host pointer sprite). Synthetic because no Linux guest
#              emits an XOR cursor (its cursors are ARGB; the Windows I-beam is the real source).
#   leg hover  the Linux desktop guest (interactive.sh, broker Wayland backend, fullscreen) shows a
#              full-screen pattern with corner markers and a named cursor (left_ptr, xterm, crosshair);
#              the HOST pointer is moved (mutter RemoteDesktop) to points of the frame; per point:
#              guest pointer == host pointer mapped into the guest (the broker input path), the guest
#              frame (QEMU screendump) has NO cursor (kayfabe composes none), the host screencast shows
#              the guest's own cursor image (XFixes) at the pointer: shape, position, colour.
#   leg grab   CTRL+ALT+G (the broker's grab chord, injected as host keys), relative motion, then the
#              same checks at the guest's own pointer (the broker's overlay), CTRL+ALT+G again.
#   log        qemu.log has the CAP_CURSOR_STREAM relay line ("the BROKER draws it ... nothing is
#              composed into the frame"), no `host cursor COMPOSED`, cursor_refused=0.
#
# Usage (trusted GPU host only; one VM at a time):
#   scripts/bench/display/cursor_lane.sh --rev <kf3-rev8> [--out DIR] [--legs "xor hover grab"]
#        [--cursors "left_ptr xterm crosshair"] [--wait SECONDS]
#
# Hostile guest: everything the guest returns (its pointer position, its cursor PAM) is parsed with
# bounds (cursor_check.py refuses malformed or oversized images; ssh output is size-limited) and only
# compared; nothing from the guest is executed on the host. The capture reads the HOST screen.
#
# Preconditions (each a verdict, not a crash): no /tmp/kf-host-quiet, $KF_LOCK free (flock -w WAIT,
# default 0), no other qemu-system process, no broker already on the session's display.sock, a
# Wayland session active on seat0, MemAvailable >= 10 GiB. GDM and the owner's session are not
# touched: the lane adds a remote-desktop session for its own pointer and stops it at the end (the
# pointer moves on the host screen while it runs).
#
# Result: $OUT/result.json {"lane":"cursor","verdict":PASS|FAIL|INCONCLUSIVE|BUSY|ERROR|
# ROUTE_UNAVAILABLE,"legs":{...}} and RESULT lines; CURSOR_LANE_START / CURSOR_LANE_EXIT.
# Exit: 0 PASS, 1 FAIL, 2 ERROR, 3 INCONCLUSIVE, 4 ROUTE_UNAVAILABLE, 75 BUSY.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
W=${CL_W:-/var/lib/kf-windows-20261005}
LOCK=${KF_LOCK:-/tmp/kayfabe-fastguest.lock}
REV="" OUT="" WAIT=0 LEGS="xor hover grab" CURSORS="left_ptr xterm crosshair"
while [ $# -gt 0 ]; do
    case "$1" in
        --rev) REV=$2; shift 2 ;;
        --out) OUT=$2; shift 2 ;;
        --legs) LEGS=$2; shift 2 ;;
        --cursors) CURSORS=$2; shift 2 ;;
        --wait) WAIT=$2; shift 2 ;;
        *) echo "usage: cursor_lane.sh --rev REV [--out DIR] [--legs L] [--cursors C] [--wait S]" >&2; exit 2 ;;
    esac
done
OUT=${OUT:-/data/claude-autochk92/cursor-$(date +%Y%m%d-%H%M%S)}
mkdir -p "$OUT" || exit 2
chmod 0755 "$OUT"
LOG="$OUT/lane.log"
VERDICT=ERROR REASON=""
declare -A LEG
CAP_PID="" XB_PID="" TC_PID="" LAUNCHER="" ROOT=""
say() { echo "$*" | tee -a "$LOG"; }
finish() {
    [ -n "$CAP_PID" ] && { echo quit >&7 2>/dev/null; sleep 1; kill "$CAP_PID" 2>/dev/null; }
    [ -n "$TC_PID" ] && kill "$TC_PID" 2>/dev/null
    [ -n "$XB_PID" ] && kill "$XB_PID" 2>/dev/null
    if [ -n "$LAUNCHER" ]; then
        env KF_INTERACTIVE_DIR="$ROOT" bash "$HERE/interactive.sh" stop >> "$LOG" 2>&1 < /dev/null
        for _ in $(seq 30); do kill -0 "$LAUNCHER" 2>/dev/null || break; sleep 1; done
        kill "$LAUNCHER" 2>/dev/null
        rm -f "$ROOT/desktop.qcow2"
    fi
    local rc
    case "$VERDICT" in PASS) rc=0 ;; FAIL) rc=1 ;; INCONCLUSIVE) rc=3 ;; ROUTE_UNAVAILABLE) rc=4 ;; BUSY) rc=75 ;; *) rc=2 ;; esac
    {
        printf '{"lane":"cursor","classification":"MEASURED","verdict":"%s","reason":"%s","rev":"%s","checkout":"%s","legs":{' \
            "$VERDICT" "${REASON//\"/\'}" "$REV" "$(git -C "$REPO" rev-parse HEAD 2>/dev/null)"
        local first=1
        for k in "${!LEG[@]}"; do [ $first = 1 ] || printf ','; printf '"%s":"%s"' "$k" "${LEG[$k]}"; first=0; done
        printf '},"out":"%s","at":"%s"}\n' "$OUT" "$(date -Is)"
    } > "$OUT/result.json"
    say "RESULT lane=cursor verdict=$VERDICT rev=$REV reason=$REASON"
    say "CURSOR_LANE_EXIT rc=$rc $(date -Is)"
    echo "CURSOR_LANE_EXIT rc=$rc $(date -Is)" > "$OUT/EXIT"
    exit $rc
}
trap finish EXIT
refuse() { VERDICT=$1; REASON=$2; say "CURSOR_REFUSED $1: $2"; exit 0; }
say "CURSOR_LANE_START rev=${REV:-none} checkout=$(git -C "$REPO" rev-parse --short=9 HEAD 2>/dev/null) legs=\"$LEGS\" out=$OUT $(date -Is)"
echo "CURSOR_LANE_START rev=${REV:-none} $(date -Is)" > "$OUT/START"

# ── preconditions ─────────────────────────────────────────────────────────────────────────────
[ -e /tmp/kf-host-quiet ] && refuse BUSY "host quiet window: $(cat /tmp/kf-host-quiet)"
exec 9>"$LOCK"
flock -w "$WAIT" 9 || refuse BUSY "$LOCK is held by another GPU job"
others=$(pgrep -a qemu-system | cut -c1-160)
[ -z "$others" ] || refuse BUSY "another QEMU runs (one VM at a time): $others"
avail=$(awk '/MemAvailable/ {print int($2/1048576)}' /proc/meminfo)
[ "${avail:-0}" -ge 10 ] || refuse BUSY "MemAvailable ${avail} GiB < 10 GiB"
s=$(loginctl show-seat seat0 -p ActiveSession --value 2>/dev/null)
[ -n "$s" ] || refuse ERROR "no active session on seat0"
SU=$(loginctl show-session "$s" -p Name --value); SUID=$(id -u "$SU")
[ "$(loginctl show-session "$s" -p Type --value)" = wayland ] || refuse ERROR "seat0's session is not Wayland"
[ "$(loginctl show-session "$s" -p LockedHint --value)" = no ] || refuse INCONCLUSIVE "the host session is locked (a screencast would show the lock screen)"
SRUN=/run/user/$SUID
WD=""; for w in "$SRUN"/wayland-[0-9]; do [ -S "$w" ] && { WD=$(basename "$w"); break; }; done
[ -n "$WD" ] || refuse ERROR "no Wayland socket in $SRUN"
pgrep -f "nvkvm-display-broker --socket $SRUN/nvkvm/display.sock" >/dev/null && refuse BUSY "a broker already serves $SRUN/nvkvm/display.sock"
USER_ENV=(XDG_RUNTIME_DIR="$SRUN" WAYLAND_DISPLAY="$WD" DBUS_SESSION_BUS_ADDRESS="unix:path=$SRUN/bus")
make -s -C "$REPO/broker" > "$OUT/broker-build.log" 2>&1 || refuse ERROR "broker build failed (broker-build.log)"
BROKER=$REPO/broker/nvkvm-display-broker TC=$REPO/broker/nvkvm-broker-testclient
python3 "$HERE/cursor_check.py" selftest > "$OUT/grader-selftest.log" 2>&1 || refuse ERROR "the grader's selftest failed"
CONNECTORS=$(for c in /sys/class/drm/card*-*; do [ "$(cat "$c/status" 2>/dev/null)" = connected ] && basename "$c" | sed 's/^card[0-9]*-//'; done)
say "CURSOR_PRECONDITIONS ok session=$s user=$SU wayland=$WD connectors=$(echo $CONNECTORS) mem_avail=${avail}GiB"

# ── the host capture: one remote-desktop + screencast session on the connector that shows $1 ────
cap_start() { # cap_start <connector>
    rm -f "$OUT/cap.in"; mkfifo "$OUT/cap.in"; chmod 0666 "$OUT/cap.in"
    runuser -u "$SU" -- env "${USER_ENV[@]}" python3 "$HERE/cursor_capture.py" --connector "$1" \
        < "$OUT/cap.in" > "$OUT/cap.out" 2> "$OUT/cap.err" &
    CAP_PID=$!
    exec 7> "$OUT/cap.in"
    for _ in $(seq 100); do
        grep -q '^CAP_READY\|^CAP_ERR' "$OUT/cap.out" 2>/dev/null && break
        sleep 0.2
    done
    grep -q '^CAP_READY' "$OUT/cap.out"
}
cap_stop() { [ -n "$CAP_PID" ] && { echo quit >&7; exec 7>&-; wait "$CAP_PID" 2>/dev/null; CAP_PID=""; }; }
cap() { # cap <command...>: one command, wait for its reply line
    local n; n=$(wc -l < "$OUT/cap.out")
    echo "$*" >&7
    for _ in $(seq 100); do
        [ "$(wc -l < "$OUT/cap.out")" -gt "$n" ] && { tail -1 "$OUT/cap.out"; return 0; }
        sleep 0.1
    done
    echo "CAP_ERR timeout $*"
    return 1
}
# find the connector whose screencast satisfies `cursor_check.py locate` (or the xor frame)
cap_find() { # cap_find <locate-args...>: leaves the capture running on the matching connector
    for c in $CONNECTORS; do
        cap_start "$c" || { say "CURSOR_CAPTURE $c: $(cat "$OUT/cap.out" "$OUT/cap.err" 2>/dev/null | tail -2 | tr '\n' ' ')"; cap_stop; continue; }
        cap shot "$OUT/find-$c.ppm" > /dev/null
        if python3 "$HERE/cursor_check.py" "$@" --shot "$OUT/find-$c.ppm" > "$OUT/find-$c.json" 2>&1; then
            say "CURSOR_CAPTURE on $c: $(cat "$OUT/find-$c.json")"; CONN=$c; return 0
        fi
        cap_stop
    done
    return 1
}
overall=PASS
leg() { # leg <name> <verdict>
    LEG[$1]=$2; say "CURSOR_LEG $1 $2"
    case $2 in FAIL) overall=FAIL ;; INCONCLUSIVE|ROUTE_UNAVAILABLE) [ "$overall" = PASS ] && overall=INCONCLUSIVE ;; esac
}

# ── leg xor: the broker against a synthetic VMM, true inversion on the real compositor ──────────
if [[ " $LEGS " == *" xor "* ]]; then
    XS=$SRUN/nvkvm/cursor-lane-xor.sock
    install -d -o "$SU" -m 0700 "$SRUN/nvkvm"; rm -f "$XS"
    runuser -u "$SU" -- env "${USER_ENV[@]}" "$BROKER" --socket "$XS" --backend wayland --fullscreen --verbose \
        --title "kayfabe cursor lane (xor)" > "$OUT/xor-broker.log" 2>&1 &
    XB_PID=$!
    for _ in $(seq 50); do [ -S "$XS" ] && break; sleep 0.2; done
    runuser -u "$SU" -- "$TC" "$XS" --present 320x240 --shm --cursor-xor 9x16 --hold 60 > "$OUT/xor-client.log" 2>&1 &
    TC_PID=$!
    sleep 3
    # the testclient frame: green with a red top-left quarter; find the connector that shows it and
    # the host pixel of frame pixel (200,150), on the green field
    if ! cap_find locate-xor --frame-size 320 240 --at 200 150; then
        if grep -q CAP_ERR "$OUT/cap.out" 2>/dev/null; then leg xor ROUTE_UNAVAILABLE; else leg xor INCONCLUSIVE; fi
    else
        # shellcheck disable=SC2046  # two integers, split on purpose
        set -- $(sed -n 's/.*"host": \[\([0-9]*\), \([0-9]*\)\].*/\1 \2/p' "$OUT/find-$CONN.json")
        cap move "$1" "$2" >> "$LOG"; sleep 0.7
        cap shot "$OUT/xor-shot.ppm" >> "$LOG"
        python3 "$HERE/cursor_check.py" xor --shot "$OUT/xor-shot.ppm" --host "$1" "$2" --frame-size 320 240 --mask 9 16 \
            --out "$OUT/xor.json" | tee -a "$LOG" > /dev/null
        v=$(sed -n 's/.*"verdict": "\([A-Z]*\)".*/\1/p' "$OUT/xor.json")
        grep -q 'XOR composed by the broker' "$OUT/xor-broker.log" || { say "CURSOR_XOR broker log has no 'XOR composed by the broker'"; v=FAIL; }
        grep -q 'XOR approximated' "$OUT/xor-broker.log" && { say "CURSOR_XOR the broker approximated"; v=FAIL; }
        leg xor "${v:-INCONCLUSIVE}"
        cap_stop
    fi
    kill "$TC_PID" "$XB_PID" 2>/dev/null; TC_PID="" XB_PID=""
fi

# ── legs hover / grab: the Linux desktop guest ─────────────────────────────────────────────────
if [[ " $LEGS " == *" hover "* || " $LEGS " == *" grab "* ]]; then
    [ -n "$REV" ] || refuse ERROR "--rev is required for the guest legs"
    [ -x "/workspace/bench/kf3-bins/$REV/qemu-system-x86_64" ] || ln -sfn "$W/kf3-bins/$REV" "/workspace/bench/kf3-bins/$REV"
    [ -x "/workspace/bench/kf3-bins/$REV/qemu-system-x86_64" ] || refuse ERROR "no kf3 binary for $REV"
    ROOT=$OUT/guest; mkdir -p "$ROOT"
    qemu-img create -q -f qcow2 -F qcow2 -b "$W/broker-interactive/desktop.qcow2" "$ROOT/desktop.qcow2" || refuse ERROR "qemu-img"
    env KF_INTERACTIVE_DIR="$ROOT" KF_RUN_DIR="$ROOT/run" KF_IMG="$ROOT/desktop.qcow2" KF3_REV="$REV" \
        KF_LOCK="$ROOT/private.lock" KF_RAM_MB=${CL_RAM_MB:-6144} KF_FB_MB=${CL_FB_MB:-4096} \
        KF_BROKER_BACKEND=wayland BROKER_BIN="$BROKER" BROKER_ARGS="--fullscreen" \
        bash "$HERE/interactive.sh" > "$ROOT/launcher.log" 2>&1 < /dev/null &
    LAUNCHER=$!
    Q=$ROOT/run/qemu.log MON=$ROOT/run/qemu.mon
    G=(ssh -i /workspace/bench/guest_key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
       -o LogLevel=ERROR -o ConnectTimeout=5 ubuntu@192.168.77.2)
    up=0
    for _ in $(seq 1 100); do
        kill -0 "$LAUNCHER" 2>/dev/null || refuse ERROR "the launcher exited ($(tail -2 "$ROOT/launcher.log" | tr '\n' ' '))"
        timeout 8 "${G[@]}" 'pgrep -x cinnamon' >/dev/null 2>&1 && grep -aq 'display console registered' "$Q" 2>/dev/null && { up=1; break; }
        sleep 3
    done
    [ "$up" = 1 ] || refuse ERROR "the guest desktop never came up"
    sleep 5
    for f in cursor_guest_pattern.py xcursor.py; do timeout 20 "${G[@]}" "cat > /tmp/$f" < "$HERE/$f"; done
    GX=(env DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority)
    hmp_shot() { # the guest frame kayfabe produced
        python3 - "$MON" "$1" <<'PY'
import socket, sys, time
s = socket.socket(socket.AF_UNIX); s.settimeout(10); s.connect(sys.argv[1]); time.sleep(0.3); s.recv(65536)
s.sendall(f"screendump {sys.argv[2]} kf0\n".encode()); time.sleep(2.0); s.close()
PY
        [ -s "$1" ]
    }
    guest_ptr() { timeout 10 "${G[@]}" "${GX[*]} python3 /tmp/xcursor.py pointer" 2>/dev/null | head -c 200 | awk '/^POINTER/ {print $2, $3}'; }
    pattern() { # pattern <cursor name>
        timeout 10 "${G[@]}" "pkill -f cursor_guest_pattern.py; ${GX[*]} nohup python3 /tmp/cursor_guest_pattern.py --cursor $1 --seconds 900 > /tmp/pattern.log 2>&1 &"
        sleep 4
    }
    pattern "${CURSORS%% *}"
    hmp_shot "$OUT/frame-0.ppm" || refuse ERROR "HMP screendump failed"
    read -r GW GH < <(sed -n 2p "$OUT/frame-0.ppm")
    if ! cap_find locate --frame-size "$GW" "$GH"; then
        grep -q CAP_ERR "$OUT/cap.out" 2>/dev/null && { leg capture ROUTE_UNAVAILABLE; REASON="mutter screencast/remote desktop: $(grep -m1 CAP_ERR "$OUT/cap.out")"; } \
            || leg capture INCONCLUSIVE
    else
        MAPJ=$(cat "$OUT/find-$CONN.json")
        tohost() { python3 -c "import json; m = json.loads('$MAPJ'); print(round(m['ox'] + ($1 + 0.5) * m['sx'] - 0.5), round(m['oy'] + ($2 + 0.5) * m['sy'] - 0.5))"; }
        if [[ " $LEGS " == *" hover "* ]]; then
            hv=PASS
            for cur in $CURSORS; do
                pattern "$cur"
                for pt in "$((GW / 2)) $((GH / 2))" "$((GW / 4)) $((GH / 4))" "$((3 * GW / 4)) $((2 * GH / 3))"; do
                    set -- $pt; read -r HX HY < <(tohost "$1" "$2")
                    cap move "$HX" "$HY" >> "$LOG"; sleep 1.2
                    read -r PX PY < <(guest_ptr)
                    timeout 10 "${G[@]}" "${GX[*]} python3 /tmp/xcursor.py image /tmp/c.pam >/dev/null; head -c 300000 /tmp/c.pam" > "$OUT/cursor-$cur.pam"
                    tag="hover-$cur-$1x$2"
                    hmp_shot "$OUT/$tag-frame.ppm"
                    cap shot "$OUT/$tag-shot.ppm" >> "$LOG"
                    python3 "$HERE/cursor_check.py" hover --shot "$OUT/$tag-shot.ppm" --frame "$OUT/$tag-frame.ppm" \
                        --cursor "$OUT/cursor-$cur.pam" --host "$HX" "$HY" --guest-pointer "${PX:-0}" "${PY:-0}" \
                        --out "$OUT/$tag.json" | sed "s/^/$tag /" | tee -a "$LOG" > /dev/null
                    case $(sed -n 's/.*"verdict": "\([A-Z]*\)".*/\1/p' "$OUT/$tag.json") in
                        PASS) ;; INCONCLUSIVE) [ "$hv" = PASS ] && hv=INCONCLUSIVE ;; *) hv=FAIL ;;
                    esac
                done
            done
            leg hover "$hv"
        fi
        if [[ " $LEGS " == *" grab "* ]]; then
            pattern "${CURSORS%% *}"
            read -r HX HY < <(tohost "$((GW / 2))" "$((GH / 2))"); cap move "$HX" "$HY" >> "$LOG"; sleep 1
            for k in "29 1" "56 1" "34 1" "34 0" "56 0" "29 0"; do cap key $k >> "$LOG"; done
            sleep 1; cap rel 40 25 >> "$LOG"; sleep 1.2
            read -r PX PY < <(guest_ptr)
            timeout 10 "${G[@]}" "${GX[*]} python3 /tmp/xcursor.py image /tmp/c.pam >/dev/null; head -c 300000 /tmp/c.pam" > "$OUT/cursor-grab.pam"
            hmp_shot "$OUT/grab-frame.ppm"
            cap shot "$OUT/grab-shot.ppm" >> "$LOG"
            read -r GHX GHY < <(tohost "${PX:-0}" "${PY:-0}")
            python3 "$HERE/cursor_check.py" grab --shot "$OUT/grab-shot.ppm" --frame "$OUT/grab-frame.ppm" \
                --cursor "$OUT/cursor-grab.pam" --host "$GHX" "$GHY" --guest-pointer "${PX:-0}" "${PY:-0}" \
                --out "$OUT/grab.json" | tee -a "$LOG" > /dev/null
            gv=$(sed -n 's/.*"verdict": "\([A-Z]*\)".*/\1/p' "$OUT/grab.json")
            grep -aq 'grab' "$ROOT/run/broker.log" 2>/dev/null || say "CURSOR_GRAB no grab line in the broker log"
            for k in "29 1" "56 1" "34 1" "34 0" "56 0" "29 0"; do cap key $k >> "$LOG"; done
            leg grab "${gv:-INCONCLUSIVE}"
        fi
        cap_stop
    fi
    # the relay's own statement: the broker draws the cursor, kayfabe composes nothing
    if grep -aq 'the BROKER draws it in every mode (CAP_CURSOR_STREAM); nothing is composed into the frame' "$Q"; then
        lv=PASS
    else
        lv=FAIL; say "CURSOR_LOG no CAP_CURSOR_STREAM relay line"
    fi
    grep -aq 'host cursor COMPOSED' "$Q" && { lv=FAIL; say "CURSOR_LOG kayfabe composed a cursor"; }
    st=$(grep -ao 'broker\[[^]]*\]' "$Q" | tail -1)
    say "CURSOR_LOG status $st"
    [[ "$st" == *"cursor_refused=0"* ]] || { lv=FAIL; say "CURSOR_LOG cursor_refused is not 0"; }
    leg log "$lv"
fi
VERDICT=$overall
[ -n "$REASON" ] || REASON="legs: $LEGS"
exit 0
