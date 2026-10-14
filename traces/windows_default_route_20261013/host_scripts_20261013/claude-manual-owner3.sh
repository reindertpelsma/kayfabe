#!/usr/bin/env bash
# Trusted host only. Owner's personal Windows instance (data in owner-state/manual1380), production profile, no automation, no tracing.
# Caller: flock -o /tmp/kayfabe-fastguest.lock bash this.sh   (holds the GPU lock for the whole session). Stop: touch /tmp/kf-stop-claude-manual
set -euo pipefail
W=/var/lib/kf-windows-20261005; REV=${REV:-ec45aad9}; N=${N:-2910}
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_claude_net.sh
RUN=$W/boundary-kayfabe-$N; OS=$W/owner-state/manual1380-fresh; STOPF=/tmp/kf-stop-claude-manual
FB=${FB:-4096}; RAM=${RAM:-8192}; EXTRA_DROP=${EXTRA_DROP:-}
export KF_LOCKED=1
! pgrep qemu-system || { echo "a QEMU is running"; exit 12; }
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test -f "$OS/resume.qcow2" && test -f "$OS/OVMF_VARS.fd"
test -x "$W/kf3-bins/$REV/qemu-system-x86_64"
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 53687091200
rm -f /tmp/kf-stop-winprod "$STOPF"
mkdir -p -m 0700 "$RUN"
ln -sfn "$OS/resume.qcow2" "$RUN/windows.qcow2"; ln -sfn "$OS/OVMF_VARS.fd" "$RUN/OVMF_VARS.fd"
cleanup() { rc=$?; trap - EXIT; bash "$B" stop "$N" || true; bash "$W/pti-iommu_nogdm.sh" DMA-FQ || true; rm -f /tmp/kf-stop-winprod "$STOPF"; echo "OWNER_SESSION_EXIT=$rc at=$(date -Is)"; }
trap cleanup EXIT
bash "$W/pti-iommu_nogdm.sh" identity
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export WIN_CPU="host,-vmx,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_vpindex,hv_runtime,hv_time,hv_synic,hv_stimer,hv_tlbflush,hv_ipi,hv_frequencies"
export WIN_NET_RESTRICT=on
export KF3_REV=$REV WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS= PROD_EXTRA_FLAGS= WIN_DROP="KF3_TIMER_MAP $EXTRA_DROP" WIN_FB_MB=$FB WIN_RAM_MB=$RAM WIN_REUSE=1
export WIN_TITLE="kayfabe Windows OWNER SESSION (run $N, $REV)" WIN_MAX_SECONDS=0
bash "$B" run "$N"
python3 /data/claude-manual-prepare.py "boundary-kayfabe-$N" || true
QPID=$(sed -n 's/^QPID=//p' "$W/windows-broker-run$N.state")
echo "OWNER_SESSION_READY=$(date -Is) run=$N rev=$REV fb=$FB ram=$RAM drop='$EXTRA_DROP' qpid=$QPID"
while kill -0 "$QPID" 2>/dev/null; do
  if test -e "$STOPF"; then echo "OWNER_SESSION_STOP_REQUEST=$(date -Is)"; break; fi
  sleep 5
done
