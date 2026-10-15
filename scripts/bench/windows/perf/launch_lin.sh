#!/usr/bin/env bash
# launch_lin.sh MODE(K|V) — the LINUX desktop guest (broker-interactive/desktop.qcow2 overlay) for the perf campaign: K = kayfabe (kf3-gpu, production flags, broker window),
# V = VFIO native (vfio-pci of the 4070, detach + trap restore). Same overlay (OVL), vars, RAM, vCPU, tap network (guest 192.168.77.2), serial, QMP for both.
# Caller: flock -o /tmp/kayfabe-fastguest.lock bash this.sh MODE     Stop: touch /tmp/kf-stop-perf
set -uo pipefail
MODE=${1:?K|V}; W=/var/lib/kf-windows-20261005; REV=${REV:-666ea966}; QEMU=$W/kf3-bins/$REV/qemu-system-x86_64
OVL=${OVL:-/data/claude-perf/disk/L.qcow2}; VARS=${VARS:-/data/claude-perf/disk/L.vars.fd}; R=${R:-/data/claude-perf/linux/run-$MODE-$(date +%H%M%S)}; STOPF=/tmp/kf-stop-perf
GPU=0000:01:00.0; AUD=0000:01:00.1; PCI=/sys/bus/pci/devices; RAM=${RAM_MB:-8192}; SMP=${SMP:-6}
say(){ printf "[linux-perf] %s %s\n" "$(date +%T)" "$*"; }
drv(){ basename "$(readlink -f $PCI/$1/driver 2>/dev/null)" 2>/dev/null; }
xid(){ dmesg 2>/dev/null | grep -c "NVRM: Xid"; }
mkdir -p -m 700 "$R"; JRN=$R/marker.txt; ln -sfn "$R" /data/claude-perf/linux/latest; RESTORED=1; QPID=""
restore() {
  [ $RESTORED = 1 ] && return; RESTORED=1
  for b in $GPU $AUD; do echo > $PCI/$b/driver_override; [ -e $PCI/$b/driver ] && echo $b > $PCI/$b/driver/unbind; done
  modprobe nvidia; echo $GPU > /sys/bus/pci/drivers_probe; echo $AUD > /sys/bus/pci/drivers_probe
  modprobe nvidia_uvm; modprobe nvidia_modeset; modprobe nvidia_drm modeset=1 fbdev=1
  systemctl start nvidia-persistenced 2>/dev/null; sleep 3
  echo "RESTORE_END $(date -Is) gpu=$(drv $GPU) smi=$(nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>&1) xid=$(xid)" | tee -a $JRN
}
cleanup() { rc=$?; trap - EXIT
  if [ -n "$QPID" ] && kill -0 $QPID 2>/dev/null; then
    say "stopping guest (ssh poweroff)"; ssh -i /workspace/bench/guest_key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 ubuntu@192.168.77.2 'sudo systemctl poweroff' >/dev/null 2>&1
    for i in $(seq 60); do kill -0 $QPID 2>/dev/null || break; sleep 2; done
    kill -0 $QPID 2>/dev/null && { python3 $W/boundary-tools/qmp.py $R/qmp.sock cmd system_powerdown >/dev/null 2>&1; for i in $(seq 40); do kill -0 $QPID 2>/dev/null || break; sleep 2; done; }
    kill -0 $QPID 2>/dev/null && { echo "UNCLEAN: qemu alive, quit via QMP" | tee -a $JRN; python3 $W/boundary-tools/qmp.py $R/qmp.sock cmd quit >/dev/null 2>&1; sleep 6; }
  fi
  echo "LINUX_EXIT mode=$MODE $(date -Is) xid=$(xid)" >> $JRN
  restore; rm -f $STOPF; echo "LINUX_SESSION_EXIT=$rc at=$(date -Is)"; }
trap cleanup EXIT
pgrep -f "^[^ ]*qemu-system" >/dev/null && { echo "a QEMU runs; refusing"; exit 2; }
test -f $OVL && test -f $VARS && test -x $QEMU || { echo missing inputs; exit 2; }
COMMON=( -name linux-perf-$MODE -object memory-backend-memfd,id=ram0,size=${RAM}M,share=on -machine q35,accel=kvm,memory-backend=ram0 -m $RAM -cpu host -smp $SMP
  -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd -drive if=pflash,format=raw,unit=1,file=$VARS
  -drive if=virtio,file=$OVL,format=qcow2 -netdev tap,id=n0,ifname=nvktap0,script=no,downscript=no -device virtio-net-pci,netdev=n0,mac=52:54:00:12:34:56,romfile=
  -vga none )
