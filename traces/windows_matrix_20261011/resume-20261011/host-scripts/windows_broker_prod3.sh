#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# shellcheck disable=SC1090,SC2054  # the state file is this script's own; commas are QEMU property lists
# PRODUCTION COPY (host-only, untracked): FLAGS = class-B only, no measurement flag, no KF3_RPC_TRACE
# windows_broker.sh — the Windows 11 guest (NVIDIA 580.88) on kf3, shown in a SECOND display-broker
# window on the host's live desktop, beside the Linux guest of scripts/bench/display/interactive.sh
# (its own socket, its own window title, its own run directory). 2026-10-08, the owner: "It's ok to
# have 2 broker screens on the 1.20 host."
#
# ★ 2026-10-08 (later): INSTANCES. `desktop` (the persistent overlay, window "kayfabe Windows", socket
#   windows.sock) and each `run N` (a fresh overlay, window "kayfabe Windows run N", socket windows-runN.sock)
#   run side by side, each with its own state file ($W/windows-broker-<inst>.state). Every command that acts on
#   a guest names its instance (`desktop` or the run number); nothing ever picks one by default.
#
#   windows_broker.sh run [N]     boot run N (default: the next free boundary-kayfabe-N) DETACHED:
#                                 a fresh overlay of the baseline, its own broker window, QEMU; returns
#   windows_broker.sh desktop     boot the persistent desktop overlay ($W/windows-desktop); refused while
#                                 any QEMU has that overlay open (never two VMs on one disk)
#   windows_broker.sh stop INST   CLEAN shutdown: ACPI power-down, wait up to 180 s, then QGA shutdown,
#                                 wait up to 120 s more; only then a kill, logged UNCLEAN — and for `desktop`
#                                 not even then unless WIN_FORCE_KILL=1 (an unclean stop corrupted the
#                                 desktop overlay's UEFI variables once, 2026-10-08). Then its broker window.
#   windows_broker.sh broker INST (re)start only that instance's broker window (kf3 reconnects to it)
#   windows_broker.sh status [INST]  running guests, run directories, QGA ping, host Xid count
#   windows_broker.sh autologon   (desktop overlay, guest running) through QGA as SYSTEM: give the
#                                 local account 'kf' a password from $W/windows-desktop/secrets
#                                 (0600, generated here, never committed) and enable Winlogon autologon
#   windows_broker.sh nvidia disable|enable|status INST   the guest's NVIDIA display device, through
#                                 QGA + pnputil (nvidia_device.ps1); takes effect at the next boot (`reboot`)
#   windows_broker.sh reboot INST restart the guest (QGA, Restart-Computer -Force)
#
# ★ 2026-10-08 (run58): the visible, interactive desktop TODAY is the Basic Display fallback: `desktop`,
#   `autologon`, `nvidia disable`, `reboot` — Windows then draws through Microsoft Basic Display on kf3's GOP
#   framebuffer (1920x1080, software rendering, no GPU acceleration) and kf3 shows it in the window. With the NVIDIA
#   driver enabled the screen stays on the boot frame (runs 54-57 in traces/windows_code43_walls_20261007/README.md).
#
# Same guest and harness as runs 47-53 (`pc_sdr_experiment.py`'s pinned template: machine pc, 8 GiB,
# 8 vCPU, virtio-blk/net, QGA, kf3 fb-mb=4096 bar1-size=128 MiB), with these differences, all named
# in the run's command.json:
#   - kf3 `display-broker=<sock>,display-broker-uid=<uid>` (the window) and, by default, `gop=on`
#     with NO QEMU std VGA (WIN_STDVGA=1 restores run53's `-device VGA,addr=0x9` and gop off): the
#     window then shows OVMF and Windows' boot screen on kf3's GOP, then whatever nvlddmkm scans out;
#   - input: the pc machine's PS/2 keyboard and mouse (inbox Windows drivers) plus a USB tablet on
#     qemu-xhci (inbox HID; absolute pointer while hovering);
#   - no GPU lock: the Linux guest of interactive.sh holds /tmp/kayfabe-fastguest.lock, and two kf3
#     VMs on one GPU is the point of this launcher (watch `dmesg` for Xid).
# The audited runner (`pc_boundary_experiment.py`) is not used because it takes that lock and
# refuses to add devices; its env (KF3_RPC_TRACE=1 and the flag list) is reproduced here.
#
# env: KF3_REV (kf3 binary under $W/kf3-bins; required), WIN_FLAGS (space-separated extra KF3_*
#   flags set to 1, e.g. "KF3_PREEMPT_BIND_PROBE"), WIN_STDVGA (0), WIN_MAX_SECONDS (0 = no bound),
#   WIN_TITLE ("kayfabe Windows"), WIN_RAM_MB (8192), WIN_FB_MB (4096; the store is real host VRAM:
#   [measured, run54 at a88764b3, 2026-10-08] beside the Linux guest (fb-mb=8192) a 4096 MiB store is
#   refused NoMemory on the 12 GiB RTX 4070, so 2048 there).
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
W=${WIN_DIR:-/var/lib/kf-windows-20261005}
B=${BROKER_BIN:-/opt/nvkvm-broker/nvkvm-display-broker}
TOOLS=$W/boundary-tools
cmd=${1:-}
say(){ printf '[windows] %s\n' "$*"; }
die(){ printf '[windows] ★ %s\n' "$*" >&2; exit 2; }
[ "$(id -u)" = 0 ] || die "run as root"

