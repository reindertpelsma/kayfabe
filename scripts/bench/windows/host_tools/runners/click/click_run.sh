#!/bin/bash
# click_run.sh MODE N REV [GESTURE] — one scripted-input boot (README section 18, 2026-10-09). MODE = kf3 | vfio.
#   kf3 : the launch of gl-run.sh (flood off, the flags of run 114 incl. KF3_NO_BATCHED_MAP, BAR0 read trace, GSP observer,
#         timestamped trace.log) WITHOUT the QGA polling monitor. QMP screendumps only (click_ctl.py), then ONE gesture,
#         90 s of recording, and only THEN the QGA live-log collection (gl-monitor.py) once, then a clean stop.
#   vfio: the reference launch of the same baseline image (vfio_dvi_reference.sh `run N fresh`, patched copy: + qemu-xhci +
#         usb-tablet, + input_event_* trace events, vfio_region_read/write traced). The caller has detached the 4070
#         (`click_run.sh vfio-detach` / `vfio-restore`). LogonUI probe over QGA until its first positive answer only.
# GESTURE = drag (default; the owner's gesture: press, hold, drag up, release) | click (press + release at the centre).
# The CALLER holds /tmp/kayfabe-fastguest.lock (flock -o) for the whole session (click_launch.sh does).
# Run directories: kf3 $W/boundary-kayfabe-N ; vfio $W/click/vfioN/boot1 . Evidence copy: $W/click/evidence/runN-MODE .
set -u
W=/var/lib/kf-windows-20261005
C=$W/click
MODE=${1:?mode}; N=${2:-}; REV=${3:-66eeebb6}; GESTURE=${4:-drag}
K=$W/wreset
EV=$C/trace-events-click.txt
L(){ echo "CLICKRUN $(date -u +%FT%T.%3NZ) $*" | tee -a ${O:-/dev/null}/click-run.log; }

stage() {   # patched copies of the existing harness (differences recorded in README section 18)
  mkdir -p $C/scripts/bench/windows $C/evidence
  { cat $K/scripts/bench/trace-events-vfio-reference.txt; printf '%s\n' input_event_btn input_event_abs input_event_rel input_event_sync input_event_key_qcode; } > $EV
  sed "s#\$HERE/../trace-events-vfio-reference.txt#$EV#" $K/scripts/bench/windows/windows_broker.sh > $C/scripts/bench/windows/windows_broker.sh
  cp $K/scripts/bench/windows/nvidia_device.ps1 $C/scripts/bench/windows/ 2>/dev/null
  sed -e '/-global i440FX-pcihost.pci-hole64-size=32G/a\      -device qemu-xhci,id=xhci,addr=0x7 -device usb-tablet,bus=xhci.0' \
      -e '/# VDR_TRACE_BAR0=1: also every trapped/i\    printf "%s\\n" input_event_btn input_event_abs input_event_rel input_event_sync input_event_key_qcode >> "$R/trace-events"' \
      $W/vfio-dvi-20261008/tools/vfio_dvi_reference.sh > $C/vfio_dvi_reference_click.sh
  chmod +x $C/vfio_dvi_reference_click.sh
}
stage

case "$MODE" in
stage-only)   exit 0 ;;
vfio-detach)  O=$C/evidence; mkdir -p $O; VDR_DIR=$C/vfio-session bash $C/vfio_dvi_reference_click.sh detach; exit $? ;;
vfio-restore) O=$C/evidence; mkdir -p $O; VDR_DIR=$C/vfio-session bash $C/vfio_dvi_reference_click.sh restore; exit $? ;;
esac

O=$C/evidence/run$N-$MODE; mkdir -p $O
cp $C/click_ctl.py $C/gl-monitor.py $O/ 2>/dev/null
drv(){ basename "$(readlink -f /sys/bus/pci/devices/0000:01:00.0/driver 2>/dev/null)" 2>/dev/null; }
if [ "$(pgrep -c qemu-system)" != 0 ]; then L "REFUSED: another QEMU is running"; exit 4; fi

