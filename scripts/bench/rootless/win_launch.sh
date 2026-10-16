#!/usr/bin/env bash
# win_launch.sh — start the Win11 guest on kf3 as the CURRENT uid (root or unprivileged), no root-only step.
# Rootless audit 2026-10-16 (traces/rootless_audit_20261016). Mirrors the QEMU line of
# scripts/bench/windows/windows_broker.sh (production flag list minus KF3_TIMER_MAP, as the perf lane's K config)
# with these differences, each forced by "no root":
#   - the display broker is started here, as the same uid, with --backend test (no compositor needed);
#   - the run directory, overlay and OVMF_VARS are given by the caller (user-owned);
#   - no runuser, no pgrep/pkill of other uids, no iommu-group or dmesg reads.
# usage: INST=/path/to/inst RUN=/path/to/rundir OVL=overlay.qcow2 VARS=OVMF_VARS.fd [RAM_MB=16384] win_launch.sh start|stop
set -euo pipefail
W=/var/lib/kf-windows-20261005
INST=${INST:?install dir (bin/qemu-system-x86_64, bin/nvkvm-display-broker)}
RUN=${RUN:?run dir}; OVL=${OVL:?}; VARS=${VARS:?}
RAM_MB=${RAM_MB:-16384}; FB_MB=${FB_MB:-8192}; BAR1=${BAR1:-8589934592}
. "${IDENTITY:-$W/win11-full-b/identity.env}"
TOOLS=${TOOLS:-$W/win11-full/kfwin11-tools.iso}; APPS=${APPS:-$W/appmatrix/image/kfapps.iso}
case "${1:-start}" in
stop)
  [ -s "$RUN/qemu.pid" ] && kill -0 "$(cat "$RUN/qemu.pid")" 2>/dev/null && kill "$(cat "$RUN/qemu.pid")"
  [ -s "$RUN/broker.pid" ] && kill -- "-$(cat "$RUN/broker.pid")" 2>/dev/null || true
  exit 0;;
esac
mkdir -p -m 0700 "$RUN"; rm -f "$RUN"/qmp.sock "$RUN"/qga.sock "$RUN"/display.sock
SOCK=$RUN/display.sock; UIDN=$(id -u)
# the test backend takes its events from stdin and exits on EOF: keep stdin open with tail -f /dev/null
setsid sh -c 'tail -f /dev/null | exec "$0" --socket "$1" --backend test --persist --verbose --title rootless-audit' \
   "$INST/bin/nvkvm-display-broker" "$SOCK" > "$RUN/broker.log" 2>&1 < /dev/null & echo $! > "$RUN/broker.pid"
for _ in $(seq 50); do [ -S "$SOCK" ] && break; sleep 0.2; done
[ -S "$SOCK" ] || { echo "broker did not create $SOCK"; tail -5 "$RUN/broker.log"; exit 2; }
KF3="kf3-gpu,gpu-uuid=$GPU_UUID,fb-mb=$FB_MB,bar1-size=$BAR1,bar2-size=33554432,display=on,guest-driver=580.65.06,bus=pci.0,addr=0x6,id=kf0,display-broker=$SOCK,display-broker-uid=$UIDN,gop=on"
ARGS=(-name "rootless-audit-$UIDN" -nodefaults -no-user-config
  -machine pc,accel=kvm,smm=on,memory-backend=ram0
  -object "memory-backend-memfd,id=ram0,size=${RAM_MB}M,share=on" -m "$RAM_MB" -smp 8
  -cpu "host,-vmx,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_vpindex,hv_runtime,hv_time,hv_synic,hv_stimer,hv_tlbflush,hv_ipi,hv_frequencies"
  -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd
  -drive "if=pflash,format=raw,unit=1,file=$VARS"
  -rtc base=utc,driftfix=slew -global kvm-pit.lost_tick_policy=discard
  -display none -qmp "unix:$RUN/qmp.sock,server=on,wait=off"
  -serial "file:$RUN/serial.log" -monitor none -action reboot=reset,panic=none
  -netdev "user,id=n0,restrict=off,net=10.0.2.0/24,host=10.0.2.2,dns=10.0.2.3,dhcpstart=10.0.2.15"
  -device virtio-net-pci,netdev=n0,mac=$VM_MAC,addr=0xa,disable-modern=on
  -device virtio-serial-pci,id=serial0,addr=0x3,disable-modern=on
  -chardev "socket,id=qga0,path=$RUN/qga.sock,server=on,wait=off"
  -device virtserialport,bus=serial0.0,chardev=qga0,name=org.qemu.guest_agent.0
  -drive "file=$OVL,if=none,id=disk0,format=qcow2,discard=unmap"
  -device virtio-blk-pci,drive=disk0,addr=0x4,disable-modern=on,bootindex=1
  -global i440FX-pcihost.pci-hole64-size=32G
  -device qemu-xhci,id=xhci,addr=0x7 -device usb-tablet,bus=xhci.0 -device usb-kbd,bus=xhci.0,id=kfkbd
  -device "$KF3"
  -uuid "$VM_UUID" -smbios "type=1,manufacturer=KayfabeLab,product=KF-Guest,serial=$VM_SERIAL"
  -drive "file=$TOOLS,if=none,id=cd1,media=cdrom,readonly=on" -device ide-cd,drive=cd1,bus=ide.0,unit=0
  -drive "file=$APPS,if=none,id=cd3,media=cdrom,readonly=on" -device ide-cd,drive=cd3,bus=ide.1,unit=0
  -fw_cfg name=opt/ovmf/X-PciMmio64Mb,string=32768)
FLAGS="KF3_WIN_USER_CHANNELS_PASSTHROUGH KF3_WIN_TWIN_DEFAPI_OBJECT KF3_KERNEL_GR_CE KF3_KERNEL_GR_WORK KF3_TRANSLATED_CE_RELAY KF3_DEFERRED_API KF3_PREEMPT_BIND_PROBE KF3_DISPLAY_IMP_ENABLE KF3_DISPLAY_CTRL_PROBE KF3_ZCULL_BIND_PROBE KF3_SW_RUNLIST_HOST_OWNED KF3_ASYNC_PREEMPT KF3_GFX_POOL_PROBE KF3_SW_RUNLIST_PROBE KF3_MEMORY_LIST_PROBE KF3_DISPLAY_SDR_COLOR KF3_DISPLAY_CAPS_PROBE KF3_DISPLAY_LUT_MIRROR KF3_DISPLAY_ILUT_OFFSET_256"
ENV=(); for f in $FLAGS; do ENV+=("$f=1"); done
printf '%s\n' "${ENV[@]}" > "$RUN/flags.txt"; printf '%q ' "${ARGS[@]}" > "$RUN/argv.txt"
env "${ENV[@]}" setsid "$INST/bin/qemu-system-x86_64" "${ARGS[@]}" > "$RUN/qemu.log" 2>&1 < /dev/null &
echo $! > "$RUN/qemu.pid"
sleep 3; kill -0 "$(cat "$RUN/qemu.pid")" || { echo "qemu died"; tail -20 "$RUN/qemu.log"; exit 3; }
echo "WIN_STARTED uid=$UIDN qpid=$(cat "$RUN/qemu.pid") run=$RUN $(date -Is)"
