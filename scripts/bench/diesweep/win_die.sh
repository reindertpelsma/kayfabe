#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# win_die.sh — boot the Windows LTSC baseline under kf3 on a RENTED box (vast KVM VM, no desktop
# session, no display broker) and collect per-die evidence. 2026-10-14, die sweep lane
# (docs/design/V3_DIE_SWEEP_20261014.md). Runs ON the box, as root, from the kayfabe checkout.
#
#   win_die.sh start            boot $RUN's overlay (created on first use over $W/base.qcow2) DETACHED
#   win_die.sh wait-qga SECS    wait until the guest agent answers (exit 1 on timeout)
#   win_die.sh ps FILE.ps1      run a PowerShell file in the guest as SYSTEM (QGA, -EncodedCommand)
#   win_die.sh shot NAME        QMP screendump of kf0 (kf3's own console surface) -> $RUN/shots/NAME.ppm.gz
#   win_die.sh stop             guest shutdown (QGA), then ACPI, then kill (logged)
#   win_die.sh status           QEMU alive? QGA? marker lines
#
# The guest and kf3 arguments are windows_broker.sh's (machine pc, 8 GiB, 8 vCPU, virtio-blk/net, QGA,
# kf3 fb-mb=4096 bar1-size=128 MiB gop=on) and the owner host's PRODUCTION flag profile of 2026-10-13
# (host copy windows_broker_claude_inst2.sh, run 2990 at bf510bdf: class-B flags, no measurement flags,
# no KF3_RPC_TRACE, KF3_TIMER_MAP dropped), with these differences, each forced by the box:
#   - no display broker (no graphical session on a rented VM): kf3 display=on draws into its QEMU
#     console, which `screendump device=kf0` reads — the same surface the broker window shows;
#   - no IOMMU-group switch (the rented VM has no IOMMU group to switch);
#   - OVMF code and the baseline's VARS come from the owner host ($W/OVMF_CODE_4M.fd), not the box's
#     distro, so the firmware matches the varstore the image was installed with.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
K="$(cd "$HERE/../../.." && pwd)"
W=${WIN_DIR:-/root/win}
RUN=${WIN_RUN:-$W/run}
QMP=$K/scripts/bench/windows/qmp.py
say(){ printf '[win_die] %s\n' "$*"; }
die(){ printf '[win_die] FAIL %s\n' "$*" >&2; exit 2; }
qpid(){ [ -s "$RUN/qpid" ] && kill -0 "$(cat "$RUN/qpid")" 2>/dev/null && cat "$RUN/qpid"; }