# ── instances ──────────────────────────────────────────────────────────────────────────────────
# $1 = `desktop` or a run number → INST, STATE, RUNDIR, WTITLE, SOCKNAME
inst() {
    case "${1:-}" in
    desktop) INST=desktop; RUNDIR=$W/windows-desktop; WTITLE=${WIN_TITLE:-kayfabe Windows}; SOCKNAME=windows.sock ;;
    ''|*[!0-9]*) die "name the instance: desktop or a run number (got '${1:-}')" ;;
    *) INST=run$1; RUNDIR=$W/boundary-kayfabe-$1; WTITLE=${WIN_TITLE:-kayfabe Windows run $1}; SOCKNAME=windows-run$1.sock ;;
    esac
    STATE=$W/windows-broker-$INST.state
    # the single state file of the first version (2026-10-08, runs 54-59) named the desktop
    if [ "$INST" = desktop ] && [ ! -s "$STATE" ] && [ -s "$W/windows-broker.state" ] \
       && grep -qx "RUN=$W/windows-desktop" "$W/windows-broker.state"; then
        cp "$W/windows-broker.state" "$STATE"
    fi
}

# the desktop session (seat0's active graphical session), as interactive.sh finds it
session() {
    local s
    s=$(loginctl show-seat seat0 -p ActiveSession --value 2>/dev/null)
    [ -n "$s" ] || s=$(loginctl list-sessions --no-legend | awk '$4 == "seat0" {print $1; exit}')
    [ -n "$s" ] || return 1
    SU=$(loginctl show-session "$s" -p Name --value); SUID=$(id -u "$SU" 2>/dev/null) || return 1
    STYPE=$(loginctl show-session "$s" -p Type --value)
    SRUN=/run/user/$SUID
    WD=""; for w in "$SRUN"/wayland-[0-9]; do [ -S "$w" ] && { WD=$(basename "$w"); break; }; done
    XD=""; XA=""
    local xp; xp=$(pgrep -u "$SU" -o -x Xwayland || pgrep -o -x Xorg)
    if [ -n "$xp" ]; then
        XD=$(tr '\0' '\n' < "/proc/$xp/cmdline" | grep -m1 '^:[0-9]')
        XA=$(tr '\0' '\n' < "/proc/$xp/cmdline" | grep -A1 -m1 '^-auth$' | tail -1)
    fi
    SOCK=$SRUN/nvkvm/$SOCKNAME
}

broker_down() {
    pkill -f "^[^ ]*nvkvm-display-broker --socket $SOCK( |\$)" 2>/dev/null
    sleep 1
    pkill -9 -f "^[^ ]*nvkvm-display-broker --socket $SOCK( |\$)" 2>/dev/null
    return 0
}

broker_up() {   # $1 = log
    local log=$1 backend=x11
    [ -x "$B" ] || die "no broker at $B (scripts/bench/display/interactive.sh prep builds it)"
    [ "$STYPE" = wayland ] && [ -n "$WD" ] && backend=wayland
    broker_down
    install -d -o "$SU" -m 0700 "$(dirname "$SOCK")"; rm -f "$SOCK"
    # shellcheck disable=SC2086
    setsid runuser -u "$SU" -- env XDG_RUNTIME_DIR="$SRUN" WAYLAND_DISPLAY="$WD" DISPLAY="$XD" \
        XAUTHORITY="$XA" DBUS_SESSION_BUS_ADDRESS="unix:path=$SRUN/bus" \
        "$B" --socket "$SOCK" --backend "$backend" --persist --verbose \
        --title "$WTITLE" ${BROKER_ARGS:-} > "$log" 2>&1 < /dev/null &
    for _ in $(seq 50); do [ -S "$SOCK" ] && break; sleep 0.2; done
    [ -S "$SOCK" ] || { tail -5 "$log" >&2; die "the broker did not create $SOCK (log $log)"; }
    say "broker: backend $backend, socket $SOCK, title '$WTITLE' (log $log)"
}