COMMON2=( -device virtio-keyboard-pci -device virtio-mouse-pci -display none -msg timestamp=on
  -qmp unix:$R/qmp.sock,server=on,wait=off -serial file:$R/serial.log -monitor unix:$R/qemu.mon,server,nowait )
if [ $MODE = V ]; then
  RESTORED=0
  { echo "DETACH_START $(date -Is) gpu=$(drv $GPU) xid=$(xid)"; } | tee -a $JRN
  nodes=$(ls /dev/nvidia* 2>/dev/null; for c in /sys/class/drm/card* /sys/class/drm/renderD*; do [ "$(readlink -f $c/device 2>/dev/null)" = "$(readlink -f $PCI/$GPU)" ] && echo /dev/dri/$(basename $c); done)
  systemctl stop nvidia-persistenced 2>/dev/null
  users=$(fuser $nodes 2>/dev/null | tr -s " "); if [ -n "$users" ]; then echo "4070 nodes open by: $users" | tee -a $JRN; fuser -v $nodes 2>&1 | tee -a $JRN; systemctl start nvidia-persistenced 2>/dev/null; RESTORED=1; exit 3; fi
  for m in nvidia_drm nvidia_modeset nvidia_uvm; do modprobe -r $m || { echo "modprobe -r $m failed" | tee -a $JRN; exit 4; }; done
  modprobe vfio-pci; for b in $GPU $AUD; do echo vfio-pci > $PCI/$b/driver_override; done
  for b in $AUD $GPU; do [ -e $PCI/$b/driver ] && echo $b > $PCI/$b/driver/unbind; echo $b > /sys/bus/pci/drivers_probe; done
  echo "DETACH_END $(date -Is) gpu=$(drv $GPU)" | tee -a $JRN
  [ "$(drv $GPU)" = vfio-pci ] && [ "$(drv $AUD)" = vfio-pci ] || { echo "vfio-pci did not bind" | tee -a $JRN; exit 5; }
  ARGS=( "${COMMON[@]}" -device vfio-pci,host=$GPU,bus=pcie.0,addr=0x2.0,multifunction=on -device vfio-pci,host=$AUD,bus=pcie.0,addr=0x2.1 "${COMMON2[@]}" -device virtio-tablet-pci )
  UNAME="$QEMU"
else
  SU=ubuntu; SUID=$(id -u $SU); SOCK=/run/user/$SUID/nvkvm/display.sock
  [ -S $SOCK ] || { echo "no broker socket $SOCK: run interactive.sh broker first"; exit 6; }
  ARGS=( "${COMMON[@]}" -device "kf3-gpu,id=kf0,guest-driver=580.159.04,fb-mb=8192,bar1-size=134217728,bar2-size=33554432,display=on,gop=on,x11-dispsw=on,display-broker=$SOCK,display-broker-uid=$SUID${KF3_EXTRA:+,$KF3_EXTRA}" "${COMMON2[@]}" -device virtio-tablet-pci,display=kf0,head=0 )
fi
echo "LINUX_START $(date -Is) mode=$MODE rev=$REV xid=$(xid)" >> $JRN
printf "%q " "$QEMU" "${ARGS[@]}" > $R/cmdline.txt
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n "/^KF3_/p")
rm -f $STOPF
if [ $MODE = K ]; then env ${PERF_FLAGS:-} setsid "$QEMU" "${ARGS[@]}" > $R/qemu.log 2>&1 < /dev/null & else setsid "$QEMU" "${ARGS[@]}" > $R/qemu.log 2>&1 < /dev/null & fi
QPID=$!; echo $QPID > $R/qemu.pid; sleep 3
kill -0 $QPID || { tail -5 $R/qemu.log; exit 7; }
echo "LINUX_SESSION_READY=$(date -Is) mode=$MODE qpid=$QPID dir=$R"
while kill -0 $QPID 2>/dev/null; do [ -e $STOPF ] && { echo "LINUX_STOP_REQUEST=$(date -Is)"; break; }; sleep 5; done
