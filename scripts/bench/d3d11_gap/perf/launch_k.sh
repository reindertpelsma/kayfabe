#!/usr/bin/env bash
# launch_k.sh — kayfabe launch for the Windows perf campaign (copy of live3/launch3.sh; owner config: 16 GiB RAM, 8 vCPU, fb 8192, BAR1 8 GiB, hv_* CPU).
# Differences: own run id (N=2995), own qcow2 overlay + OVMF_VARS (env OVL, VARS), kfapps.iso as a third CD, KF3 flags from env PERF_FLAGS (empty = production), own stop file.
# Caller: flock -o /tmp/kayfabe-fastguest.lock bash this.sh     Stop: touch /tmp/kf-stop-perf
set -euo pipefail
W=/var/lib/kf-windows-20261005; REV=${REV:-666ea966}; N=${N:-2995}
B=${BROKER_SCRIPT:-/data/claude-gap/broker_gap.sh}   # copy of windows_broker_claude_oglw.sh with BROKER_BACKEND override (default unchanged: wayland on the owner session)
D=$W/win11-full-b; TOOLS=$W/win11-full/kfwin11-tools.iso; RUN=$W/boundary-kayfabe-$N; STOPF=/tmp/kf-stop-perf
OVL=${OVL:?overlay qcow2}; VARS=${VARS:?OVMF_VARS copy}
export KF_LOCKED=1; . $D/identity.env
export BROKER_BIN=${BROKER_BIN:-/data/claude-resize/broker-bin/nvkvm-display-broker-666ea966}
test -x "$BROKER_BIN" && test -f "$OVL" && test -f "$VARS" && test -f "$TOOLS" && test -x "$W/kf3-bins/$REV/qemu-system-x86_64"
case "$(cat /sys/kernel/iommu_groups/11/type)" in DMA-FQ|identity) ;; *) echo bad-iommu-type; exit 13;; esac
test "$(df -B1 --output=avail /data | tail -1)" -gt 21474836480
need_kb=$(( (${RAM_MB:-16384} + 2048) * 1024 )); avail_kb=$(awk "/MemAvailable/{print \$2}" /proc/meminfo)
[ "$avail_kb" -ge "$need_kb" ] || { echo "host RAM: MemAvailable ${avail_kb} kB < ${need_kb} kB needed"; exit 14; }
vram_free=$(nvidia-smi --query-gpu=memory.free --format=csv,noheader,nounits | head -1)
[ "$vram_free" -ge $(( ${FB_MB:-8192} + 1536 )) ] || { echo "host VRAM: ${vram_free} MiB free"; exit 15; }
pgrep -f "^[^ ]*qemu-system-x86_64 " >/dev/null && { echo "a qemu is running"; exit 16; }
rm -f "$STOPF"; mkdir -p -m 0700 "$RUN"
ln -sfn "$OVL" "$RUN/windows.qcow2"; ln -sfn "$VARS" "$RUN/OVMF_VARS.fd"
cleanup() { rc=$?; trap - EXIT; bash "$B" stop "$N" || true; rm -f "$STOPF"; echo "PERF_SESSION_EXIT=$rc at=$(date -Is)"; }
trap cleanup EXIT
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n "/^KF3_/p")
export WIN_CPU="host,-vmx,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_vpindex,hv_runtime,hv_time,hv_synic,hv_stimer,hv_tlbflush,hv_ipi,hv_frequencies"
export WIN_NET_RESTRICT=off WIN_MAC=$VM_MAC WIN_GPU_UUID=$GPU_UUID
export KF3_REV=$REV WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS="${PERF_FLAGS:-}" PROD_EXTRA_FLAGS= WIN_DROP="KF3_TIMER_MAP" WIN_FB_MB=${FB_MB:-8192} WIN_RAM_MB=${RAM_MB:-16384} WIN_BAR1=${BAR1:-8589934592} WIN_REUSE=1
export WIN_QEMU_EXTRA="-uuid $VM_UUID -smbios type=1,manufacturer=KayfabeLab,product=KF-Guest,serial=$VM_SERIAL -drive file=$TOOLS,if=none,id=cd1,media=cdrom,readonly=on -device ide-cd,drive=cd1,bus=ide.0,unit=0 -drive file=/var/lib/vast-windows-test-media/virtio.iso,if=none,id=cd2,media=cdrom,readonly=on -device ide-cd,drive=cd2,bus=ide.0,unit=1 -drive file=$W/appmatrix/image/kfapps.iso,if=none,id=cd3,media=cdrom,readonly=on -device ide-cd,drive=cd3,bus=ide.1,unit=0 -fw_cfg name=opt/ovmf/X-PciMmio64Mb,string=32768"
export WIN_TITLE="KAYFABE PERF RUN $N ${REV^^}" WIN_MAX_SECONDS=0
bash "$B" run "$N"
QPID=$(sed -n "s/^QPID=//p" "$W/windows-broker-run$N.state")
echo "PERF_SESSION_READY=$(date -Is) run=$N rev=$REV flags=[${PERF_FLAGS:-}] qpid=$QPID"
while kill -0 "$QPID" 2>/dev/null; do
  if test -e "$STOPF"; then echo "PERF_SESSION_STOP_REQUEST=$(date -Is)"; break; fi
  sleep 5
done
