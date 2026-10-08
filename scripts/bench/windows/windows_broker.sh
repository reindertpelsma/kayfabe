#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# shellcheck disable=SC1090,SC2054  # the state file is this script's own; commas are QEMU property lists
# windows_broker.sh — the Windows 11 guest (NVIDIA 580.88) on kf3, shown in a SECOND display-broker
# window on the host's live desktop, beside the Linux guest of scripts/bench/display/interactive.sh
# (its own socket, its own window title, its own run directory). 2026-10-08, the owner: "It's ok to
# have 2 broker screens on the 1.20 host."
#
#   windows_broker.sh run [N]     boot run N (default: the next free boundary-kayfabe-N) DETACHED:
#                                 a fresh overlay of the baseline, the broker window, QEMU; returns
#   windows_broker.sh desktop     boot the persistent desktop overlay ($W/windows-desktop) instead of
#                                 a fresh one (autologon is configured there once, see `autologon`)
#   windows_broker.sh stop        ACPI power-down of the running Windows guest (kill after 90 s),
#                                 then its broker window; the Linux guest is never touched
#   windows_broker.sh broker      (re)start only the Windows broker window (kf3 reconnects to it)
#   windows_broker.sh status      the running guest, its run directory, QGA ping, host Xid count
#   windows_broker.sh autologon   (desktop overlay, guest running) through QGA as SYSTEM: give the
#                                 local account 'kf' a password from $W/windows-desktop/secrets
#                                 (0600, generated here, never committed) and enable Winlogon autologon
#   windows_broker.sh nvidia disable|enable|status   (guest running) the guest's NVIDIA display device, through
#                                 QGA + pnputil (nvidia_device.ps1); takes effect at the next boot (`reboot`)
#   windows_broker.sh reboot      restart the guest (QGA, Restart-Computer -Force)
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
STATE=$W/windows-broker.state
cmd=${1:-}
say(){ printf '[windows] %s\n' "$*"; }
die(){ printf '[windows] ★ %s\n' "$*" >&2; exit 2; }
[ "$(id -u)" = 0 ] || die "run as root"

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
    SOCK=$SRUN/nvkvm/windows.sock
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
        --title "${WIN_TITLE:-kayfabe Windows}" ${BROKER_ARGS:-} > "$log" 2>&1 < /dev/null &
    for _ in $(seq 50); do [ -S "$SOCK" ] && break; sleep 0.2; done
    [ -S "$SOCK" ] || { tail -5 "$log" >&2; die "the broker did not create $SOCK (log $log)"; }
    say "broker: backend $backend, socket $SOCK, title '${WIN_TITLE:-kayfabe Windows}' (log $log)"
}

running_pid() { [ -s "$STATE" ] && . "$STATE" && [ -n "${QPID:-}" ] && kill -0 "$QPID" 2>/dev/null && echo "$QPID"; }

xid_count() { dmesg 2>/dev/null | grep -c 'NVRM: Xid'; }

case "$cmd" in
stop)
    session || true
    if q=$(running_pid); then
        . "$STATE"
        python3 "$TOOLS/qmp.py" "$RUN/qmp.sock" cmd system_powerdown >/dev/null 2>&1
        for _ in $(seq 90); do kill -0 "$q" 2>/dev/null || break; sleep 1; done
        kill -0 "$q" 2>/dev/null && { kill "$q"; sleep 3; kill -9 "$q" 2>/dev/null; say "QEMU $q killed after 90 s"; }
        echo "WINDOWS_EXIT stop $(date -Is)" >> "$RUN/marker.txt"
    else
        say "no Windows guest running"
    fi
    [ -n "${SOCK:-}" ] && broker_down
    say "stopped (Xid count now $(xid_count))"; exit 0 ;;
status)
    if q=$(running_pid); then
        . "$STATE"; say "running: QEMU $q, run $RUN, kf3 $KF3_REV, since $(stat -c %y "$RUN/marker.txt")"
        timeout 15 python3 "$TOOLS/qmp.py" "$RUN/qga.sock" qga-ping >/dev/null 2>&1 && say "QGA answers" || say "QGA silent"
    else say "no Windows guest running"; fi
    say "host Xid lines in dmesg: $(xid_count)"; exit 0 ;;
broker)
    session || die "no graphical session on seat0"
    broker_up "$W/windows-broker-$(date +%Y%m%d-%H%M%S).log"; exit 0 ;;
