#!/usr/bin/env bash
# launch_v.sh — VFIO native reference session for the Windows perf campaign: detach the 4070 from nvidia, boot the SAME guest disk (overlay OVL, vars VARS), same RAM/vCPU/CPU flags/SMBIOS/CDs
# as launch_k.sh, stock KVM irqfd/ioeventfd (no observer, no tracing), wait for /tmp/kf-stop-perf, stop the guest gracefully, ALWAYS restore the GPU to nvidia (trap).
# Caller: flock -o /tmp/kayfabe-fastguest.lock bash this.sh      Stop: touch /tmp/kf-stop-perf
set -uo pipefail
W=/var/lib/kf-windows-20261005; REV=${REV:-666ea966}; QEMU=$W/kf3-bins/$REV/qemu-system-x86_64
OVL=${OVL:?}; VARS=${VARS:?}; R=${R:-/data/claude-gap/vfio/boot-$(date +%H%M%S)}; STOPF=/tmp/kf-stop-perf
GPU=0000:01:00.0; AUD=0000:01:00.1; PCI=/sys/bus/pci/devices
D=$W/win11-full-b; TOOLS=$W/win11-full/kfwin11-tools.iso; . $D/identity.env
say(){ printf "[vfio-perf] %s %s\n" "$(date +%T)" "$*"; }
drv(){ basename "$(readlink -f $PCI/$1/driver 2>/dev/null)" 2>/dev/null; }
xid(){ dmesg 2>/dev/null | grep -c "NVRM: Xid"; }
[ "$(id -u)" = 0 ] || { echo root; exit 2; }
mkdir -p -m 700 "$R"; JRN=$R/vfio-bind.txt; ln -sfn "$R" /data/claude-gap/vfio/latest
RESTORED=0
restore() {
  [ $RESTORED = 1 ] && return; RESTORED=1
  echo "RESTORE_START $(date -Is) gpu=$(drv $GPU) aud=$(drv $AUD)" | tee -a $JRN
  for b in $GPU $AUD; do echo > $PCI/$b/driver_override; [ -e $PCI/$b/driver ] && echo $b > $PCI/$b/driver/unbind; done
  modprobe nvidia; echo $GPU > /sys/bus/pci/drivers_probe; echo $AUD > /sys/bus/pci/drivers_probe
  modprobe nvidia_uvm; modprobe nvidia_modeset; modprobe nvidia_drm modeset=1 fbdev=1
  systemctl start nvidia-persistenced 2>/dev/null; sleep 3
  echo "RESTORE_END $(date -Is) gpu=$(drv $GPU) aud=$(drv $AUD) smi=$(nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>&1) xid=$(xid)" | tee -a $JRN
}
cleanup() { rc=$?; trap - EXIT
  if [ -n "${QPID:-}" ] && kill -0 $QPID 2>/dev/null; then
    say "stopping guest"; timeout 40 python3 /data/kfagent_server.py run "shutdown /s /t 0 /f; \"shutdown issued\"" 30 >/dev/null 2>&1
    for i in $(seq 90); do kill -0 $QPID 2>/dev/null || break; sleep 2; done
    if kill -0 $QPID 2>/dev/null; then python3 $W/boundary-tools/qmp.py $R/qmp.sock cmd system_powerdown >/dev/null 2>&1; for i in $(seq 90); do kill -0 $QPID 2>/dev/null || break; sleep 2; done; fi
    kill -0 $QPID 2>/dev/null && { echo "UNCLEAN: qemu still alive, quit via QMP" | tee -a $JRN; python3 $W/boundary-tools/qmp.py $R/qmp.sock cmd quit >/dev/null 2>&1; sleep 8; }
  fi
  echo "WINDOWS_EXIT $(date -Is) xid=$(xid)" >> $R/marker.txt
  restore; rm -f $STOPF; echo "VFIO_SESSION_EXIT=$rc at=$(date -Is)"; }
trap cleanup EXIT
pgrep -a qemu-system && { echo "a QEMU runs; refusing"; exit 2; }
test -f "$OVL" && test -f "$VARS" && test -x "$QEMU" || { echo "missing inputs"; exit 2; }
nodes=$(ls /dev/nvidia* 2>/dev/null; for c in /sys/class/drm/card* /sys/class/drm/renderD*; do [ "$(readlink -f $c/device 2>/dev/null)" = "$(readlink -f $PCI/$GPU)" ] && echo /dev/dri/$(basename $c); done)
{ echo "DETACH_START $(date -Is) gpu=$(drv $GPU) aud=$(drv $AUD) iommu=$(cat /sys/kernel/iommu_groups/11/type) xid=$(xid)"
  echo "drm cards of the 4070: $(for c in /sys/class/drm/card*; do [ "$(readlink -f $c/device)" = "$(readlink -f $PCI/$GPU)" ] && echo -n "$(basename $c) "; done)"
  echo "connectors connected: $(for c in /sys/class/drm/card*-*; do [ "$(cat $c/status 2>/dev/null)" = connected ] && echo -n "$(basename $c) "; done)"; } | tee -a $JRN