# the QEMU of the current instance, if alive AND still the QEMU of that run directory
running_pid() {
    [ -s "$STATE" ] || return 1
    local qp rd
    qp=$(sed -n 's/^QPID=//p' "$STATE"); rd=$(sed -n 's/^RUN=//p' "$STATE")
    [ -n "$qp" ] && kill -0 "$qp" 2>/dev/null && tr '\0' ' ' < "/proc/$qp/cmdline" | grep -q "file=$rd/windows.qcow2" \
        && echo "$qp"
}

xid_count() { dmesg 2>/dev/null | grep -c 'NVRM: Xid'; }

wait_gone() {   # $1 = pid, $2 = seconds
    local i; for i in $(seq "$2"); do kill -0 "$1" 2>/dev/null || return 0; sleep 1; done; return 1
}

case "$cmd" in
stop)
    inst "${2:-}"; session || true
    if q=$(running_pid); then
        python3 "$TOOLS/qmp.py" "$RUNDIR/qmp.sock" cmd system_powerdown >/dev/null 2>&1
        if wait_gone "$q" 180; then how=acpi
        else
            say "no ACPI shutdown after 180 s — asking the guest agent"
            timeout 30 python3 "$TOOLS/qmp.py" "$RUNDIR/qga.sock" qga-exec shutdown.exe /s /f /t 0 >/dev/null 2>&1
            if wait_gone "$q" 120; then how=qga
            elif [ "$INST" = desktop ] && [ "${WIN_FORCE_KILL:-0}" != 1 ]; then
                die "the desktop guest (QEMU $q) did not shut down; NOT killed (WIN_FORCE_KILL=1 kills it, unclean)"
            else
                kill "$q"; wait_gone "$q" 10 || kill -9 "$q" 2>/dev/null; how=UNCLEAN-kill
                say "★ QEMU $q killed after 300 s without a clean shutdown (UNCLEAN)"
            fi
        fi
        echo "WINDOWS_EXIT stop=$how $(date -Is) xid=$(xid_count)" >> "$RUNDIR/marker.txt"
    else
        say "no $INST guest running"
    fi
    [ -n "${SOCK:-}" ] && broker_down
    say "$INST stopped (Xid count now $(xid_count))"; exit 0 ;;
status)
    for st in "$W"/windows-broker-*.state; do
        [ -e "$st" ] || continue
        i=${st##*/windows-broker-}; i=${i%.state}; [ "$i" = desktop ] || i=${i#run}
        [ -n "${2:-}" ] && [ "$2" != "$i" ] && continue
        inst "$i"
        if q=$(running_pid); then
            say "$INST: QEMU $q, $RUNDIR, kf3 $(sed -n 's/^KF3_REV=//p' "$STATE")"
            if timeout 15 python3 "$TOOLS/qmp.py" "$RUNDIR/qga.sock" qga-ping >/dev/null 2>&1; then
                say "  QGA answers"
            else
                say "  QGA silent"
            fi
        fi
    done
    say "host Xid lines in dmesg: $(xid_count)"; exit 0 ;;
broker)
    inst "${2:-}"; session || die "no graphical session on seat0"
    broker_up "$RUNDIR/broker-$(date +%Y%m%d-%H%M%S).log"; exit 0 ;;
