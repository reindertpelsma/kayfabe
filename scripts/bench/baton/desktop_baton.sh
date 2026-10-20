#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# desktop_baton.sh - two windowed GPU clients side by side in a Linux DESKTOP guest under kayfabe,
# presented through the host display broker, with per-frame logs and the baton analysis.
# Trusted host only (172.22.1.20), run as root.  STATUS: LIVE 2026-10-20.
#
#   desktop_baton.sh plan                 print the configuration and every command; touches nothing
#   desktop_baton.sh run TAG              the whole thing (see below); evidence in $OUTROOT
#
# What `run` does (every wait is bounded; nothing is ever killed with SIGKILL; only own PIDs are signalled):
#   1. WAIT   until no QEMU runs and the shared lock is free, or the owner's Windows VM holds it (another
#             agent's lane is never interrupted).  A restore handoff in flight is never raced.
#   2. STOP   the owner's VM the way the owner launcher documents: touch the stop flag, wait for
#             OWNER_SESSION_EXIT in its log and for its QEMU to disappear.  A refusal is a STOP (exit 30).
#   3. LOCK   the shared GPU lock (flock -o, as run.sh of the night reproducer), then
#   4. BOOT   the Linux desktop guest through scripts/bench/display/interactive.sh: a throw-away qcow2
#             child of the provisioned Cinnamon/Xorg image, kf3 binary $W/kf3-bins/$REV, display=on,
#             display-broker=<socket>; interactive.sh starts the broker ITSELF in the host GNOME session
#             (wayland backend, as the session user) from $BROKER_BIN, before QEMU.
#   5. TEST   in the guest, over ssh: build glwin, check the renderer is the NVIDIA GPU, calibrate, then the
#             phases of $PLAN (default solo:30 pair:300 canary:300), each analysed on the host.
#   6. STOP   the guest (poweroff over ssh, then interactive.sh stop), collect counters.
#   7. RESTORE the owner's VM exactly as /data/codex-N1/night_repro/campaign.py restore(): rm the stop
#             flag, next unused run number, REV=0d7ba743, WIN_FLAGS=..., flock -w 90 -o LOCK launcher,
#             detached; then wait for OWNER_SESSION_READY (reported, never forced).
#
# env (defaults in `plan`): REV, ARM (mirror0|perf6|plain), KF3_FLAGS, W, OUTROOT, PLAN, K_MS, RAM_MB, SMP, FB_MB, BAR1, BROKER_BIN,
#   OWNER_LAUNCHER, OWNER_STOP, OWNER_REV, OWNER_WIN_FLAGS, LOCK, WAIT_S, NO_OWNER_RESTORE=1 (do not restore).
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
W=${W:-/var/lib/kf-windows-20261005}
REV=${REV:-0d7ba743}
ARM=${ARM:-mirror0}                     # mirror0 = KF3_TSG_MIRROR=0 | perf6 = the owner's Windows flags | plain = none
KF3_FLAGS=${KF3_FLAGS:-}                # extra KF3_* settings for a stage-B boot, e.g. "KF3_DEBUG_FORCE_MIGRATABLE=1"
PLAN=${PLAN:-"solo:30 pair:300 canary:300"}
K_MS=${K_MS:-5}
RAM_MB=${RAM_MB:-12288}; SMP=${SMP:-8}; FB_MB=${FB_MB:-8192}; BAR1=${BAR1:-}
BASE_IMG=${BASE_IMG:-$W/broker-interactive/desktop.qcow2}
BROKER_BIN=${BROKER_BIN:-/data/claude-cursor6/nvkvm-display-broker}
OWNER_LAUNCHER=${OWNER_LAUNCHER:-/data/claude-manual-owner-perf6.sh}
OWNER_STOP=${OWNER_STOP:-/tmp/kf-stop-claude-manual}
OWNER_REV=${OWNER_REV:-0d7ba743}
OWNER_WIN_FLAGS=${OWNER_WIN_FLAGS:-"KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5 KF3_TSG_MIRROR=0"}
LOCK=${LOCK:-/tmp/kayfabe-fastguest.lock}
WAIT_S=${WAIT_S:-5400}
GUEST_IP=${GUEST_IP:-192.168.77.2}
KEY=${KEY:-/workspace/bench/guest_key}
say() { printf '[desktop_baton %s] %s\n' "$(date +%H:%M:%S)" "$*"; }
die() { say "STOP: $*"; exit "${2:-2}"; }

