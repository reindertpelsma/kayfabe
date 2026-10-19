#!/usr/bin/env bash
# MEASUREMENT ONLY (trap experiments 2026-10-19). Copy of the TSG lane's win-launch.sh (itself a copy of the owner's perf5 launcher) with
# this lane's directories. Caller holds the GPU lock:  flock /tmp/kayfabe-fastguest.lock bash launch.sh   Stop: touch $ROOT/STOP_VM
# env: REV (dir under W/kf3-bins), N (run number), SIDE (disk dir under $ROOT/disk), WIN_FLAGS (the KF3_* flags, space separated)
set -euo pipefail
ROOT=/data/claude-trapexp-20261019
W=$ROOT/W; REAL=/var/lib/kf-windows-20261005; REV=${REV:?}; N=${N:-4100}; SIDE=${SIDE:?}; export WIN_DIR=$W
B=/data/claude-gap/broker_gap.sh
D=$ROOT/disk/$SIDE; TOOLS=$REAL/win11-full/kfwin11-tools.iso; RUN=$W/boundary-kayfabe-$N; STOPF=$ROOT/STOP_VM
export KF_LOCKED=1; . $D/identity.env
export BROKER_BIN=/data/claude-cursor6/nvkvm-display-broker
case "$(cat /sys/kernel/iommu_groups/11/type)" in DMA-FQ|identity) ;; *) echo bad-iommu-type; exit 13;; esac
test -f "$D/windows11.qcow2" && test -f "$D/OVMF_VARS.fd" && test -f "$TOOLS"
test -x "$W/kf3-bins/$REV/qemu-system-x86_64"
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 53687091200
rm -f /tmp/kf-stop-winprod "$STOPF"
rm -rf "$RUN"; mkdir -p -m 0700 "$RUN"
ln -sfn "$D/windows11.qcow2" "$RUN/windows.qcow2"; ln -sfn "$D/OVMF_VARS.fd" "$RUN/OVMF_VARS.fd"
cleanup() { rc=$?; trap - EXIT; bash "$B" stop "$N" || true; rm -f /tmp/kf-stop-winprod "$STOPF"; echo "OWNER_SESSION_EXIT=$rc at=$(date -Is)"; }
trap cleanup EXIT
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export WIN_CPU="host,-vmx,migratable=off,+invtsc,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_vpindex,hv_runtime,hv_time,hv_synic,hv_stimer,hv_tlbflush,hv_ipi,hv_frequencies"
export WIN_NET_RESTRICT=off WIN_MAC=$VM_MAC WIN_GPU_UUID=$GPU_UUID
export KF3_REV=$REV WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS="${WIN_FLAGS:-KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5}" PROD_EXTRA_FLAGS= WIN_DROP="KF3_TIMER_MAP" WIN_FB_MB=8192 WIN_RAM_MB=12288 WIN_BAR1=8589934592 WIN_REUSE=1
export WIN_QEMU_EXTRA="-uuid $VM_UUID -smbios type=1,manufacturer=KayfabeLab,product=KF-Guest,serial=$VM_SERIAL -drive file=$TOOLS,if=none,id=cd1,media=cdrom,readonly=on -device ide-cd,drive=cd1,bus=ide.0,unit=0 -drive file=/var/lib/vast-windows-test-media/virtio.iso,if=none,id=cd2,media=cdrom,readonly=on -device ide-cd,drive=cd2,bus=ide.0,unit=1"
export WIN_TITLE="kayfabe Windows 11 (trap-exp run $N, $REV)" WIN_MAX_SECONDS=0
bash "$B" run "$N"
QPID=$(sed -n 's/^QPID=//p' "$W/windows-broker-run$N.state")
echo "OWNER_SESSION_READY=$(date -Is) run=$N rev=$REV install=win11x64.iso qpid=$QPID"
while kill -0 "$QPID" 2>/dev/null; do
  if test -e "$STOPF"; then echo "OWNER_SESSION_STOP_REQUEST=$(date -Is)"; break; fi
  sleep 5
done
