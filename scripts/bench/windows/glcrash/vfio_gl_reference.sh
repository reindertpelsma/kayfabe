#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# vfio_refusal_ablation.sh — the 2026-10-09 refusal-ablation copy of the REFERENCE harness vfio_dvi_reference.sh.
#   Differences: QEMU = the ablation build (x-gsp-refuse, x-gsp-observer-seconds=7200); +qemu-xhci +usb-tablet (the scripted
#   gesture); +input_event_* trace events; VDR_REFUSE=FILE adds x-gsp-refuse=FILE; one session dir per boot (VDR_DIR).
# Original header follows.
# vfio_dvi_reference.sh — REFERENCE harness (2026-10-08): Windows 11 + NVIDIA 580.88 on the host RTX 4070 through
# vfio-pci (no kayfabe), the Philips 243V5 (DVI-D via a DVI-D-to-HDMI cable) on the 4070's HDMI-A-1 as its only
# display. Same guest, firmware, machine and patched QEMU (x-gsp-observer) as the 2026-10-05 boundary run vfio-10,
# plus timestamped QEMU trace events for every MSI/MSI-X/INTx delivery per vector (`-msg timestamp=on`: UTC
# ISO-8601 with microseconds, the same wall clock as the observer's `qpc`, QEMU_CLOCK_REALTIME ns).
#
#   vfio_dvi_reference.sh detach            bind 01:00.0 + 01:00.1 to vfio-pci (journal in $D/vfio-bind.txt).
#                                           GNOME runs on the iGPU (udev mutter-device-ignore on the 4070), so
#                                           gdm is NOT stopped; refuses if any process holds a 4070 node.
#   vfio_dvi_reference.sh restore           back to nvidia + snd_hda_intel, aux modules, nvidia-persistenced
#   vfio_dvi_reference.sh run N [fresh]     boot N (dir $D/bootN) DETACHED on the session disk $D/disk
#                                           (`fresh`: a new overlay of the vfio-10 baseline + its UEFI vars;
#                                           env VDR_TRACE_BAR0=1 adds vfio_region_read/write)
#   vfio_dvi_reference.sh stop N            clean stop: QGA shutdown, then ACPI (180 s each), only then kill (UNCLEAN)
#   vfio_dvi_reference.sh ps N FILE.ps1     run a PowerShell file in the guest through QGA (SYSTEM); output to $D/bootN
#   vfio_dvi_reference.sh clock N           host UTC vs guest-get-time, 5 samples (guest ETW FILETIME alignment)
#   vfio_dvi_reference.sh fetch N GUEST LOCAL   copy a guest file out through QGA guest-file-read
#
# The caller holds the GPU lock for the whole session (`flock -o /tmp/kayfabe-fastguest.lock …`); `run`
# refuses if nobody holds it. Never touches the kayfabe demos. `-action reboot=shutdown`: a guest reboot ends
# the QEMU process (each boot is its own trace).
set -uo pipefail
W=/var/lib/kf-windows-20261005
A=$W/vfio-ablation-20261009
D=${VDR_DIR:-$W/vfio-gl-20261011}
QEMU=${VDR_QEMU:-$A/bin/qemu-system-x86_64}   # ablation build: observer + x-gsp-refuse (NOT a kf3-bins binary)
TOOLS=$W/boundary-tools
GPU=0000:01:00.0; AUD=0000:01:00.1; PCI=/sys/bus/pci/devices
say(){ printf '[vdr] %s\n' "$*"; }
die(){ printf '[vdr] ★ %s\n' "$*" >&2; exit 2; }
[ "$(id -u)" = 0 ] || die "run as root"
drv(){ basename "$(readlink -f $PCI/$1/driver 2>/dev/null)" 2>/dev/null; }
xid(){ dmesg 2>/dev/null | grep -c 'NVRM: Xid'; }
qga_py() {   # $1 sock, $2 python body using q(cmd,args)
python3 - "$1" "$2" <<'PY'
import json, socket, sys, time, base64
s = socket.socket(socket.AF_UNIX); s.settimeout(60); s.connect(sys.argv[1]); f = s.makefile('rwb', buffering=0)
def q(c, a=None):
    m = {'execute': c}
    if a is not None: m['arguments'] = a
    f.write(json.dumps(m).encode() + b'\n')
    if c == 'guest-shutdown': return None
    return json.loads(f.readline())
tok = int(time.time()) & 0x7fffffff
f.write(json.dumps({'execute': 'guest-sync', 'arguments': {'id': tok}}).encode() + b'\n')
while json.loads(f.readline()).get('return') != tok: pass
exec(sys.argv[2])
PY
}
cmd=${1:-}
case "$cmd" in
detach)
    mkdir -p "$D"
    pgrep -a qemu-system && die "a QEMU runs; refusing"
    nodes=$(ls /dev/nvidia* 2>/dev/null; for c in /sys/class/drm/card* /sys/class/drm/renderD*; do
        [ "$(readlink -f $c/device 2>/dev/null)" = "$(readlink -f $PCI/$GPU)" ] && echo /dev/dri/$(basename $c); done)
    {
      echo "DETACH_START $(date -Is) gpu=$(drv $GPU) aud=$(drv $AUD) iommu=$(cat /sys/kernel/iommu_groups/11/type) xid=$(xid)"
      echo "connectors: $(for c in /sys/class/drm/card*-*; do [ "$(cat $c/status 2>/dev/null)" = connected ] && echo -n "$(basename $c) "; done)"
      echo "nvidia_drm modeset=$(cat /sys/module/nvidia_drm/parameters/modeset 2>/dev/null) fbdev=$(cat /sys/module/nvidia_drm/parameters/fbdev 2>/dev/null)"
    } | tee -a "$D/vfio-bind.txt"
    systemctl stop nvidia-persistenced 2>/dev/null
    # shellcheck disable=SC2086
    users=$(fuser $nodes 2>/dev/null | tr -s ' '); [ -n "$users" ] && die "4070 nodes still open by: $users ($(fuser -v $nodes 2>&1 | tail -n +2))"
    for m in nvidia_drm nvidia_modeset nvidia_uvm; do modprobe -r $m || die "modprobe -r $m failed"; done
    modprobe vfio-pci
    for b in $GPU $AUD; do echo vfio-pci > $PCI/$b/driver_override; done
    for b in $AUD $GPU; do [ -e $PCI/$b/driver ] && echo $b > $PCI/$b/driver/unbind; echo $b > /sys/bus/pci/drivers_probe; done
    echo "DETACH_END $(date -Is) gpu=$(drv $GPU) aud=$(drv $AUD)" | tee -a "$D/vfio-bind.txt"
    [ "$(drv $GPU)" = vfio-pci ] && [ "$(drv $AUD)" = vfio-pci ] || die "vfio-pci did not bind" ;;
