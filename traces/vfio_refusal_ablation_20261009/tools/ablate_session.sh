#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# ablate_session.sh -- one hardware session of the 2026-10-09 VFIO refusal ablation. Run as
#   flock -o /tmp/kayfabe-fastguest.lock bash ablate_session.sh SPECFILE
# SPECFILE: one boot per line `LABEL GESTURE REFUSE_FILE|- LOCK_SECS IDLE_SECS RECORD_SECS` (# comments).
# It detaches the 4070 to vfio-pci, runs every boot on a FRESH overlay of the same baseline (Windows 11 26100, NVIDIA 580.88,
# lock screen, no autologon), one after the other, and restores the 4070 to nvidia (IOMMU group 11 stays on DMA-FQ).
# Per boot: ablate_ctl.py (QMP screendumps of the std VGA, LogonUI probe over QGA until the first positive answer only, the
# ONE gesture or none) and, after the recording window, gl-monitor.py (the guest's own live logs through QGA), then a clean stop.
set -u
W=/var/lib/kf-windows-20261005
A=$W/vfio-ablation-20261009
T=$A/tools
SPEC=${1:?spec file}
S=$A/runs/session-$(date -u +%Y%m%dT%H%M%S)
mkdir -p $S
L(){ echo "ABLATE $(date -u +%FT%T.%3NZ) $*" | tee -a $S/session.log; }
drv(){ basename "$(readlink -f /sys/bus/pci/devices/0000:01:00.0/driver 2>/dev/null)" 2>/dev/null; }
L "SESSION START spec=$SPEC driver=$(drv) iommu=$(cat /sys/kernel/iommu_groups/11/type) qemu=$(sha256sum $A/bin/qemu-system-x86_64 | cut -c1-16) xid=$(dmesg | grep -c 'NVRM: Xid')"
[ "$(pgrep -c qemu-system)" = 0 ] || { L "REFUSED: a QEMU is running"; exit 4; }
[ "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ ] || { L "REFUSED: group 11 is not DMA-FQ"; exit 3; }
VDR_DIR=$S/bind bash $T/vfio_refusal_ablation.sh detach 2>&1 | tee -a $S/session.log
[ "$(drv)" = vfio-pci ] || { L "detach failed"; VDR_DIR=$S/bind bash $T/vfio_refusal_ablation.sh restore 2>&1 | tee -a $S/session.log; exit 3; }
while read -r -u 3 LABEL GESTURE REF LOCKS IDLES RECS; do
  O=$S/$LABEL; mkdir -p $O
  RUN=$O/boot1
  L "BOOT $LABEL gesture=$GESTURE refuse=$REF lock=$LOCKS idle=$IDLES record=$RECS"
  [ "$REF" != - ] && cp "$REF" $O/refuse.txt
  export VDR_DIR=$O VDR_TRACE_BAR0=1
  if [ "$REF" != - ]; then export VDR_REFUSE=$O/refuse.txt; else unset VDR_REFUSE; fi
  bash $T/vfio_refusal_ablation.sh run 1 fresh 2>&1 | tee -a $O/run.log
  QPID=$(cat $RUN/qemu.pid 2>/dev/null)
  [ -n "$QPID" ] && kill -0 "$QPID" 2>/dev/null || { L "QEMU did not start"; continue; }
  python3 -I $T/ablate_ctl.py vfio $RUN $QPID $O/ctl --gesture $GESTURE --lock-secs $LOCKS --idle-secs $IDLES --record-secs $RECS > $O/ctl.out 2>&1
  CRC=$?
  L "ctl rc=$CRC: $(tr '\n' ' ' < $O/ctl/click.txt | cut -c1-700)"
  if [ "$REF" != - ]; then
    if python3 -I $T/verify_rewrites.py $RUN/gsp.jsonl "$REF" > $O/verify.txt 2>&1; then
      L "VERIFY ok: $(tail -1 $O/verify.txt)"
    else
      L "*** VERIFY FAILED: a rewrite was INEFFECTIVE (real GSP returned status 0). See $O/verify.txt ***"
      tail -20 $O/verify.txt | sed 's/^/  VERIFY /' | tee -a $O/run.log
    fi
  fi
  if kill -0 $QPID 2>/dev/null && [ -f $T/post_idle.ps1 ]; then
    # liveness check AFTER the idle window (nvidia-smi issues RM controls, so never before it)
    timeout 150 python3 -I $T/qga_run_ps.py $RUN $T/post_idle.ps1 $O/post_idle.txt > /dev/null 2>&1
    L "post_idle: nvidia-smi: $(grep -a 'nvidia-smi exit' $O/post_idle.txt | tr -d '\r') ; $(grep -a -m1 ConfigManagerErrorCode $O/post_idle.txt | tr -s ' ')"
  fi
  if kill -0 $QPID 2>/dev/null; then
    mkdir -p $O/guestlogs
    GL_RUNDIR=$RUN timeout 330 python3 -I $T/gl-monitor.py $LABEL $QPID $O/guestlogs 200 > $O/guestlogs/monitor.out 2>&1
    L "gl-monitor: $(tail -1 $O/guestlogs/monitor.log 2>/dev/null)"
  else
    L "QEMU gone before the guest logs: $(tail -2 $RUN/marker.txt 2>/dev/null | tr '\n' ' ')"
  fi
  # stop: QGA shutdown, ACPI, then (kill only if both fail). A wedged guest: SIGKILL by pid after the reference's waits.
  if kill -0 $QPID 2>/dev/null; then
    timeout 420 bash $T/vfio_refusal_ablation.sh stop 1 2>&1 | tee -a $O/run.log
  fi
  for i in $(seq 20); do kill -0 $QPID 2>/dev/null || break; sleep 1; done
  if kill -0 $QPID 2>/dev/null; then L "QEMU $QPID still alive: SIGKILL by pid"; kill -9 $QPID; sleep 3; fi
  L "BOOT $LABEL END qemu_left=$(pgrep -c qemu-system) refuse_lines=$(grep -c 'x-gsp-refuse' $RUN/qemu.log 2>/dev/null) gsp_bytes=$(stat -c %s $RUN/gsp.jsonl 2>/dev/null) trace_bytes=$(stat -c %s $RUN/trace.log 2>/dev/null) xid=$(dmesg | grep -c 'NVRM: Xid')"
done 3< <(grep -v '^\s*#' "$SPEC" | grep -v '^\s*$')
VDR_DIR=$S/bind bash $T/vfio_refusal_ablation.sh restore 2>&1 | tee -a $S/session.log
L "SESSION END driver=$(drv) iommu=$(cat /sys/kernel/iommu_groups/11/type) qemu_left=$(pgrep -c qemu-system) xid=$(dmesg | grep -c 'NVRM: Xid')"
