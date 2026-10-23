#!/usr/bin/env bash
# Host-campaign 2026-10-23 arm holder. Derived from /data/claude-tsgmirror-20261018/win-launch.sh (same launcher broker_gap.sh, same WIN_* env).
# Usage: flock -o /tmp/kayfabe-fastguest.lock bash hold.sh N REV on|off|na [broker_bin]
# Stop: touch /data/claude-hc-20261023/stop-N
set -euo pipefail
N=$1; REV=$2; MIRROR=$3; BROKER_BIN_ARG=${4:-/data/claude-cursor6/nvkvm-display-broker}
L=/data/claude-hc-20261023; W=$L/W; export WIN_DIR=$W
B=/data/claude-gap/broker_gap.sh; REAL=/var/lib/kf-windows-20261005
OWNERDISK=/data/claude-owner-overlay/windows-overlay.qcow2; OWNERVARS=/data/claude-owner-overlay/OVMF_VARS.fd
TOOLS=$REAL/win11-full/kfwin11-tools.iso; RUN=$W/boundary-kayfabe-$N; STOPF=$L/stop-$N
export KF_LOCKED=1; . $L/identity.env
export BROKER_BIN=$BROKER_BIN_ARG
case "$(cat /sys/kernel/iommu_groups/11/type)" in DMA-FQ|identity) ;; *) echo bad-iommu-type; exit 13;; esac
test -x "$W/kf3-bins/$REV/qemu-system-x86_64" && test -f "$TOOLS" && test -f $OWNERDISK
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 53687091200
test "$(awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo)" -gt 20000 || { echo "RAM_REFUSED"; exit 14; }
! pgrep -x qemu-system-x86_64 >/dev/null || { echo "ANOTHER_QEMU_RUNNING_REFUSED"; exit 15; }
rm -f "$STOPF"
mkdir -p -m 0700 "$RUN"
qemu-img create -q -f qcow2 -F qcow2 -b $OWNERDISK $RUN/windows.qcow2
cp $OWNERVARS $RUN/OVMF_VARS.fd
cleanup() { rc=$?; trap - EXIT; bash "$B" stop "$N" || true; rm -f "$STOPF"; echo "HOLD_EXIT=$rc at=$(date -Is)"; }
trap cleanup EXIT
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export WIN_CPU="host,-vmx,migratable=off,+invtsc,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_vpindex,hv_runtime,hv_time,hv_synic,hv_stimer,hv_tlbflush,hv_ipi,hv_frequencies"
export WIN_NET_RESTRICT=off WIN_MAC=$VM_MAC WIN_GPU_UUID=$GPU_UUID
FL="KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5"
[ "$MIRROR" = off ] && FL="$FL KF3_TSG_MIRROR=0"
export KF3_REV=$REV WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS="$FL" PROD_EXTRA_FLAGS= WIN_DROP="KF3_TIMER_MAP" WIN_FB_MB=8192 WIN_RAM_MB=12288 WIN_BAR1=8589934592 WIN_REUSE=1
export WIN_QEMU_EXTRA="-uuid $VM_UUID -smbios type=1,manufacturer=KayfabeLab,product=KF-Guest,serial=$VM_SERIAL -drive file=$TOOLS,if=none,id=cd1,media=cdrom,readonly=on -device ide-cd,drive=cd1,bus=ide.0,unit=0 -drive file=/var/lib/vast-windows-test-media/virtio.iso,if=none,id=cd2,media=cdrom,readonly=on -device ide-cd,drive=cd2,bus=ide.0,unit=1"
export WIN_TITLE="hc20261023 run $N $REV mirror=$MIRROR" WIN_MAX_SECONDS=0
echo "XID_BEFORE=$(dmesg | grep -c 'NVRM: Xid')" > $RUN/hc-xid-before.txt
bash "$B" run "$N"
QPID=$(sed -n 's/^QPID=//p' "$W/windows-broker-run$N.state")
echo "HOLD_READY=$(date -Is) run=$N rev=$REV mirror=$MIRROR qpid=$QPID"
while kill -0 "$QPID" 2>/dev/null; do
  if test -e "$STOPF"; then echo "HOLD_STOP_REQUEST=$(date -Is)"; break; fi
  sleep 3
done