autologon)
    inst desktop
    running_pid > /dev/null || die "the desktop guest is not running"
    RUN=$RUNDIR
    install -d -m 0700 "$RUN/secrets"
    [ -s "$RUN/secrets/win_password" ] || { openssl rand -base64 18 | tr -d '\n/+=' > "$RUN/secrets/win_password"; chmod 0600 "$RUN/secrets/win_password"; }
    PW=$(cat "$RUN/secrets/win_password")
    ps="\$u='kf'; if (-not (Get-LocalUser -Name \$u -ErrorAction SilentlyContinue)) { New-LocalUser -Name \$u -Password (ConvertTo-SecureString '$PW' -AsPlainText -Force) -PasswordNeverExpires | Out-Null; Add-LocalGroupMember -Group Administrators -Member \$u } else { Set-LocalUser -Name \$u -Password (ConvertTo-SecureString '$PW' -AsPlainText -Force) }; \$k='HKLM:\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon'; Set-ItemProperty \$k AutoAdminLogon '1'; Set-ItemProperty \$k DefaultUserName \$u; Set-ItemProperty \$k DefaultPassword '$PW'; Set-ItemProperty \$k DefaultDomainName '.'; Remove-ItemProperty \$k AutoLogonCount -ErrorAction SilentlyContinue; Set-ItemProperty 'HKLM:\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\PasswordLess\\Device' DevicePasswordLessBuildVersion 0 -Type DWord -ErrorAction SilentlyContinue; 'AUTOLOGON_SET'"
    # ⊘ [measured 2026-10-08, run58 desktop] with only the Winlogon values, the first reboot logged in and the
    # next boot showed the lock screen: Windows had reset AutoAdminLogon to 0 and dropped DefaultPassword
    # (DevicePasswordLessBuildVersion was 2, the passwordless default). Setting it to 0 is the documented fix
    # (inferred to be the cause; checked by the following boot).
    timeout 120 python3 "$TOOLS/qmp.py" "$RUN/qga.sock" qga-exec powershell.exe -NoProfile -Command "$ps" | grep -E 'AUTOLOGON_SET|rror'
    say "autologon set for 'kf' (password in $RUN/secrets/win_password, 0600); takes effect at the next boot"
    exit 0 ;;
nvidia)
    act=${2:-status}; case "$act" in disable|enable|status) ;; *) die "nvidia disable|enable|status INST" ;; esac
    inst "${3:-}"
    running_pid > /dev/null || die "no $INST guest running"
    tmp=$(mktemp); printf '$Action = %s\n' "'$act'" > "$tmp"; cat "$HERE/nvidia_device.ps1" >> "$tmp"
    timeout 280 python3 "$TOOLS/qmp.py" "$RUNDIR/qga.sock" qga-exec powershell.exe -NoProfile -Command "$(cat "$tmp")"
    rm -f "$tmp"; exit 0 ;;
reboot)
    inst "${2:-}"
    running_pid > /dev/null || die "no $INST guest running"
    timeout 30 python3 "$TOOLS/qmp.py" "$RUNDIR/qga.sock" qga-exec powershell.exe -NoProfile -Command "Restart-Computer -Force"
    say "$INST restarting (the window shows OVMF, then Windows)"; exit 0 ;;
run|desktop) ;;
*) die "usage: $0 run [N] | desktop | stop INST | broker INST | status [INST] | autologon | nvidia disable|enable|status INST | reboot INST" ;;
esac

# ── run / desktop ───────────────────────────────────────────────────────────────────────────────
[ -n "${KF3_REV:-}" ] || die "KF3_REV names the kf3 binary (under $W/kf3-bins)"
QBIN=$W/kf3-bins/$KF3_REV/qemu-system-x86_64
[ -x "$QBIN" ] || die "no kf3 binary at $QBIN"
if [ "$cmd" = desktop ]; then
    inst desktop
    q=$(running_pid) && die "the desktop guest is already running (QEMU $q)"
    pgrep -f "file=$RUNDIR/windows.qcow2" > /dev/null && die "a QEMU already has $RUNDIR/windows.qcow2 open"
    NAME=kayfabe-windows-desktop
    mkdir -p "$RUNDIR"; chmod 0700 "$RUNDIR"
    [ -e "$RUNDIR/windows.qcow2" ] || qemu-img create -q -f qcow2 -F qcow2 -b "$W/baseline/windows.qcow2" "$RUNDIR/windows.qcow2" || die "qemu-img"
    [ -e "$RUNDIR/OVMF_VARS.fd" ] || cp "$W/baseline/OVMF_VARS.fd" "$RUNDIR/OVMF_VARS.fd"