case "$ARM" in
    perf6) ARM_ENV=(KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5 KF3_TSG_MIRROR=0) ;;
    mirror0) ARM_ENV=(KF3_TSG_MIRROR=0) ;;
    plain) ARM_ENV=() ;;
    *) die "ARM must be mirror0, perf6 or plain" ;;
esac
# shellcheck disable=SC2206  # KF3_FLAGS is a space-separated list of NAME=VALUE
ARM_ENV+=($KF3_FLAGS)

xid_count() { dmesg 2>/dev/null | grep -c 'NVRM: Xid'; }
alive() { [ -r "/proc/$1/stat" ] && [ "$(awk '{print $3}' "/proc/$1/stat" 2>/dev/null)" != Z ]; }
owner_pids() { pgrep -f "^(/usr/bin/)?bash $OWNER_LAUNCHER" 2>/dev/null; }
any_qemu() { pgrep -x qemu-system-x86 >/dev/null 2>&1; }
lock_holders() { fuser "$LOCK" 2>/dev/null | tr -s ' '; }
next_run() {   # the owner's rule (night_repro/run.py next_run): max(3094, existing boundary-kayfabe-N) + 1
    local m=3094 n d
    for d in "$W"/boundary-kayfabe-*; do n=${d##*-}; case $n in ''|*[!0-9]*) continue ;; esac; [ "$n" -gt "$m" ] && m=$n; done
    echo $(( m + 1 ))
}

ssh_opts=(-i "$KEY" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5
          -o ServerAliveInterval=15 -o ServerAliveCountMax=8)
G() { ssh "${ssh_opts[@]}" "ubuntu@$GUEST_IP" "$@"; }

plan() {
    cat <<EOF
DESKTOP_BATON_PLAN (nothing is touched)
  checkout        $REPO ($(git -C "$REPO" rev-parse --short=8 HEAD 2>/dev/null))
  kf3 binary      $W/kf3-bins/$REV/qemu-system-x86_64      ARM=$ARM  env: ${ARM_ENV[*]:-<none>}
  guest image     child of $BASE_IMG   (Ubuntu noble, kernel 6.8.0-142, NVIDIA 580.159.04, Cinnamon on Xorg, lightdm autologin ubuntu)
  guest           ram=${RAM_MB}M smp=$SMP kf3 fb-mb=$FB_MB bar1=${BAR1:-128M (interactive.sh default)}  ssh ubuntu@$GUEST_IP key $KEY
  kf3 device      (interactive.sh) guest-driver=580.159.04,display=on,gop=on,x11-dispsw=on,display-broker=/run/user/<uid>/nvkvm/display.sock,display-broker-uid=<uid>
  broker          started by interactive.sh in the host GNOME session (wayland backend): $BROKER_BIN
  phases          $PLAN   (K=$K_MS ms frames; glwin windows side by side, vsync 0; canary window vsync 1)
  owner VM        stop: touch $OWNER_STOP, wait OWNER_SESSION_EXIT; restore: REV=$OWNER_REV N=$(next_run) WIN_FLAGS="$OWNER_WIN_FLAGS"
                  flock -w 90 -o $LOCK bash $OWNER_LAUNCHER   (campaign.py restore())
  lock            flock -w 300 -o $LOCK  (stage 2); queue wait up to ${WAIT_S}s
  interactive.sh  env KF_INTERACTIVE_DIR=\$OUTROOT KF_RUN_DIR=\$OUTROOT/run KF_IMG=\$OUTROOT/desktop.qcow2 KF3_REV=$REV BENCH_DIR=\$OUTROOT/bench \\
                  KF_LOCK=\$OUTROOT/private.lock KF_RAM_MB=$RAM_MB KF_SMP=$SMP KF_FB_MB=$FB_MB BROKER_BIN=$BROKER_BIN bash $REPO/scripts/bench/display/interactive.sh
EOF
}

# ---------------------------------------------------------------------------------------------------
# stage 1 (no lock): wait for the host, stop the owner, run stage 2 under the lock, restore the owner
# ---------------------------------------------------------------------------------------------------
owner_log_of() { local p log; for p in $(owner_pids); do log=$(readlink "/proc/$p/fd/1" 2>/dev/null); [ -f "$log" ] && { echo "$log"; return; }; done; }

wait_for_host() {
    local deadline=$(( $(date +%s) + WAIT_S )) free_since=0 now
    while :; do
        now=$(date +%s)
        [ "$now" -gt "$deadline" ] && die "the host did not become available within ${WAIT_S}s" 20
        if [ -n "$(owner_pids)" ]; then
            # the owner's VM holds the lock: ours to stop, once it says it is up
            local log; log=$(owner_log_of)
            if [ -n "$log" ] && grep -q 'OWNER_SESSION_READY' "$log" 2>/dev/null; then say "owner VM is up (log $log)"; return 0; fi
            say "owner launcher running, not READY yet; waiting"
        elif any_qemu; then
            say "a QEMU runs and it is not the owner's ($(pgrep -x qemu-system-x86 | tr '\n' ' ')); waiting"; free_since=0
        elif [ -n "$(lock_holders)" ]; then
            say "lock $LOCK held by $(lock_holders) (another lane); waiting"; free_since=0
        else
            # nobody holds anything.  A restore handoff may be about to start the owner: give it 120 s
            [ "$free_since" = 0 ] && free_since=$now
            if [ $(( now - free_since )) -ge 120 ]; then say "host idle for 120 s: the owner VM is down"; return 0; fi
            say "host idle; waiting for a pending owner restore (up to 120 s)"
        fi
        sleep 20
    done
}

stop_owner() {
    local log pids
    pids=$(owner_pids); [ -n "$pids" ] || { say "no owner launcher running: nothing to stop"; return 0; }
    log=$(owner_log_of); [ -n "$log" ] || die "owner launcher running but its log is not found; no action taken" 30
    say "stopping the owner VM cleanly: touch $OWNER_STOP (log $log)"
    OWNER_TOUCHED=1
    touch "$OWNER_STOP"
    local i
    for i in $(seq 80); do   # 400 s: ACPI 180 s + guest agent 120 s inside the launcher
        if grep -q 'OWNER_SESSION_EXIT=' "$log" 2>/dev/null && [ -z "$(owner_pids)" ] && ! any_qemu; then
            say "owner VM stopped: $(grep -a 'OWNER_SESSION_EXIT=' "$log" | tail -1)"; return 0
        fi
        sleep 5
    done
    die "owner VM did not stop cleanly within 400 s; REFUSED, no signal sent (flag $OWNER_STOP left in place)" 30
}

restore_owner() {
    [ "${NO_OWNER_RESTORE:-0}" = 1 ] && { say "NO_OWNER_RESTORE=1: owner VM NOT restored"; return 0; }
    if [ -n "$(owner_pids)" ]; then say "owner launcher already running: nothing to restore"; return 0; fi
    [ "${OWNER_TOUCHED:-0}" = 1 ] || { say "the owner VM was never stopped or displaced by this run: nothing to restore"; return 0; }
    if any_qemu; then say "a QEMU is running (not ours to judge): NOT restoring; coordinator must"; return 0; fi
    local n out; n=$(next_run); out=$OUTROOT/owner-restore-$n.log
    rm -f "$OWNER_STOP"
    say "restoring the owner VM: run $n rev $OWNER_REV flags [$OWNER_WIN_FLAGS] -> $out"
    ( cd / && REV=$OWNER_REV N=$n WIN_FLAGS="$OWNER_WIN_FLAGS" setsid flock -w 90 -o "$LOCK" bash "$OWNER_LAUNCHER" \
        > "$out" 2>&1 < /dev/null & )
    local i
    for i in $(seq 120); do
        if grep -q 'OWNER_SESSION_READY' "$out" 2>/dev/null; then say "RESTORED: $(grep -a OWNER_SESSION_READY "$out" | tail -1)"; return 0; fi
        sleep 5
    done
    say "WARNING: OWNER_SESSION_READY not seen in 600 s ($out): the coordinator must check the owner VM"
    return 1
}

stage1() {
    TAG=${1:?run needs a TAG}
    OUTROOT=${OUTROOT:-/data/claude-desktop-baton-$TAG}
    [ "$(id -u)" = 0 ] || die "run as root"
    [ ! -e "$OUTROOT" ] || die "$OUTROOT exists (never reused)"
    [ -x "$W/kf3-bins/$REV/qemu-system-x86_64" ] || die "no kf3 binary $W/kf3-bins/$REV"
    [ -x "$BROKER_BIN" ] || die "no broker $BROKER_BIN"
    [ -r "$KEY" ] && [ -e "$BASE_IMG" ] || die "missing key or base image"
    mkdir -p "$OUTROOT"; export OUTROOT
    echo "DESKTOP_BATON_START tag=$TAG rev=$REV arm=$ARM plan=[$PLAN] checkout=$(git -C "$REPO" rev-parse --short=8 HEAD) $(date -Is)" | tee "$OUTROOT/START"
    trap 'rc=$?; say "restoring (exit rc=$rc)"; restore_owner; echo "DESKTOP_BATON_EXIT rc=$rc $(date -Is)" | tee "$OUTROOT/EXIT"' EXIT
    wait_for_host
    stop_owner
    OWNER_TOUCHED=1   # from here the host is ours; the owner VM is restored at exit (it was down or just stopped)
    flock -w 300 -o "$LOCK" env BATON_STAGE2=1 OUTROOT="$OUTROOT" TAG="$TAG" bash "${BASH_SOURCE[0]}" stage2 "$TAG"
    return $?
}

# ---------------------------------------------------------------------------------------------------
# stage 2 (lock held): boot the guest, run the phases, stop the guest
# ---------------------------------------------------------------------------------------------------
session_info() {
    local s; s=$(loginctl show-seat seat0 -p ActiveSession --value 2>/dev/null)
    [ -n "$s" ] || return 1
    SESSION=$s; SU=$(loginctl show-session "$s" -p Name --value); SUID=$(id -u "$SU")
    echo "HOST_SESSION user=$SU uid=$SUID type=$(loginctl show-session "$s" -p Type --value) locked=$(loginctl show-session "$s" -p LockedHint --value) idle=$(loginctl show-session "$s" -p IdleHint --value) active=$(loginctl show-session "$s" -p Active --value)"
}

guest_down() {
    say "stopping the guest"
    G 'sudo systemctl poweroff' >/dev/null 2>&1
    local i; for i in $(seq 40); do any_qemu || break; sleep 3; done
    if any_qemu; then say "guest still up after 120 s: interactive.sh stop (ACPI, then kill of ITS OWN qemu)"; fi
    env KF_INTERACTIVE_DIR="$OUTROOT" BENCH_DIR="$OUTROOT/bench" bash "$REPO/scripts/bench/display/interactive.sh" stop > "$OUTROOT/stop.log" 2>&1
    kill -0 "${LPID:-0}" 2>/dev/null && { for i in $(seq 30); do kill -0 "$LPID" 2>/dev/null || break; sleep 1; done; }
}

stage2() {
    TAG=${1:?tag}; OUT=${OUTROOT:?}
    [ "${BATON_STAGE2:-0}" = 1 ] || die "stage2 runs under the lock only (use: run TAG)"
    any_qemu && die "a QEMU is running after the lock was taken: $(pgrep -a -x qemu-system-x86 | cut -c1-120)" 31
    session_info > "$OUT/host_session.txt" || die "no graphical session on seat0" 32
    cat "$OUT/host_session.txt"
    # keep the host session awake for the run (an idle/blanked GNOME would stop the broker window's frame callbacks);
    # the inhibitor is a `sleep` that expires by itself, never signalled
    local est=900 ph2; for ph2 in $PLAN; do est=$(( est + ${ph2##*:} + 60 )); done
    runuser -u "$SU" -- env XDG_RUNTIME_DIR="/run/user/$SUID" DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$SUID/bus" \
        gnome-session-inhibit --inhibit idle --reason "kayfabe desktop baton" sleep "$est" > "$OUT/inhibit.log" 2>&1 < /dev/null &
    XID0=$(xid_count); echo "HOST_XID_BEFORE $XID0" | tee "$OUT/host_xid.txt"
    mkdir -p "$OUT/bench/kf3-bins" "$OUT/phases"
    ln -sfn "$W/kf3-bins/$REV" "$OUT/bench/kf3-bins/$REV"; ln -sfn "$KEY" "$OUT/bench/guest_key"
    qemu-img create -q -f qcow2 -F qcow2 -b "$BASE_IMG" "$OUT/desktop.qcow2" || die "qemu-img"
    { echo "revs: kayfabe=$(git -C "$REPO" rev-parse HEAD) kf3=$REV qemu_sha=$(sha256sum "$W/kf3-bins/$REV/qemu-system-x86_64" | cut -c1-16)"
      echo "broker=$BROKER_BIN sha=$(sha256sum "$BROKER_BIN" | cut -c1-16) rev=$(cat "$(dirname "$BROKER_BIN")/REV" 2>/dev/null)"
      echo "host: $(nvidia-smi --query-gpu=name,driver_version,memory.used,memory.total --format=csv,noheader)"
      echo "env_arm=${ARM_ENV[*]:-none}"; } | tee "$OUT/revs.txt"
    local unset_kf3=(); local v
    for v in $(compgen -e | grep '^KF3_'); do unset_kf3+=(-u "$v"); done
    local extra=""; [ -n "$BAR1" ] && extra="bar1-size=$BAR1"
    LAUNCH_ENV=(KF_INTERACTIVE_DIR="$OUT" KF_RUN_DIR="$OUT/run" KF_IMG="$OUT/desktop.qcow2" KF3_REV="$REV" BENCH_DIR="$OUT/bench"
                KF_LOCK="$OUT/private.lock" KF_RAM_MB="$RAM_MB" KF_SMP="$SMP" KF_FB_MB="$FB_MB" BROKER_BIN="$BROKER_BIN"
                KF3_EXTRA="$extra" "${ARM_ENV[@]}")
    say "booting the Linux desktop guest (interactive.sh)"
    env "${unset_kf3[@]}" "${LAUNCH_ENV[@]}" bash "$REPO/scripts/bench/display/interactive.sh" > "$OUT/launcher.log" 2>&1 < /dev/null &
    LPID=$!
    trap 'say "stage 2 exit: stopping the guest"; guest_down' EXIT
    local up=0 i
    for i in $(seq 140); do
        kill -0 "$LPID" 2>/dev/null || { tail -8 "$OUT/launcher.log"; die "the launcher exited before the guest came up" 33; }
        if timeout 8 ssh "${ssh_opts[@]}" "ubuntu@$GUEST_IP" true 2>/dev/null; then up=1; break; fi
        sleep 3
    done
    [ "$up" = 1 ] || die "guest ssh never came up (420 s)" 34
    say "guest ssh is up"
    G 'mkdir -p ~/baton' && scp "${ssh_opts[@]}" "$HERE/glwin.c" "$HERE/desktop_baton_guest.sh" "ubuntu@$GUEST_IP:baton/" >/dev/null || die "scp" 35
    G 'bash ~/baton/desktop_baton_guest.sh wait-desktop 240' | tee "$OUT/wait_desktop.txt" | tail -1
    grep -q DESKTOP_UP "$OUT/wait_desktop.txt" || die "the guest desktop did not come up" 36
    G 'bash ~/baton/desktop_baton_guest.sh build' | tee "$OUT/guest_build.txt" | tail -2
    G 'bash ~/baton/desktop_baton_guest.sh info' > "$OUT/guest_info.txt" 2>&1; sed 's/^/GUEST_INFO /' "$OUT/guest_info.txt"
    grep -q 'OpenGL renderer.*NVIDIA' "$OUT/guest_info.txt" || die "the guest GL renderer is not the NVIDIA GPU: no GPU number is valid" 37
    G 'sudo dmesg | grep -E "Xid|Flip event timeout|nvAssert" | tail -5' > "$OUT/guest_dmesg_before.txt" 2>&1
    local lay; lay=$(G "bash ~/baton/desktop_baton_guest.sh calibrate $K_MS" 2> "$OUT/calibrate.err" | sed -n 's/^LAYERS=//p')
    [ -n "$lay" ] || { cat "$OUT/calibrate.err"; die "calibration failed" 38; }
    echo "CALIBRATE K_MS=$K_MS LAYERS=$lay ($(tr '\n' ' ' < "$OUT/calibrate.err" | cut -c1-200))" | tee "$OUT/calibrate.txt"
    local n=0 ph mode dur
    for ph in $PLAN; do
        mode=${ph%%:*}; dur=${ph##*:}; n=$((n + 1))
        local pd="$OUT/phases/$n-$mode"; mkdir -p "$pd"
        say "phase $n: $mode ${dur}s"
        nvidia-smi --query-gpu=timestamp,utilization.gpu,clocks.sm,memory.used --format=csv,noheader -l 1 > "$pd/host_nvsmi.csv" 2>/dev/null &
        HS=$!
        # a screenshot of what the kf3 display shows, mid-phase (QEMU monitor screendump)
        ( sleep $(( dur / 2 + 14 )); printf 'screendump %s\n' "$pd/screen.ppm" | timeout 10 socat - "UNIX-CONNECT:$OUT/run/qemu.mon" >/dev/null 2>&1
          command -v convert >/dev/null && [ -s "$pd/screen.ppm" ] && convert "$pd/screen.ppm" "$pd/screen.png" 2>/dev/null ) &
        SS=$!
        timeout $(( dur + 150 )) ssh "${ssh_opts[@]}" "ubuntu@$GUEST_IP" "bash ~/baton/desktop_baton_guest.sh run $mode $dur ~/baton/out/$n-$mode $lay" > "$pd/phase.log" 2>&1
        echo "PHASE_SSH_RC=$?" >> "$pd/phase.log"
        kill "$HS" 2>/dev/null; wait "$SS" 2>/dev/null
        scp -r "${ssh_opts[@]}" "ubuntu@$GUEST_IP:baton/out/$n-$mode/." "$pd/" >/dev/null 2>&1
        grep -E 'PHASE_|WINDOWS|ERR' "$pd/phase.log" | sed 's/^/  /'
        local args=() f
        for f in A B; do [ -s "$pd/$f.csv" ] && args+=(--client "$pd/$f.log:$pd/$f.csv"); done
        [ -s "$pd/C.csv" ] && args+=(--canary "$pd/C.log:$pd/C.csv")
        if [ ${#args[@]} -gt 0 ]; then
            python3 -I "$HERE/desktop_baton_parse.py" report "${args[@]}" --per-second > "$pd/report.txt" 2>&1
            sed 's/^/  /' "$pd/report.txt" | grep -v 'fps/s'
        else echo "  NO CLIENT LOGS in $pd"; fi
        local x; x=$(xid_count)
        if [ "$x" -gt "$XID0" ]; then
            say "HOST XID ROSE ($XID0 -> $x): aborting the remaining phases"; dmesg | grep 'NVRM: Xid' | tail -5 | tee "$OUT/host_xid_new.txt"; break
        fi
    done
    echo "HOST_XID_AFTER $(xid_count) (before $XID0)" | tee -a "$OUT/host_xid.txt"
    G 'sudo dmesg | grep -E "Xid|Flip event timeout|nvAssert" | tail -20' > "$OUT/guest_dmesg_after.txt" 2>&1
    guest_down
    trap - EXIT
    local Q="$OUT/run/qemu.log"
    cp "$OUT"/run/broker.log "$OUT/broker.log" 2>/dev/null
    grep -aE 'broker\[|kf3: display|fps|tick' "$Q" 2>/dev/null | cut -c1-260 | tail -40 > "$OUT/qemu_display_lines.txt"
    grep -aE 'Present|frames arrive|dma-buf|shm tier|backend|window|attached|GRAB' "$OUT/broker.log" 2>/dev/null | cut -c1-240 | head -30 > "$OUT/broker_lines.txt"
    sed 's/^/BROKER_LINE /' "$OUT/broker_lines.txt" | head -12
    echo "QEMU_COUNTERS $(grep -ao 'broker\[[^]]*\]' "$Q" | tail -1 | cut -c1-300)"
    echo "QEMU_EVENTS stall=$(grep -ac 'display: STALL' "$Q") exception=$(grep -ac 'Exception' "$Q") scanout_refused=$(grep -ac 'scanout REFUSED' "$Q")"
    return 0
}

case "${1:-}" in
    plan) plan ;;
    run) shift; stage1 "$@" ;;
    stage2) shift; stage2 "$@" ;;
    *) echo "usage: $0 plan | run TAG" >&2; exit 2 ;;
esac