autologon)
    q=$(running_pid) || die "no Windows guest running"
    . "$STATE"
    [ "$RUN" = "$W/windows-desktop" ] || die "autologon only on the persistent desktop overlay (windows_broker.sh desktop)"
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
    running_pid > /dev/null || die "no Windows guest running"
    . "$STATE"
    act=${2:-status}; case "$act" in disable|enable|status) ;; *) die "nvidia disable|enable|status" ;; esac
    tmp=$(mktemp); printf '$Action = %s\n' "'$act'" > "$tmp"; cat "$HERE/nvidia_device.ps1" >> "$tmp"
    timeout 280 python3 "$TOOLS/qmp.py" "$RUN/qga.sock" qga-exec powershell.exe -NoProfile -Command "$(cat "$tmp")"
    rm -f "$tmp"; exit 0 ;;
reboot)
    running_pid > /dev/null || die "no Windows guest running"
    . "$STATE"
    timeout 30 python3 "$TOOLS/qmp.py" "$RUN/qga.sock" qga-exec powershell.exe -NoProfile -Command "Restart-Computer -Force"
    say "guest restarting (the window shows OVMF, then Windows)"; exit 0 ;;
run|desktop) ;;
*) die "usage: $0 run [N] | desktop | stop | broker | status | autologon | nvidia disable|enable|status | reboot" ;;
esac

# ── run / desktop ───────────────────────────────────────────────────────────────────────────────
[ -n "${KF3_REV:-}" ] || die "KF3_REV names the kf3 binary (under $W/kf3-bins)"
QBIN=$W/kf3-bins/$KF3_REV/qemu-system-x86_64
[ -x "$QBIN" ] || die "no kf3 binary at $QBIN"
q=$(running_pid) && die "a Windows guest is already running (QEMU $q); $0 stop first"
session || die "no graphical session on seat0"
if [ "$cmd" = desktop ]; then
    RUN=$W/windows-desktop; NAME=kayfabe-windows-desktop
    mkdir -p "$RUN"; chmod 0700 "$RUN"
    [ -e "$RUN/windows.qcow2" ] || qemu-img create -q -f qcow2 -F qcow2 -b "$W/baseline/windows.qcow2" "$RUN/windows.qcow2" || die "qemu-img"
    [ -e "$RUN/OVMF_VARS.fd" ] || cp "$W/baseline/OVMF_VARS.fd" "$RUN/OVMF_VARS.fd"
else
    N=${2:-}
    if [ -z "$N" ]; then N=54; while [ -e "$W/boundary-kayfabe-$N" ]; do N=$((N + 1)); done; fi
    RUN=$W/boundary-kayfabe-$N; NAME=boundary-kayfabe-$N
    [ -e "$RUN" ] && die "$RUN exists (a run directory is never reused)"
    mkdir -m 0700 "$RUN"
    qemu-img create -q -f qcow2 -F qcow2 -b "$W/baseline/windows.qcow2" "$RUN/windows.qcow2" || die "qemu-img"
    cp "$W/baseline/OVMF_VARS.fd" "$RUN/OVMF_VARS.fd"
fi
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
# the harness's flag list (pc_boundary_experiment.py FLAGS + pc_sdr_experiment.py) and KF3_DEFERRED_API
FLAGS="KF3_GFX_POOL_PROBE KF3_TIMER_MAP KF3_TSPACE KF3_SW_RUNLIST_PROBE KF3_MEMORY_LIST_PROBE KF3_DISPLAY_SDR_COLOR KF3_DISPLAY_METHOD_TRACE KF3_KERNEL_GR_CE KF3_KERNEL_NVDEC_CTX KF3_KERNEL_NVENC_CTX KF3_KERNEL_OFA_CTX KF3_KERNEL_GR_WORK KF3_SW_SUBCH_INERT KF3_TRANSLATED_CE_RELAY KF3_BAR0_TRACE KF3_MAPLOG KF3_DEFERRED_API ${WIN_FLAGS:-}"
ENV=(KF3_RPC_TRACE=1); for f in $FLAGS; do ENV+=("$f=1"); done
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
    setsid bash -c "sleep $WIN_MAX_SECONDS; kill -0 $QPID 2>/dev/null && '$0' stop" > /dev/null 2>&1 < /dev/null &
fi
setsid bash -c "while kill -0 $QPID 2>/dev/null; do sleep 5; done; echo \"WINDOWS_EXIT qemu-gone \$(date -Is) xid_after=\$(dmesg | grep -c 'NVRM: Xid')\" >> '$RUN/marker.txt'" > /dev/null 2>&1 < /dev/null &
cat <<EOF

  ┌ kayfabe Windows window: "${WIN_TITLE:-kayfabe Windows}" on ${SU}'s desktop (beside the Linux one) ┐
  │ hover = absolute USB tablet; CTRL+ALT+G grab (PS/2 relative mouse); CTRL+ALT+F fullscreen
  │ guest PowerShell as SYSTEM: scripts/bench/windows/qga_run_ps.py ${N:-desktop} <file.ps1>
  │ stop: $0 stop
  └────────────────────────────────────────────────────────────────────────────────────────────┘
EOF
exit 0