if [ "$MODE" = kf3 ]; then
  RUN=$W/boundary-kayfabe-$N
  B=$C/scripts/bench/windows/windows_broker.sh
  EXTRA="KF3_DISPLAY_CORE_AT_VBLANK KF3_DISPLAY_WRITE_TRACE KF3_DISPLAY_HDCP_STATE KF3_DISPLAY_PRIVATE_PROBE KF3_DISPLAY_HOTPLUG_EDID_SEEN KF3_DISPLAY_BLANK_STATE KF3_DISPLAY_ARMED_DEFAULTS KF3_DISPLAY_LOADV KF3_DISPLAY_CAPS_PROBE KF3_DISPLAY_LUT_MIRROR KF3_DISPLAY_ILUT_OFFSET_256 KF3_PT_STALL_SNAPSHOT KF3_DISPLAY_TRACE KF3_NO_BATCHED_MAP"
  L "START mode=kf3 run=$N rev=$REV gesture=$GESTURE flood=OFF extra=[$EXTRA] driver=$(drv) xid=$(dmesg | grep -c 'NVRM: Xid')"
  [ "$(drv)" = nvidia ] || { L "REFUSED: 01:00.0 is not on nvidia"; exit 3; }
  bash $W/pti-iommu_nogdm.sh identity 2>&1 | tee -a $O/click-run.log
  GRP=$(basename "$(readlink /sys/bus/pci/devices/0000:01:00.0/iommu_group)")
  T=$(cat /sys/kernel/iommu_groups/$GRP/type)
  if [ "$T" != identity ]; then L "REFUSED: group $GRP is $T"; bash $W/pti-iommu_nogdm.sh DMA-FQ; exit 3; fi
  export KF3_REV=$REV
  export WIN_FLAGS="KF3_PREEMPT_BIND_PROBE KF3_DISPLAY_IMP_ENABLE KF3_DISPLAY_CTRL_PROBE KF3_ZCULL_BIND_PROBE KF3_SW_RUNLIST_HOST_OWNED KF3_WIN_USER_CHANNELS_PASSTHROUGH KF3_TCENSUS KF3_RELAY_PB_PEEK KF3_WIN_TWIN_DEFAPI_OBJECT KF3_RELAY_GET_REFRESH KF3_ASYNC_PREEMPT KF3_USERD_RELAY_OFF $EXTRA"
  export WIN_TRACE=1 WIN_GSP_OBSERVER=1 WIN_GSP_OBSERVER_SECONDS=1800
  export KF3_BAR0_READ_TRACE=1 KF3_READ_TRACE_RANGES=0x110000-0x110fff,0xb81000-0xb81fff KF3_READ_TRACE_MAX_BYTES=2G
  WIN_REUSE=0 bash $B run $N > $W/click-launch-$N.log 2>&1 || { L "LAUNCH FAILED"; cat $W/click-launch-$N.log; bash $W/pti-iommu_nogdm.sh DMA-FQ; exit 2; }
  QPID=$(sed -n 's/^QPID=//p' $W/windows-broker-run$N.state)
  L "launched qpid=$QPID rundir=$RUN"
  python3 -I $C/click_ctl.py kf3 $RUN $QPID $O --gesture $GESTURE > $O/ctl.out 2>&1
  CRC=$?
  L "click_ctl rc=$CRC: $(tr '\n' ' ' < $O/click.txt | cut -c1-600)"
  # the QGA live-log collection, once, AFTER the recording (not a confounder any more)
  mkdir -p $O/guestlogs
  GL_RUNDIR=$RUN timeout 330 python3 -I $C/gl-monitor.py $N $QPID $O/guestlogs 200 > $O/guestlogs/monitor.out 2>&1
  L "gl-monitor done: $(tail -1 $O/guestlogs/monitor.log 2>/dev/null)"
  bash $B stop $N 2>&1 | tail -2 | tee -a $O/click-run.log
  for j in $(seq 1 60); do kill -0 $QPID 2>/dev/null || break; sleep 2; done
  if kill -0 $QPID 2>/dev/null; then
    timeout 10 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd quit >/dev/null 2>&1; sleep 5
    L "clean stop failed: QMP quit"
  fi
  sleep 3
  L "qemu left: $(pgrep -c qemu-system)"
  bash $W/pti-iommu_nogdm.sh DMA-FQ 2>&1 | tee -a $O/click-run.log
  L "END type=$(cat /sys/kernel/iommu_groups/$GRP/type) driver=$(drv) xid_lines=$(dmesg | grep -c 'NVRM: Xid')"
else
  D=$C/vfio$N; export VDR_DIR=$D VDR_TRACE_BAR0=1
  RUN=$D/boot1
  L "START mode=vfio run=$N gesture=$GESTURE driver=$(drv) iommu=$(cat /sys/kernel/iommu_groups/11/type) xid=$(dmesg | grep -c 'NVRM: Xid')"
  [ "$(drv)" = vfio-pci ] || { L "REFUSED: 01:00.0 is not on vfio-pci (click_run.sh vfio-detach first)"; exit 3; }
  bash $C/vfio_dvi_reference_click.sh run 1 fresh 2>&1 | tee -a $O/click-run.log
  QPID=$(cat $RUN/qemu.pid)
  python3 -I $C/click_ctl.py vfio $RUN $QPID $O --gesture $GESTURE > $O/ctl.out 2>&1
  CRC=$?
  L "click_ctl rc=$CRC: $(tr '\n' ' ' < $O/click.txt | cut -c1-600)"
  if kill -0 $QPID 2>/dev/null; then
    mkdir -p $O/guestlogs
    GL_RUNDIR=$RUN timeout 330 python3 -I $C/gl-monitor.py $N $QPID $O/guestlogs 200 > $O/guestlogs/monitor.out 2>&1
    L "gl-monitor done: $(tail -1 $O/guestlogs/monitor.log 2>/dev/null)"
    bash $C/vfio_dvi_reference_click.sh stop 1 2>&1 | tee -a $O/click-run.log
  else
    L "vfio QEMU already gone (guest shut down or crashed): $(tail -3 $RUN/marker.txt 2>/dev/null | tr '\n' ' ')"
    bash $C/vfio_dvi_reference_click.sh stop 1 2>&1 | tee -a $O/click-run.log
  fi
  sleep 3
  L "qemu left: $(pgrep -c qemu-system); driver=$(drv)"
fi
