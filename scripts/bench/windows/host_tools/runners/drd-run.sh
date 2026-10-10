#!/bin/bash
# drd-run.sh N REV "EXTRA_FLAGS" — one Windows boot on kf3 (display reply diff batch, 2026-10-09; copy of flip-run.sh).
# The CALLER holds /tmp/kayfabe-fastguest.lock (flock -o holder for the whole session); this script never takes it.
# IOMMU identity for the run (run90/92's adopted-USERD config), DMA-FQ restored at the end, gdm untouched.
# Fresh overlay (WIN_REUSE=0). Waits up to 240 s for the stall marker (0x00730108 after the first Passthrough
# birth); on a stall: 30 s more, then QMP quit (a bugchecked guest answers neither ACPI nor QGA). No stall: QGA
# probes (D3D11 clear + read-back, D3D12 fence, nvidia-smi in the user session, screenshot), then a clean stop.
set -u
W=/var/lib/kf-windows-20261005
N=$1; REV=$2; EXTRA=${3:-}
K=$W/drd
B=$K/scripts/bench/windows/windows_broker.sh
QGA=$K/scripts/bench/windows/qga_run_ps.py
RUN=$W/boundary-kayfabe-$N
L(){ echo "DRDRUN $(date -Is) $*" | tee -a $W/drd-run$N.log; }
L "START run=$N rev=$REV extra=[$EXTRA] xid=$(dmesg | grep -c 'NVRM: Xid')"
bash $W/pti-iommu_nogdm.sh identity 2>&1 | tee -a $W/drd-run$N.log
G=$(basename "$(readlink /sys/bus/pci/devices/0000:01:00.0/iommu_group)")
T=$(cat /sys/kernel/iommu_groups/$G/type)
if [ "$T" != identity ]; then L "REFUSED: group $G is $T"; exit 3; fi
export KF3_REV=$REV
export WIN_FLAGS="KF3_PREEMPT_BIND_PROBE KF3_DISPLAY_IMP_ENABLE KF3_DISPLAY_CTRL_PROBE KF3_ZCULL_BIND_PROBE KF3_SW_RUNLIST_HOST_OWNED KF3_WIN_USER_CHANNELS_PASSTHROUGH KF3_TCENSUS KF3_RELAY_PB_PEEK KF3_WIN_TWIN_DEFAPI_OBJECT KF3_RELAY_GET_REFRESH KF3_ASYNC_PREEMPT KF3_USERD_RELAY_OFF $EXTRA"
WIN_REUSE=0 bash $B run $N > $W/drd-launch-$N.log 2>&1 || { L "LAUNCH FAILED"; cat $W/drd-launch-$N.log; bash $W/pti-iommu_nogdm.sh DMA-FQ; exit 2; }
QPID=$(sed -n 's/^QPID=//p' $W/windows-broker-run$N.state)
LOG=$RUN/qemu.log
L "launched qpid=$QPID flags=[$WIN_FLAGS]"
stall=0
for i in $(seq 1 480); do
  kill -0 $QPID 2>/dev/null || { L "qemu gone at ${i}x0.5s"; break; }
  if sed -n '/BORN Passthrough/,$p' $LOG 2>/dev/null | grep -q 'cmd=0x00730108'; then stall=1; break; fi
  sleep 0.5
done
L "wait done after ${i}x0.5s stall=$stall puts_line=[$(grep -o 'disp\[writes=[0-9]* puts=[0-9]* methods=[0-9]* updates=[0-9]*' $LOG | tail -1)]"
if [ $stall = 1 ]; then
  sleep 30
  L "after 30s: frees=$(grep -c 'route=passthrough' $LOG) [$(grep -o 'disp\[writes=[0-9]* puts=[0-9]* methods=[0-9]* updates=[0-9]*' $LOG | tail -1)]"
  timeout 10 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd quit >/dev/null 2>&1
  for j in $(seq 1 30); do kill -0 $QPID 2>/dev/null || break; sleep 1; done
  echo "WINDOWS_EXIT stop=qmp-quit-after-bugcheck $(date -Is)" >> $RUN/marker.txt
  L "killed (QMP quit) alive=$(kill -0 $QPID 2>/dev/null && echo yes || echo no)"
else
  for s in d3d11_clear_probe.ps1 d3d12_signal_probe.ps1 vdr_monitor_info.ps1; do
    timeout 300 python3 $QGA $N $W/winflip/$s $RUN/flip-$s.out > /dev/null 2>&1
    L "ps $s rc-line: $(grep -a '^\[rc=' $RUN/flip-$s.out 2>/dev/null | tail -1)"
  done
  L "no stall: [$(grep -o 'disp\[writes=[0-9]* puts=[0-9]* methods=[0-9]* updates=[0-9]*' $LOG | tail -1)] marker=$(grep -c 'cmd=0x00730108' $LOG)"
  bash $B stop $N 2>&1 | tail -2 | tee -a $W/drd-run$N.log
  for j in $(seq 1 60); do kill -0 $QPID 2>/dev/null || break; sleep 2; done
  if kill -0 $QPID 2>/dev/null; then
    timeout 10 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd quit >/dev/null 2>&1; sleep 5
    L "clean stop failed: QMP quit"
  fi
fi
sleep 3
L "qemu left: $(pgrep -c qemu-system)"
bash $W/pti-iommu_nogdm.sh DMA-FQ 2>&1 | tee -a $W/drd-run$N.log
L "END type=$(cat /sys/kernel/iommu_groups/$G/type) xid=$(dmesg | grep -c 'NVRM: Xid') driver=$(basename $(readlink /sys/bus/pci/devices/0000:01:00.0/driver))"