else
    N=${2:-}
    if [ -z "$N" ]; then N=54; while [ -e "$W/boundary-kayfabe-$N" ]; do N=$((N + 1)); done; fi
    inst "$N"; NAME=boundary-kayfabe-$N
    if [ "${WIN_REUSE:-0}" = 1 ]; then
        # ★ 2026-10-08 (run62): a SECOND boot of run N's own disk in a NEW QEMU process (after a clean
        # stop) — the guest-side change made in its first boot (e.g. a registry value) is kept. An
        # in-guest reboot is not the same thing: [measured, run57 and run62 at 883f878e] the second
        # boot inside one QEMU process found the NVIDIA adapter at Code 43 both times.
        [ -e "$RUNDIR/windows.qcow2" ] || die "WIN_REUSE=1: $RUNDIR has no disk"
        running_pid > /dev/null && die "run $N is still running"
        pgrep -f "file=$RUNDIR/windows.qcow2" > /dev/null && die "a QEMU already has $RUNDIR/windows.qcow2 open"
        mv "$RUNDIR/qemu.log" "$RUNDIR/qemu-boot$(date +%H%M%S).log" 2>/dev/null
    else
        [ -e "$RUNDIR" ] && die "$RUNDIR exists (a run directory is never reused; WIN_REUSE=1 boots its disk again)"
        mkdir -m 0700 "$RUNDIR"
        qemu-img create -q -f qcow2 -F qcow2 -b "$W/baseline/windows.qcow2" "$RUNDIR/windows.qcow2" || die "qemu-img"
        cp "$W/baseline/OVMF_VARS.fd" "$RUNDIR/OVMF_VARS.fd"
    fi
fi
RUN=$RUNDIR
session || die "no graphical session on seat0"
rm -f "$RUN"/qmp.sock "$RUN"/qga.sock
stamp=$(date +%Y%m%d-%H%M%S)
echo "WINDOWS_START kf3=$KF3_REV run=$RUN checkout=$(git -C "$HERE/../../.." rev-parse --short=8 HEAD 2>/dev/null) xid_before=$(xid_count) $(date -Is)" | tee -a "$RUN/marker.txt"
broker_up "$RUN/broker-$stamp.log"

KF3="kf3-gpu,fb-mb=${WIN_FB_MB:-4096},bar1-size=134217728,bar2-size=33554432,display=on,guest-driver=580.65.06,bus=pci.0,addr=0x6,id=kf0,display-broker=$SOCK,display-broker-uid=$SUID"
VGA=()
if [ "${WIN_STDVGA:-0}" = 1 ]; then VGA=(-device VGA,addr=0x9); else KF3+=",gop=on"; fi
# shellcheck disable=SC2054
ARGS=(-name "$NAME" -nodefaults -no-user-config
    -machine pc,accel=kvm,smm=on,memory-backend=ram0
    -object "memory-backend-memfd,id=ram0,size=${WIN_RAM_MB:-8192}M,share=on" -m "${WIN_RAM_MB:-8192}" -smp 8 -cpu host,-vmx
    -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd
    -drive "if=pflash,format=raw,unit=1,file=$RUN/OVMF_VARS.fd"
    -rtc base=utc,driftfix=slew -global kvm-pit.lost_tick_policy=discard
    "${VGA[@]}" -display none
    -qmp "unix:$RUN/qmp.sock,server=on,wait=off"
    -serial "file:$RUN/serial.log" -monitor none -action reboot=reset,panic=none
    -netdev "user,id=n0,restrict=off,net=10.0.2.0/24,host=10.0.2.2,dns=10.0.2.3,dhcpstart=10.0.2.15"
    -device virtio-net-pci,netdev=n0,mac=52:54:00:56:77:20,addr=0xa,disable-modern=on
    -device virtio-serial-pci,id=serial0,addr=0x3,disable-modern=on
    -chardev "socket,id=qga0,path=$RUN/qga.sock,server=on,wait=off"
    -device virtserialport,bus=serial0.0,chardev=qga0,name=org.qemu.guest_agent.0
    -drive "file=$RUN/windows.qcow2,if=none,id=disk0,format=qcow2,discard=unmap"
    -device virtio-blk-pci,drive=disk0,addr=0x4,disable-modern=on,bootindex=1
    -global i440FX-pcihost.pci-hole64-size=32G
    -device qemu-xhci,id=xhci,addr=0x7 -device usb-tablet,bus=xhci.0
    -device "$KF3")
# ★ 2026-10-09, DIAGNOSTIC (default off; docs/design/V3_BAR0_TRACE_MODE.md): WIN_TRACE=1 records the
# boot with the VFIO reference's tracer, as win_vm.sh's WINVM_TRACE does — QEMU trace events into
# $RUN/trace.log (kf3's BAR0 read records also need KF3_BAR0_READ_TRACE=1 in the environment) and,
# with WIN_GSP_OBSERVER=1, the shared GSP observer into $RUN/gsp.jsonl (fresh per run directory).
if [ "${WIN_TRACE:-0}" = 1 ]; then
    ARGS+=(-msg timestamp=on -trace "events=$HERE/../trace-events-vfio-reference.txt,file=$RUN/trace.log")
    [ "${WIN_GSP_OBSERVER:-0}" = 1 ] && ARGS+=(-global "kf3-gpu.x-gsp-observer=$RUN/gsp.jsonl"
        -global "kf3-gpu.x-gsp-observer-seconds=${WIN_GSP_OBSERVER_SECONDS:-3600}")