case "${1:-}" in
start)
    q=$(qpid) && die "QEMU $q already runs $RUN"
    QBIN=${QBIN:-$(ls -1t /workspace/bench/kf3-bins/*/qemu-system-x86_64 2>/dev/null | head -1)}
    [ -x "$QBIN" ] || die "no kf3 binary (QBIN)"
    REV=$(basename "$(dirname "$QBIN")")
    mkdir -p -m 0700 "$RUN" "$RUN/shots"
    if [ ! -e "$RUN/windows.qcow2" ]; then
        QIMG=$(command -v qemu-img || ls /workspace/bench/qemu-build-kf3/qemu-img 2>/dev/null)
        "$QIMG" create -q -f qcow2 -F qcow2 -b "$W/base.qcow2" "$RUN/windows.qcow2" || die qemu-img
        cp "$W/OVMF_VARS.fd" "$RUN/OVMF_VARS.fd"
    fi
    rm -f "$RUN/qmp.sock" "$RUN/qga.sock"
    [ -e "$RUN/qemu.log" ] && mv "$RUN/qemu.log" "$RUN/qemu-$(date +%H%M%S).log"
    # WIN_KF3_DISPLAY="display=off" + WIN_STDVGA=1: GB20x, whose kf3 display plane is refused at realize (doc §3)
    KF3="kf3-gpu,fb-mb=${WIN_FB_MB:-4096},bar1-size=134217728,bar2-size=33554432,${WIN_KF3_DISPLAY:-display=on,gop=on},guest-driver=580.65.06,bus=pci.0,addr=0x6,id=kf0"
    VGA=(); [ "${WIN_STDVGA:-0}" = 1 ] && VGA=(-device VGA,addr=0x9,id=vga0)
    ARGS=(-name kayfabe-diesweep -nodefaults -no-user-config
        -machine pc,accel=kvm,smm=on,memory-backend=ram0
        -object "memory-backend-memfd,id=ram0,size=${WIN_RAM_MB:-8192}M,share=on" -m "${WIN_RAM_MB:-8192}" -smp 8
        -cpu "${WIN_CPU:-host,-vmx,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_vpindex,hv_runtime,hv_time,hv_synic,hv_stimer,hv_tlbflush,hv_ipi,hv_frequencies}"
        -drive "if=pflash,format=raw,unit=0,readonly=on,file=$W/OVMF_CODE_4M.fd"
        -drive "if=pflash,format=raw,unit=1,file=$RUN/OVMF_VARS.fd"
        -rtc base=utc,driftfix=slew -global kvm-pit.lost_tick_policy=discard
        "${VGA[@]}" -display none
        -qmp "unix:$RUN/qmp.sock,server=on,wait=off"
        -serial "file:$RUN/serial.log" -monitor none -action reboot=reset,panic=none
        -netdev "user,id=n0,restrict=${WIN_NET_RESTRICT:-on}"
        -device virtio-net-pci,netdev=n0,addr=0xa,disable-modern=on
        -device virtio-serial-pci,id=serial0,addr=0x3,disable-modern=on
        -chardev "socket,id=qga0,path=$RUN/qga.sock,server=on,wait=off"
        -device virtserialport,bus=serial0.0,chardev=qga0,name=org.qemu.guest_agent.0
        -drive "file=$RUN/windows.qcow2,if=none,id=disk0,format=qcow2,discard=unmap"
        -device virtio-blk-pci,drive=disk0,addr=0x4,disable-modern=on,bootindex=1
        -global i440FX-pcihost.pci-hole64-size=32G
        -device qemu-xhci,id=xhci,addr=0x7 -device usb-tablet,bus=xhci.0
        -device "$KF3")
    FLAGS="KF3_WIN_USER_CHANNELS_PASSTHROUGH KF3_WIN_TWIN_DEFAPI_OBJECT KF3_KERNEL_GR_CE KF3_KERNEL_GR_WORK KF3_TRANSLATED_CE_RELAY KF3_DEFERRED_API KF3_PREEMPT_BIND_PROBE KF3_DISPLAY_IMP_ENABLE KF3_DISPLAY_CTRL_PROBE KF3_ZCULL_BIND_PROBE KF3_SW_RUNLIST_HOST_OWNED KF3_ASYNC_PREEMPT KF3_GFX_POOL_PROBE KF3_SW_RUNLIST_PROBE KF3_MEMORY_LIST_PROBE KF3_DISPLAY_SDR_COLOR KF3_DISPLAY_CAPS_PROBE KF3_DISPLAY_LUT_MIRROR KF3_DISPLAY_ILUT_OFFSET_256 ${WIN_FLAGS:-}"
    while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
    ENV=(); for f in $FLAGS; do case $f in *=*) ENV+=("$f") ;; *) ENV+=("$f=1") ;; esac; done
    printf '%s\n' "${ENV[@]}" "WIN_KF3_DISPLAY=${WIN_KF3_DISPLAY:-display=on,gop=on}" "WIN_STDVGA=${WIN_STDVGA:-0}" > "$RUN/flags.txt"; printf '%q ' "$QBIN" "${ARGS[@]}" > "$RUN/argv.txt"
    echo "WIN_DIE_START kf3=$REV checkout=$(git -C "$K" rev-parse --short=9 HEAD) gpu=$(nvidia-smi --query-gpu=name --format=csv,noheader) xid_before=$(dmesg | grep -c 'NVRM: Xid') $(date -Is)" | tee -a "$RUN/marker.txt"
    env "${ENV[@]}" setsid "$QBIN" "${ARGS[@]}" > "$RUN/qemu.log" 2>&1 < /dev/null &
    QPID=$!; echo "$QPID" > "$RUN/qpid"
    setsid bash -c "while kill -0 $QPID 2>/dev/null; do sleep 5; done; echo \"WIN_DIE_EXIT qemu-gone \$(date -Is) xid_after=\$(dmesg | grep -c 'NVRM: Xid')\" >> '$RUN/marker.txt'" > /dev/null 2>&1 < /dev/null &
    say "QEMU $QPID kf3 $REV run $RUN"; exit 0 ;;
wait-qga)
    end=$(( $(date +%s) + ${2:-600} ))
    while [ "$(date +%s)" -lt "$end" ]; do
        qpid > /dev/null || die "QEMU is gone"
        timeout 10 python3 "$QMP" "$RUN/qga.sock" qga-ping > /dev/null 2>&1 && { say "QGA answers $(date -Is)"; exit 0; }
        sleep 5
    done
    die "QGA silent after ${2:-600} s" ;;
ps)
    [ -f "${2:-}" ] || die "ps FILE.ps1"
    enc=$(iconv -f utf-8 -t utf-16le < "$2" | base64 -w0)
    timeout "${3:-300}" python3 "$QMP" "$RUN/qga.sock" qga-exec powershell.exe -NoProfile -ExecutionPolicy Bypass -EncodedCommand "$enc"; exit $? ;;
shot)
    n=${2:-shot}
    python3 "$QMP" "$RUN/qmp.sock" cmd screendump "{\"filename\":\"$RUN/shots/$n.ppm\",\"device\":\"${WIN_SHOT_DEV:-kf0}\"}" > /dev/null \
        && gzip -f "$RUN/shots/$n.ppm" && say "shot $RUN/shots/$n.ppm.gz $(date -Is)" || die "screendump failed"
    exit 0 ;;
stop)
    q=$(qpid) || { say "no QEMU"; exit 0; }
    timeout 30 python3 "$QMP" "$RUN/qga.sock" qga-exec shutdown.exe /s /f /t 0 > /dev/null 2>&1
    how=qga; for _ in $(seq 120); do kill -0 "$q" 2>/dev/null || break; sleep 1; done
    if kill -0 "$q" 2>/dev/null; then
        python3 "$QMP" "$RUN/qmp.sock" cmd system_powerdown > /dev/null 2>&1; how=acpi
        for _ in $(seq 90); do kill -0 "$q" 2>/dev/null || break; sleep 1; done
    fi
    kill -0 "$q" 2>/dev/null && { kill "$q"; sleep 5; kill -9 "$q" 2>/dev/null; how=UNCLEAN-kill; }
    echo "WIN_DIE_STOP how=$how $(date -Is)" | tee -a "$RUN/marker.txt"; exit 0 ;;
status)
    q=$(qpid) && say "QEMU $q" || say "no QEMU"
    timeout 10 python3 "$QMP" "$RUN/qga.sock" qga-ping > /dev/null 2>&1 && say "QGA answers" || say "QGA silent"
    tail -4 "$RUN/marker.txt" 2>/dev/null; exit 0 ;;
*) die "usage: $0 start|wait-qga SECS|ps FILE|shot NAME|stop|status" ;;
esac
