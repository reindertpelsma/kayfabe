#!/bin/bash
# gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector
# of this directory running through QGA from the first answer (gl-monitor.py). Derived from wr-run.sh (same launch, same screenshots,
# same BAR0 read trace); the difference: no D3D probes, no marker wait, the collector instead.
# The CALLER holds /tmp/kayfabe-fastguest.lock (flock -o) from the IOMMU switch to its restore (gl-launch.sh does).
set -u
W=/var/lib/kf-windows-20261005
N=$1; REV=$2
K=$W/wreset
B=$K/scripts/bench/windows/windows_broker.sh
G=$W/guestlogs
RUN=$W/boundary-kayfabe-$N
O=$G/run$N
mkdir -p $O
L(){ echo "GLRUN $(date -u +%FT%T.%3NZ) $*" | tee -a $O/gl-run.log; }
EXTRA="KF3_DISPLAY_CORE_AT_VBLANK KF3_DISPLAY_WRITE_TRACE KF3_DISPLAY_HDCP_STATE KF3_DISPLAY_PRIVATE_PROBE KF3_DISPLAY_HOTPLUG_EDID_SEEN KF3_DISPLAY_BLANK_STATE KF3_DISPLAY_ARMED_DEFAULTS KF3_DISPLAY_LOADV KF3_DISPLAY_CAPS_PROBE KF3_DISPLAY_LUT_MIRROR KF3_DISPLAY_ILUT_OFFSET_256 KF3_PT_STALL_SNAPSHOT KF3_DISPLAY_TRACE KF3_NO_BATCHED_MAP${EXTRA_FLAGS:+ $EXTRA_FLAGS}"
L "START run=$N rev=$REV flood=OFF extra=[$EXTRA] pre: qemu=$(pgrep -c qemu-system) xid=$(dmesg | grep -c 'NVRM: Xid')"
if [ "$(pgrep -c qemu-system)" != 0 ]; then L "REFUSED: another QEMU is running"; exit 4; fi
bash $W/pti-iommu_nogdm.sh identity 2>&1 | tee -a $O/gl-run.log
GRP=$(basename "$(readlink /sys/bus/pci/devices/0000:01:00.0/iommu_group)")
T=$(cat /sys/kernel/iommu_groups/$GRP/type)
if [ "$T" != identity ]; then L "REFUSED: group $GRP is $T"; bash $W/pti-iommu_nogdm.sh DMA-FQ; exit 3; fi
export KF3_REV=$REV
export WIN_FLAGS="KF3_PREEMPT_BIND_PROBE KF3_DISPLAY_IMP_ENABLE KF3_DISPLAY_CTRL_PROBE KF3_ZCULL_BIND_PROBE KF3_SW_RUNLIST_HOST_OWNED KF3_WIN_USER_CHANNELS_PASSTHROUGH KF3_TCENSUS KF3_RELAY_PB_PEEK KF3_WIN_TWIN_DEFAPI_OBJECT KF3_RELAY_GET_REFRESH KF3_ASYNC_PREEMPT KF3_USERD_RELAY_OFF $EXTRA"
export WIN_TRACE=1 WIN_GSP_OBSERVER=1 WIN_GSP_OBSERVER_SECONDS=1800
export KF3_BAR0_READ_TRACE=1 KF3_READ_TRACE_RANGES=0x110000-0x110fff,0xb81000-0xb81fff KF3_READ_TRACE_MAX_BYTES=2G
WIN_REUSE=0 bash $B run $N > $W/wr-launch-$N.log 2>&1 || { L "LAUNCH FAILED"; cat $W/wr-launch-$N.log; bash $W/pti-iommu_nogdm.sh DMA-FQ; exit 2; }
QPID=$(sed -n 's/^QPID=//p' $W/windows-broker-run$N.state)
L "launched qpid=$QPID"
SHOTS=$RUN/shots-$(date +%H%M%S); mkdir -p $SHOTS
( for i in $(seq 1 90); do
    kill -0 $QPID 2>/dev/null || break
    ts=$(date -u +%H%M%S.%3N)
    timeout 5 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd screendump "{\"filename\":\"$SHOTS/s-$ts.ppm\",\"device\":\"kf0\"}" >/dev/null 2>&1
    sleep 0.7
  done ) &
SHOTPID=$!
python3 -I $G/gl-monitor.py $N $QPID $O 600 > $O/monitor.out 2>&1
L "monitor done: $(tail -1 $O/monitor.log)"
wait $SHOTPID 2>/dev/null
bash $B stop $N 2>&1 | tail -2 | tee -a $O/gl-run.log
for j in $(seq 1 60); do kill -0 $QPID 2>/dev/null || break; sleep 2; done
if kill -0 $QPID 2>/dev/null; then
  timeout 10 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd quit >/dev/null 2>&1; sleep 5
  L "clean stop failed: QMP quit"
fi
sleep 3
L "qemu left: $(pgrep -c qemu-system)"
bash $W/pti-iommu_nogdm.sh DMA-FQ 2>&1 | tee -a $O/gl-run.log
L "END type=$(cat /sys/kernel/iommu_groups/$GRP/type) driver=$(basename $(readlink /sys/bus/pci/devices/0000:01:00.0/driver)) xid_lines=$(dmesg | grep -c 'NVRM: Xid')"
