#!/bin/bash
# drd-run2.sh N REV "EXTRA_FLAGS" — one Windows boot on kf3 (display reply diff, boot 3 of the 2026-10-09 time-box).
# drd-run.sh (= the flip record's flip-run.sh) with one change: the stall marker (0x00730108 after the first
# Passthrough birth) no longer ends the boot. Run 98 showed it fires during normal display activity (window flips
# and 900+ VSyncs followed it), so this boot logs the marker's puts line, waits 40 s, takes a QMP screendump, runs
# the QGA probes (D3D11 clear + read-back, D3D12 fence, monitor info + nvidia-smi as SYSTEM, the user-session
# nvidia-smi + D3D11 + screenshot) with a timeout each, then stops cleanly (QMP quit if the guest does not stop).
# The CALLER holds /tmp/kayfabe-fastguest.lock (flock -o) from the IOMMU switch to its restore.
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
PUTS(){ grep -o 'disp\[writes=[0-9]* puts=[0-9]* methods=[0-9]* updates=[0-9]*' $LOG | tail -1; }
stall=0
for i in $(seq 1 480); do
  kill -0 $QPID 2>/dev/null || { L "qemu gone at ${i}x0.5s"; break; }
  if sed -n '/BORN Passthrough/,$p' $LOG 2>/dev/null | grep -q 'cmd=0x00730108'; then stall=1; break; fi
  sleep 0.5
done
L "marker after ${i}x0.5s stall=$stall puts_line=[$(PUTS)] (marker no longer ends the boot)"
sleep 40
L "after 40s: [$(PUTS)] vsyncs=$(grep -c 'WTRACE.*VSYNC' $LOG)"
mkdir -p $RUN/shots
timeout 20 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd screendump "{\"filename\":\"$RUN/shots/after40.ppm\",\"device\":\"kf0\"}" >/dev/null 2>&1
L "screendump rc=$? $(ls -la $RUN/shots/after40.ppm 2>/dev/null | awk '{print $5}') bytes"
for s in d3d11_clear_probe.ps1 d3d12_signal_probe.ps1 vdr_monitor_info.ps1 vdr_user_session.ps1; do
  kill -0 $QPID 2>/dev/null || { L "qemu gone before $s"; break; }
  timeout 180 python3 $QGA $N $W/winflip/$s $RUN/flip-$s.out > /dev/null 2>&1
  L "ps $s rc=$? lines=$(wc -l < $RUN/flip-$s.out 2>/dev/null) rc-line: $(grep -a '^\[rc=' $RUN/flip-$s.out 2>/dev/null | tail -1)"
done
timeout 20 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd screendump "{\"filename\":\"$RUN/shots/end.ppm\",\"device\":\"kf0\"}" >/dev/null 2>&1
L "before stop: [$(PUTS)] vsyncs=$(grep -c 'WTRACE.*VSYNC' $LOG)"
bash $B stop $N 2>&1 | tail -2 | tee -a $W/drd-run$N.log
for j in $(seq 1 60); do kill -0 $QPID 2>/dev/null || break; sleep 2; done
if kill -0 $QPID 2>/dev/null; then
  timeout 10 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd quit >/dev/null 2>&1; sleep 5
  L "clean stop failed: QMP quit"
fi
sleep 3
L "qemu left: $(pgrep -c qemu-system)"
bash $W/pti-iommu_nogdm.sh DMA-FQ 2>&1 | tee -a $W/drd-run$N.log
L "END type=$(cat /sys/kernel/iommu_groups/$G/type) xid=$(dmesg | grep -c 'NVRM: Xid') driver=$(basename $(readlink /sys/bus/pci/devices/0000:01:00.0/driver))"
