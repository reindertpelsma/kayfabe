#!/bin/bash
# wr-run.sh N REV "EXTRA_FLAGS" — one Windows boot on kf3 for the windows-reset record (2026-10-09).
# = traces/display_reply_diff_20261008/drd-run2.sh with this checkout ($W/wreset), plus: the boot is recorded with
# the BAR0 read-trace mode (WIN_TRACE / WIN_GSP_OBSERVER / KF3_BAR0_READ_TRACE, default-off diagnostics) unless
# WR_TRACE=0; the engine's progress (`disp[… methods= updates=]`) and any `scanout REFUSED` line are logged at
# each step; after the stop the bugcheck header is read offline by the CALLER (recover_bugcheck.py takes the lock).
# The CALLER holds /tmp/kayfabe-fastguest.lock (flock -o) from the IOMMU switch to its restore.
set -u
W=/var/lib/kf-windows-20261005
N=$1; REV=$2; EXTRA=${3:-}
K=$W/wreset
B=$K/scripts/bench/windows/windows_broker.sh
QGA=$K/scripts/bench/windows/qga_run_ps.py
RUN=$W/boundary-kayfabe-$N
L(){ echo "WRRUN $(date -Is) $*" | tee -a $W/wr-run$N.log; }
L "START run=$N rev=$REV extra=[$EXTRA] trace=${WR_TRACE:-1} xid=$(dmesg | grep -c 'NVRM: Xid')"
bash $W/pti-iommu_nogdm.sh identity 2>&1 | tee -a $W/wr-run$N.log
G=$(basename "$(readlink /sys/bus/pci/devices/0000:01:00.0/iommu_group)")
T=$(cat /sys/kernel/iommu_groups/$G/type)
if [ "$T" != identity ]; then L "REFUSED: group $G is $T"; bash $W/pti-iommu_nogdm.sh DMA-FQ; exit 3; fi
export KF3_REV=$REV
export WIN_FLAGS="KF3_PREEMPT_BIND_PROBE KF3_DISPLAY_IMP_ENABLE KF3_DISPLAY_CTRL_PROBE KF3_ZCULL_BIND_PROBE KF3_SW_RUNLIST_HOST_OWNED KF3_WIN_USER_CHANNELS_PASSTHROUGH KF3_TCENSUS KF3_RELAY_PB_PEEK KF3_WIN_TWIN_DEFAPI_OBJECT KF3_RELAY_GET_REFRESH KF3_ASYNC_PREEMPT KF3_USERD_RELAY_OFF $EXTRA"
if [ "${WR_TRACE:-1}" = 1 ]; then
  export WIN_TRACE=1 WIN_GSP_OBSERVER=1 WIN_GSP_OBSERVER_SECONDS=1800
  export KF3_BAR0_READ_TRACE=1 KF3_READ_TRACE_RANGES=0x110000-0x110fff,0xb81000-0xb81fff KF3_READ_TRACE_MAX_BYTES=2G
fi
WIN_REUSE=0 bash $B run $N > $W/wr-launch-$N.log 2>&1 || { L "LAUNCH FAILED"; cat $W/wr-launch-$N.log; bash $W/pti-iommu_nogdm.sh DMA-FQ; exit 2; }
QPID=$(sed -n 's/^QPID=//p' $W/windows-broker-run$N.state)
LOG=$RUN/qemu.log
L "launched qpid=$QPID flags=[$WIN_FLAGS]"
PUTS(){ grep -o 'disp\[writes=[0-9]* puts=[0-9]* methods=[0-9]* updates=[0-9]*' $LOG | tail -1; }
ST(){ echo "[$(PUTS)] vsyncs=$(grep -c 'WTRACE.*VSYNC' $LOG) refused=$(grep -c 'scanout REFUSED' $LOG) chinfo=$(grep -c 'cmd=0xc3700104' $LOG) vga=$(grep -c 'fn=49 ' $LOG) teardown=$(grep -c 'ChannelFreed { kind: Core' $LOG)"; }
stall=0
for i in $(seq 1 480); do
  kill -0 $QPID 2>/dev/null || { L "qemu gone at ${i}x0.5s"; break; }
  if sed -n '/BORN Passthrough/,$p' $LOG 2>/dev/null | grep -q 'cmd=0x00730108'; then stall=1; break; fi
  sleep 0.5
done
L "marker after ${i}x0.5s stall=$stall $(ST)"
L "experiments: $(grep -c 'EXPERIMENT KF3_DISPLAY_LUT_MIRROR=1' $LOG) mirror, $(grep -c 'EXPERIMENT KF3_DISPLAY_ILUT_OFFSET_256=1' $LOG) ilut-unit, $(grep -c 'CAPS_PROBE' $LOG) caps; refused: $(grep -m1 'scanout REFUSED' $LOG | cut -c1-200)"
sleep 40
L "after 40s: $(ST)"
mkdir -p $RUN/shots
timeout 20 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd screendump "{\"filename\":\"$RUN/shots/after40.ppm\",\"device\":\"kf0\"}" >/dev/null 2>&1
L "screendump rc=$? $(ls -la $RUN/shots/after40.ppm 2>/dev/null | awk '{print $5}') bytes"
for s in d3d11_clear_probe.ps1 d3d12_signal_probe.ps1 vdr_monitor_info.ps1 vdr_user_session.ps1; do
  kill -0 $QPID 2>/dev/null || { L "qemu gone before $s"; break; }
  timeout 180 python3 $QGA $N $W/winflip/$s $RUN/flip-$s.out > /dev/null 2>&1
  L "ps $s rc=$? lines=$(wc -l < $RUN/flip-$s.out 2>/dev/null) rc-line: $(grep -a '^\[rc=' $RUN/flip-$s.out 2>/dev/null | tail -1)"
done
timeout 20 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd screendump "{\"filename\":\"$RUN/shots/end.ppm\",\"device\":\"kf0\"}" >/dev/null 2>&1
L "before stop: $(ST)"
bash $B stop $N 2>&1 | tail -2 | tee -a $W/wr-run$N.log
for j in $(seq 1 60); do kill -0 $QPID 2>/dev/null || break; sleep 2; done
if kill -0 $QPID 2>/dev/null; then
  timeout 10 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd quit >/dev/null 2>&1; sleep 5
  L "clean stop failed: QMP quit"
fi
sleep 3
L "qemu left: $(pgrep -c qemu-system)"
bash $W/pti-iommu_nogdm.sh DMA-FQ 2>&1 | tee -a $W/wr-run$N.log
L "END type=$(cat /sys/kernel/iommu_groups/$G/type) xid=$(dmesg | grep -c 'NVRM: Xid') driver=$(basename $(readlink /sys/bus/pci/devices/0000:01:00.0/driver)) $(grep 'BAR0-TRACE report' $LOG | tail -1 | cut -c1-200)"