systemctl stop nvidia-persistenced 2>/dev/null
# shellcheck disable=SC2086
users=$(fuser $nodes 2>/dev/null | tr -s " "); if [ -n "$users" ]; then echo "4070 nodes open by: $users" | tee -a $JRN; fuser -v $nodes 2>&1 | tee -a $JRN; systemctl start nvidia-persistenced 2>/dev/null; RESTORED=1; exit 3; fi
for m in nvidia_drm nvidia_modeset nvidia_uvm; do modprobe -r $m || { echo "modprobe -r $m failed" | tee -a $JRN; exit 4; }; done
modprobe vfio-pci
for b in $GPU $AUD; do echo vfio-pci > $PCI/$b/driver_override; done
for b in $AUD $GPU; do [ -e $PCI/$b/driver ] && echo $b > $PCI/$b/driver/unbind; echo $b > /sys/bus/pci/drivers_probe; done
echo "DETACH_END $(date -Is) gpu=$(drv $GPU) aud=$(drv $AUD)" | tee -a $JRN
[ "$(drv $GPU)" = vfio-pci ] && [ "$(drv $AUD)" = vfio-pci ] || { echo "vfio-pci did not bind" | tee -a $JRN; exit 5; }
args=( "$QEMU" -name vfio-perf -nodefaults -no-user-config
  -machine pc,accel=kvm,smm=on,memory-backend=ram0 -object memory-backend-memfd,id=ram0,size=${RAM_MB:-16384}M,share=on -m ${RAM_MB:-16384} -smp 8
  -cpu host,-vmx,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_vpindex,hv_runtime,hv_time,hv_synic,hv_stimer,hv_tlbflush,hv_ipi,hv_frequencies
  -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd
  -drive if=pflash,format=raw,unit=1,file=$VARS
  -rtc base=utc,driftfix=slew -global kvm-pit.lost_tick_policy=discard
  -device VGA,addr=0x9 -display none -vnc unix:$R/vnc.sock
  -qmp unix:$R/qmp.sock,server=on,wait=off -serial file:$R/serial.log -monitor none -action reboot=reset,panic=none
  -netdev user,id=n0,restrict=off,net=10.0.2.0/24,host=10.0.2.2,dns=10.0.2.3,dhcpstart=10.0.2.15
  -device virtio-net-pci,netdev=n0,mac=$VM_MAC,addr=0xa,disable-modern=on
  -device virtio-serial-pci,id=serial0,addr=0x3,disable-modern=on
  -chardev socket,id=qga0,path=$R/qga.sock,server=on,wait=off
  -device virtserialport,bus=serial0.0,chardev=qga0,name=org.qemu.guest_agent.0
  -drive file=$OVL,if=none,id=disk0,format=qcow2,discard=unmap
  -device virtio-blk-pci,drive=disk0,addr=0x4,disable-modern=on,bootindex=1
  -global i440FX-pcihost.pci-hole64-size=32G
  -device qemu-xhci,id=xhci,addr=0x7 -device usb-tablet,bus=xhci.0 -device usb-kbd,bus=xhci.0,id=kfkbd
  -device vfio-pci,host=$GPU,bus=pci.0,addr=0x6.0,multifunction=on
  -device vfio-pci,host=$AUD,bus=pci.0,addr=0x6.1
  -uuid $VM_UUID -smbios type=1,manufacturer=KayfabeLab,product=KF-Guest,serial=$VM_SERIAL
  -drive file=$TOOLS,if=none,id=cd1,media=cdrom,readonly=on -device ide-cd,drive=cd1,bus=ide.0,unit=0
  -drive file=/var/lib/vast-windows-test-media/virtio.iso,if=none,id=cd2,media=cdrom,readonly=on -device ide-cd,drive=cd2,bus=ide.0,unit=1
  -drive file=$W/appmatrix/image/kfapps.iso,if=none,id=cd3,media=cdrom,readonly=on -device ide-cd,drive=cd3,bus=ide.1,unit=0
  -fw_cfg name=opt/ovmf/X-PciMmio64Mb,string=32768 )
printf "%s\n" "${args[@]}" | sed "s/serial=[A-Z0-9]*/serial=<x>/" > $R/command.txt
echo "WINDOWS_START $(date -Is) rev=$REV xid=$(xid)" >> $R/marker.txt
rm -f $STOPF
setsid "${args[@]}" > $R/qemu.log 2>&1 < /dev/null &
QPID=$!; echo $QPID > $R/qemu.pid; sleep 3
kill -0 $QPID || { tail -5 $R/qemu.log; exit 6; }
echo "VFIO_SESSION_READY=$(date -Is) qpid=$QPID dir=$R"
while kill -0 $QPID 2>/dev/null; do [ -e $STOPF ] && { echo "VFIO_SESSION_STOP_REQUEST=$(date -Is)"; break; }; sleep 5; done