restore)
    pgrep -f "qemu-system.*$D" >/dev/null && die "a QEMU of this session still runs"
    echo "RESTORE_START $(date -Is) gpu=$(drv $GPU) aud=$(drv $AUD)" | tee -a "$D/vfio-bind.txt"
    for b in $GPU $AUD; do echo > $PCI/$b/driver_override; [ -e $PCI/$b/driver ] && echo $b > $PCI/$b/driver/unbind; done
    modprobe nvidia; echo $GPU > /sys/bus/pci/drivers_probe; echo $AUD > /sys/bus/pci/drivers_probe
    modprobe nvidia_uvm; modprobe nvidia_modeset; modprobe nvidia_drm modeset=1 fbdev=1
    systemctl start nvidia-persistenced 2>/dev/null; sleep 3
    echo "RESTORE_END $(date -Is) gpu=$(drv $GPU) aud=$(drv $AUD) iommu=$(cat /sys/kernel/iommu_groups/11/type) smi=$(nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>&1) xid=$(xid)" | tee -a "$D/vfio-bind.txt" ;;
run)
    N=${2:?boot number}; R=$D/boot$N
    flock -n /tmp/kayfabe-fastguest.lock true && die "nobody holds /tmp/kayfabe-fastguest.lock: take it first (flock -o)"
    [ "$(drv $GPU)" = vfio-pci ] && [ "$(drv $AUD)" = vfio-pci ] || die "not bound to vfio-pci (detach first)"
    pgrep -a qemu-system && die "a QEMU runs; one GPU user at a time"
    [ -e "$R" ] && die "$R exists"
    mkdir -p -m 700 "$D/disk" "$R"
    if [ "${3:-}" = fresh ]; then
        [ -e "$D/disk/windows.qcow2" ] && die "session disk exists; not fresh"
        qemu-img create -f qcow2 -F qcow2 -b $W/baseline/windows.qcow2 "$D/disk/windows.qcow2" >/dev/null
        cp $W/baseline/OVMF_VARS.fd "$D/disk/OVMF_VARS.fd"
    fi
    [ -s "$D/disk/windows.qcow2" ] || die "no session disk (use fresh)"
    printf '%s\n' vfio_msi_interrupt vfio_intx_interrupt vfio_msix_vector_do_use vfio_msix_vector_release \
        vfio_msix_enable vfio_msix_disable vfio_msi_enable vfio_msi_disable vfio_intx_enable vfio_intx_disable \
        vfio_msix_early_setup vfio_msi_setup vfio_pci_reset vfio_pci_reset_flr vfio_pci_hot_reset_result > "$R/trace-events"
    # VDR_TRACE_BAR0=1: also every trapped BAR0 access (x-gsp-observer keeps BAR0 un-mmapped): doorbells, the interrupt
    # tree's leaf/top reads (which source raised each MSI) and enable writes. Large (GBs per boot); bounded by `stop`.
    printf '%s\n' input_event_btn input_event_abs input_event_rel input_event_sync input_event_key_qcode >> "$R/trace-events"
    [ "${VDR_TRACE_BAR0:-0}" = 1 ] && printf '%s\n' vfio_region_read vfio_region_write >> "$R/trace-events"
    args=( "$QEMU" -name vfio-dvi-boot$N -nodefaults -no-user-config
      -machine pc,accel=kvm,smm=on,memory-backend=ram0 -object memory-backend-memfd,id=ram0,size=8G,share=on -m 8192 -smp 8
      -cpu host,-vmx
      -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd
      -drive if=pflash,format=raw,unit=1,file=$D/disk/OVMF_VARS.fd
      -rtc base=utc,driftfix=slew -global kvm-pit.lost_tick_policy=discard
      -device VGA,addr=0x9 -display none -vnc unix:$R/vnc.sock
      -qmp unix:$R/qmp.sock,server=on,wait=off -serial file:$R/serial.log -monitor none
      -action reboot=shutdown,panic=none
      -netdev user,id=n0,restrict=off,net=10.0.2.0/24,host=10.0.2.2,dns=10.0.2.3,dhcpstart=10.0.2.15
      -device virtio-net-pci,netdev=n0,mac=52:54:00:56:77:20,addr=0xa,disable-modern=on
      -device virtio-serial-pci,id=serial0,addr=0x3,disable-modern=on
      -chardev socket,id=qga0,path=$R/qga.sock,server=on,wait=off
      -device virtserialport,bus=serial0.0,chardev=qga0,name=org.qemu.guest_agent.0
      -drive file=$D/disk/windows.qcow2,if=none,id=disk0,format=qcow2,discard=unmap
      -device virtio-blk-pci,drive=disk0,addr=0x4,disable-modern=on,bootindex=1
      -global i440FX-pcihost.pci-hole64-size=32G
      -device qemu-xhci,id=xhci,addr=0x7 -device usb-tablet,bus=xhci.0 -device usb-kbd,bus=xhci.0
      -device vfio-pci,host=$GPU,bus=pci.0,addr=0x6.0,multifunction=on,x-gsp-observer=$R/gsp.jsonl,x-gsp-observer-seconds=7200,${VDR_REFUSE:+x-gsp-refuse=$VDR_REFUSE,}x-no-kvm-intx=on,x-no-kvm-msi=on,x-no-kvm-msix=on,x-no-kvm-ioeventfd=on,x-no-vfio-ioeventfd=on,enable-migration=off
      -device vfio-pci,host=$AUD,bus=pci.0,addr=0x6.1
      -msg timestamp=on -trace events=$R/trace-events,file=$R/trace.log )
    python3 -c 'import json,sys; print(json.dumps({"schema":1,"harness":"scripts/bench/windows/glcrash/vfio_gl_reference.sh","boot":int(sys.argv[1]),"fresh":sys.argv[2]=="fresh","qemu":sys.argv[3],"baseline":"/var/lib/kf-windows-20261005/baseline/windows.qcow2 (vfio-10 baseline)","argv":sys.argv[4:]},indent=1))' \
        "$N" "${3:-reuse}" "$(sha256sum $QEMU | cut -c1-64)" "${args[@]}" > "$R/command.json"
    echo "WINDOWS_START $(date -Is) boot=$N ${3:-reuse} xid=$(xid) iommu=$(cat /sys/kernel/iommu_groups/11/type) connectors_host_view=n/a(vfio)" >> "$R/marker.txt"
    setsid nohup "${args[@]}" > "$R/qemu.log" 2>&1 < /dev/null &
    echo $! > "$R/qemu.pid"; sleep 2
    kill -0 "$(cat $R/qemu.pid)" || { tail -5 "$R/qemu.log"; die "QEMU died"; }
    say "boot $N: QEMU $(cat $R/qemu.pid), $R" ;;
