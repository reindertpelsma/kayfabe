#!/usr/bin/env bash
# perf profile (2026-10-17): the 297 fps configuration of the D3D11 gap work (rev 134dcb54, scan-off after 5080 + lazy observe 16, 8 GiB BAR1, 8 GiB store, 16 GiB RAM), no counters/tracing; the owner apps VM disk (win11-full-b).
# v2 (2026-10-17 16:15): 12 GiB RAM (host memory pressure at 16), +invtsc/migratable=off (Geekbench timer flag), lifecycle counters on, stopgap broker (badf2d7+ptr_serial)
# v3 (2026-10-17): the CANDIDATE (integration/prod-candidate-3 + KF3_TEAR_GATE flag, rev ec42327f): lifecycle (scan-off) and lazy observe are production defaults there (no scratch env flags); tear gate OFF for the 30fps fullscreen test.
# Trusted host only. Owner's FULL Windows 11 install under kayfabe (installer ISO + tools disc + virtio drivers), production profile, no tracing.
# Caller: flock -o /tmp/kayfabe-fastguest.lock bash this.sh   Stop: touch /tmp/kf-stop-hitch
set -euo pipefail
W=/var/lib/kf-windows-20261005; REV=${REV:-0452613b}; N=${N:-3081}
B=/data/claude-gap/broker_gap.sh
D=$W/win11-full-b; OVL=/data/claude-hitch/disk/H.qcow2; VARS=/data/claude-hitch/disk/H.vars.fd; TOOLS=$W/win11-full/kfwin11-tools.iso; RUN=$W/boundary-kayfabe-$N; STOPF=/tmp/kf-stop-hitch
export KF_LOCKED=1; . $D/identity.env
export BROKER_BIN=/data/claude-cursor6/nvkvm-display-broker
true
case "$(cat /sys/kernel/iommu_groups/11/type)" in DMA-FQ|identity) ;; *) echo bad-iommu-type; exit 13;; esac
test -f "$OVL" && test -f "$VARS" && test -f "$TOOLS"
test -x "$W/kf3-bins/$REV/qemu-system-x86_64"
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 53687091200
rm -f /tmp/kf-stop-winprod "$STOPF"
mkdir -p -m 0700 "$RUN"
ln -sfn "$OVL" "$RUN/windows.qcow2"; ln -sfn "$VARS" "$RUN/OVMF_VARS.fd"
cleanup() { rc=$?; trap - EXIT; bash "$B" stop "$N" || true; rm -f /tmp/kf-stop-winprod "$STOPF"; echo "HITCH_SESSION_EXIT=$rc at=$(date -Is)"; }
trap cleanup EXIT
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export WIN_CPU="host,-vmx,migratable=off,+invtsc,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_vpindex,hv_runtime,hv_time,hv_synic,hv_stimer,hv_tlbflush,hv_ipi,hv_frequencies"
# no guest internet during install (OOBE then offers offline setup; no Windows Update before the driver is pinned)
export WIN_NET_RESTRICT=off WIN_MAC=$VM_MAC WIN_GPU_UUID=$GPU_UUID
export KF3_REV=$REV WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS="${WIN_FLAGS:-}" PROD_EXTRA_FLAGS= WIN_DROP="KF3_TIMER_MAP ${EXTRA_DROP:-}" WIN_FB_MB=8192 WIN_RAM_MB=12288 WIN_BAR1=8589934592 WIN_REUSE=1
export WIN_QEMU_EXTRA="-uuid $VM_UUID -smbios type=1,manufacturer=KayfabeLab,product=KF-Guest,serial=$VM_SERIAL -drive file=$TOOLS,if=none,id=cd1,media=cdrom,readonly=on -device ide-cd,drive=cd1,bus=ide.0,unit=0 -drive file=/var/lib/vast-windows-test-media/virtio.iso,if=none,id=cd2,media=cdrom,readonly=on -device ide-cd,drive=cd2,bus=ide.0,unit=1 -drive file=$W/appmatrix/image/kfapps.iso,if=none,id=cd3,media=cdrom,readonly=on -device ide-cd,drive=cd3,bus=ide.1,unit=0"
export WIN_TITLE="kayfabe Windows 11 (run $N, $REV)" WIN_MAX_SECONDS=0
bash "$B" run "$N"
QPID=$(sed -n 's/^QPID=//p' "$W/windows-broker-run$N.state")
echo "HITCH_SESSION_READY=$(date -Is) run=$N rev=$REV install=win11x64.iso qpid=$QPID"
while kill -0 "$QPID" 2>/dev/null; do
  if test -e "$STOPF"; then echo "HITCH_SESSION_STOP_REQUEST=$(date -Is)"; break; fi
  sleep 5
done
# v4 (2026-10-17): candidate-4 = prod-candidate-3 + display pacing phase 1+2 (no parked flips, broker tick source, lock step), broker from the same commit
