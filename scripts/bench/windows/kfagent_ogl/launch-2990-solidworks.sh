#!/usr/bin/env bash
# Trusted host only. SOLIDWORKS VARIANT (2026-10-15, owner config): guest RAM 16 GiB, fb-mb 8192, BAR1 4 GiB; fallback FB_MB=4096 BAR1=536870912. Production flag profile (OGL_FLAGS empty). Owner's FULL Windows 11 install under kayfabe (installer ISO + tools disc + virtio drivers), production profile, no tracing.
# Caller: flock -o /tmp/kayfabe-fastguest.lock bash this.sh   Stop: touch /tmp/kf-stop-claude-manual
set -euo pipefail
W=/var/lib/kf-windows-20261005; REV=${REV:-bf510bdf}; N=${N:-2990}
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_claude_oglw.sh
D=$W/win11-full-b; TOOLS=$W/win11-full/kfwin11-tools.iso; RUN=$W/boundary-kayfabe-$N; STOPF=/tmp/kf-stop-claude-manual
export KF_LOCKED=1; . $D/identity.env
true
case "$(cat /sys/kernel/iommu_groups/11/type)" in DMA-FQ|identity) ;; *) echo bad-iommu-type; exit 13;; esac
test -f "$D/windows11.qcow2" && test -f "$D/OVMF_VARS.fd" && test -f "$TOOLS"
test -x "$W/kf3-bins/$REV/qemu-system-x86_64"
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 107374182400
# host RAM for a ${RAM_MB:-16384} MiB memfd guest plus 2 GiB margin; host VRAM for the fb-mb store plus 1.5 GiB margin
need_kb=$(( (${RAM_MB:-16384} + 2048) * 1024 )); avail_kb=$(awk "/MemAvailable/{print \$2}" /proc/meminfo)
[ "$avail_kb" -ge "$need_kb" ] || { echo "host RAM: MemAvailable ${avail_kb} kB < ${need_kb} kB needed"; exit 14; }
vram_free=$(nvidia-smi --query-gpu=memory.free --format=csv,noheader,nounits | head -1)
[ "$vram_free" -ge $(( ${FB_MB:-8192} + 1536 )) ] || { echo "host VRAM: ${vram_free} MiB free < $(( ${FB_MB:-8192} + 1536 )) MiB needed"; exit 15; }
rm -f /tmp/kf-stop-winprod "$STOPF"
mkdir -p -m 0700 "$RUN"
ln -sfn "$D/windows11.qcow2" "$RUN/windows.qcow2"; ln -sfn "$D/OVMF_VARS.fd" "$RUN/OVMF_VARS.fd"
cleanup() { rc=$?; trap - EXIT; bash "$B" stop "$N" || true; rm -f /tmp/kf-stop-winprod "$STOPF"; echo "OWNER_SESSION_EXIT=$rc at=$(date -Is)"; }
trap cleanup EXIT
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export WIN_CPU="host,-vmx,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_vpindex,hv_runtime,hv_time,hv_synic,hv_stimer,hv_tlbflush,hv_ipi,hv_frequencies"
# no guest internet during install (OOBE then offers offline setup; no Windows Update before the driver is pinned)
export WIN_NET_RESTRICT=off WIN_MAC=$VM_MAC WIN_GPU_UUID=$GPU_UUID
export KF3_REV=$REV WIN_TRACE=${OGL_TRACE:-0} WIN_GSP_OBSERVER=0 WIN_FLAGS="${OGL_FLAGS:-}" PROD_EXTRA_FLAGS= WIN_DROP="KF3_TIMER_MAP" WIN_FB_MB=${FB_MB:-8192} WIN_RAM_MB=${RAM_MB:-16384} WIN_BAR1=${BAR1:-4294967296} WIN_REUSE=1
export WIN_QEMU_EXTRA="-uuid $VM_UUID -smbios type=1,manufacturer=KayfabeLab,product=KF-Guest,serial=$VM_SERIAL -drive file=$TOOLS,if=none,id=cd1,media=cdrom,readonly=on -device ide-cd,drive=cd1,bus=ide.0,unit=0 -drive file=/var/lib/vast-windows-test-media/virtio.iso,if=none,id=cd2,media=cdrom,readonly=on -device ide-cd,drive=cd2,bus=ide.0,unit=1 -fw_cfg name=opt/ovmf/X-PciMmio64Mb,string=32768"
export WIN_TITLE="kayfabe Windows 11 (run $N, $REV)" WIN_MAX_SECONDS=0
bash "$B" run "$N"
QPID=$(sed -n 's/^QPID=//p' "$W/windows-broker-run$N.state")
echo "OWNER_SESSION_READY=$(date -Is) run=$N rev=$REV install=win11x64.iso qpid=$QPID"
while kill -0 "$QPID" 2>/dev/null; do
  if test -e "$STOPF"; then echo "OWNER_SESSION_STOP_REQUEST=$(date -Is)"; break; fi
  sleep 5
done