stop)
    N=${2:?}; R=$D/boot$N; q=$(cat "$R/qemu.pid")
    kill -0 "$q" 2>/dev/null || { say "boot $N not running"; exit 0; }
    how=""
    timeout 30 bash "$0" _qga "$R/qga.sock" "q('guest-shutdown',{'mode':'powerdown'})" >/dev/null 2>&1
    for i in $(seq 180); do kill -0 "$q" 2>/dev/null || { how=qga; break; }; sleep 1; done
    if [ -z "$how" ]; then
        python3 $TOOLS/qmp.py "$R/qmp.sock" cmd system_powerdown >/dev/null 2>&1
        for i in $(seq 180); do kill -0 "$q" 2>/dev/null || { how=acpi; break; }; sleep 1; done
    fi
    if [ -z "$how" ]; then
        python3 $TOOLS/qmp.py "$R/qmp.sock" cmd quit >/dev/null 2>&1; sleep 10
        kill -0 "$q" 2>/dev/null && kill "$q"; how=UNCLEAN-quit
    fi
    echo "WINDOWS_EXIT stop=$how $(date -Is) xid=$(xid)" | tee -a "$R/marker.txt" ;;
ps)
    N=${2:?}; R=$D/boot$N; f=${3:?}
    python3 "$(dirname "$0")/qga_run_ps.py" "$R" "$f" "$R/ps-$(basename "$f" .ps1).out" ;;
clock)
    N=${2:?}; R=$D/boot$N
    qga_py "$R/qga.sock" '
for i in range(5):
    h0 = time.time_ns(); g = q("guest-get-time")["return"]; h1 = time.time_ns()
    print(f"CLOCK host_mid_ns={(h0+h1)//2} guest_ns={g} guest_minus_host_ms={(g-(h0+h1)//2)/1e6:.3f} rtt_ms={(h1-h0)/1e6:.3f}")
    time.sleep(0.2)' | tee -a "$R/clock.txt" ;;
fetch)
    N=${2:?}; R=$D/boot$N
    qga_py "$R/qga.sock" "
h = q('guest-file-open', {'path': r'''$3''', 'mode': 'rb'})['return']
out = open('$4', 'wb'); n = 0
while True:
    r = q('guest-file-read', {'handle': h, 'count': 1048576})['return']
    b = base64.b64decode(r['buf-b64']); out.write(b); n += len(b)
    if r['eof'] or not b: break
q('guest-file-close', {'handle': h}); print('FETCHED', n)" ;;
_qga) qga_py "$2" "$3" ;;
*) die "usage: $0 detach | restore | run N [fresh] | stop N | ps N FILE | clock N | fetch N GUEST LOCAL" ;;
esac