fi
# the harness's flag list (pc_boundary_experiment.py FLAGS + pc_sdr_experiment.py) and KF3_DEFERRED_API
# 2026-10-10 (OWNER_RULINGS §AB): KF3_TSPACE, KF3_INCA_REFUSE and KF3_KERNEL_{NVDEC,NVENC,OFA}_CTX are
# deleted (their behaviour is hardwired ON); they are no longer listed.
FLAGS="KF3_WIN_USER_CHANNELS_PASSTHROUGH KF3_WIN_TWIN_DEFAPI_OBJECT KF3_TIMER_MAP KF3_KERNEL_GR_CE KF3_KERNEL_GR_WORK KF3_TRANSLATED_CE_RELAY KF3_DEFERRED_API KF3_PREEMPT_BIND_PROBE KF3_DISPLAY_IMP_ENABLE KF3_DISPLAY_CTRL_PROBE KF3_ZCULL_BIND_PROBE KF3_SW_RUNLIST_HOST_OWNED KF3_ASYNC_PREEMPT KF3_GFX_POOL_PROBE KF3_SW_RUNLIST_PROBE KF3_MEMORY_LIST_PROBE KF3_DISPLAY_SDR_COLOR KF3_DISPLAY_CAPS_PROBE KF3_DISPLAY_LUT_MIRROR KF3_DISPLAY_ILUT_OFFSET_256 ${PROD_EXTRA_FLAGS:-} ${WIN_FLAGS:-}"
# ★ 2026-10-11 (flag-ablation lane): WIN_DROP="KF3_A KF3_B" removes those flags from the list (they are not set at all)
for d in ${WIN_DROP:-}; do FLAGS=" $FLAGS "; FLAGS=${FLAGS// $d / }; done
ENV=(); for f in $FLAGS; do case $f in *=*) ENV+=("$f") ;; *) ENV+=("$f=1") ;; esac; done
python3 - "$RUN/command.json" "$KF3_REV" "$QBIN" "${ENV[@]}" -- "$QBIN" "${ARGS[@]}" <<'PY'
import json, sys, datetime
out, rev, qbin = sys.argv[1:4]
rest = sys.argv[4:]; i = rest.index('--')
env, argv = rest[:i], rest[i + 1:]
json.dump(dict(schema=1, launcher='scripts/bench/windows/windows_broker.sh', kf3_rev=rev, argv=argv,
               flags=dict(e.split('=', 1) for e in env),
               time_utc=datetime.datetime.now(datetime.timezone.utc).isoformat()),
          open(out, 'w'), indent=2)
PY
env "${ENV[@]}" setsid "$QBIN" "${ARGS[@]}" > "$RUN/qemu.log" 2>&1 < /dev/null &
QPID=$!
printf 'QPID=%s\nRUN=%s\nKF3_REV=%s\n' "$QPID" "$RUN" "$KF3_REV" > "$STATE"
say "QEMU pid $QPID, kf3 $KF3_REV, run $RUN (qemu.log, serial.log, command.json, marker.txt)"
if [ "${WIN_MAX_SECONDS:-0}" -gt 0 ]; then
    setsid bash -c "sleep $WIN_MAX_SECONDS; kill -0 $QPID 2>/dev/null && '$0' stop ${N:-desktop}" > /dev/null 2>&1 < /dev/null &
fi
setsid bash -c "while kill -0 $QPID 2>/dev/null; do sleep 5; done; echo \"WINDOWS_EXIT qemu-gone \$(date -Is) xid_after=\$(dmesg | grep -c 'NVRM: Xid')\" >> '$RUN/marker.txt'" > /dev/null 2>&1 < /dev/null &
cat <<EOF

  ┌ kayfabe Windows window: "$WTITLE" on ${SU}'s desktop ┐
  │ hover = absolute USB tablet; CTRL+ALT+G grab (PS/2 relative mouse); CTRL+ALT+F fullscreen
  │ guest PowerShell as SYSTEM: scripts/bench/windows/qga_run_ps.py ${N:-desktop} <file.ps1>
  │ stop: $0 stop ${N:-desktop}
  └────────────────────────────────────────────────────────────────────────────────────────────┘
EOF
exit 0
