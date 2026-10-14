#!/bin/bash
# winpass cycle: one Windows boot on kf3 (windows_broker.sh run N), guest work through QGA, clean stop.
# usage: cycle.sh N REUSE(0|1) STEPS...   steps: wait_stall etwstop wait_reboot decode hags arm hwschoff sleep:S stop
# Run under: flock /tmp/kayfabe-fastguest.lock (one GPU user at a time). Writes $RUN/winpass.log.
set -u
W=/var/lib/kf-windows-20261005
K=$W/kayfabe-winpass
B=$K/scripts/bench/windows/windows_broker.sh
QGA=$K/scripts/bench/windows/qga_run_ps.py
N=$1; REUSE=$2; shift 2
RUN=$W/boundary-kayfabe-$N
export WIN_FLAGS="${WIN_FLAGS:-KF3_PREEMPT_BIND_PROBE KF3_DISPLAY_IMP_ENABLE KF3_DISPLAY_CTRL_PROBE KF3_ZCULL_BIND_PROBE KF3_SW_RUNLIST_HOST_OWNED KF3_WIN_USER_CHANNELS_PASSTHROUGH KF3_TCENSUS KF3_RELAY_PB_PEEK KF3_WIN_TWIN_DEFAPI_OBJECT KF3_RELAY_GET_REFRESH KF3_ASYNC_PREEMPT KF3_USERD_RELAY_OFF}"
L(){ echo "WINPASS $(date -Is) $*" | tee -a "$RUN/winpass.log"; }
G=$(basename "$(readlink /sys/bus/pci/devices/0000:01:00.0/iommu_group)")
T=$(cat /sys/kernel/iommu_groups/$G/type)
if [ "$T" != identity ]; then echo "WINPASS REFUSED: GPU IOMMU group $G is $T, not identity"; exit 3; fi
WIN_REUSE=$REUSE bash $B run $N > /tmp/winpass-launch-$N.log 2>&1 || { cat /tmp/winpass-launch-$N.log; exit 2; }
L "START run=$N reuse=$REUSE rev=$KF3_REV iommu=$T steps=$*"
QPID=$(sed -n 's/^QPID=//p' $W/windows-broker-run$N.state)
LOG=$RUN/qemu.log
qga_up(){ for i in $(seq 1 ${1:-60}); do timeout 10 python3 $W/boundary-tools/qmp.py $RUN/qga.sock qga-ping >/dev/null 2>&1 && return 0; kill -0 $QPID 2>/dev/null || return 1; sleep 5; done; return 1; }
ps(){ python3 $QGA $N $W/winpass/$1 $RUN/winpass-$1.out > /dev/null; L "ps $1 rc-line: $(grep -a '^\[rc=' $RUN/winpass-$1.out | tail -1)"; }
for s in "$@"; do
  case $s in
  wait_stall)
    # the guest's diagnostic snapshot at a TDR's start (runs 78/84/88: 0x00730108 then 0x007302a5)
    # — the first one AFTER the first Passthrough birth (earlier ones are boot-time display queries)
    for i in $(seq 1 600); do sed -n '/BORN Passthrough/,$p' $LOG 2>/dev/null | grep -q 'cmd=0x00730108' && break; kill -0 $QPID 2>/dev/null || break; sleep 0.5; done
    L "stall marker after ${i}x0.5s: $(sed -n '/BORN Passthrough/,$p' $LOG | grep -c 'cmd=0x00730108')" ;;
  etwstop) ps etwstopnow.ps1 ;;
  wait_reboot)
    # a TDR that bugchecks resets the VM in this QEMU (-action reboot=reset): wait for the teardown, then QGA again
    for i in $(seq 1 600); do grep -q 'route=passthrough' $LOG && break; sleep 0.5; done
    L "teardown (passthrough frees) seen=$(grep -c 'route=passthrough' $LOG)"; sleep 30; qga_up 60; L "qga after reboot rc=$?" ;;
  qga) qga_up 60; L "qga rc=$?" ;;
  decode) ps dxg_etw_stop.ps1 ;;
  hags) ps hags.ps1 ;;
  arm) ps dxg_etw_arm.ps1 ;;
  hwschoff) ps hwsch_off.ps1 ;;
  sleep:*) sleep ${s#sleep:} ;;
  stop) bash $B stop $N 2>&1 | tail -2 | tee -a $RUN/winpass.log ;;
  kill)
    # a guest stopped at a bugcheck answers neither ACPI nor QGA: QMP quit (a run overlay, never the desktop)
    timeout 10 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd quit >/dev/null 2>&1
    for i in $(seq 1 30); do kill -0 $QPID 2>/dev/null || break; sleep 1; done
    echo "WINDOWS_EXIT stop=qmp-quit-after-bugcheck $(date -Is)" >> $RUN/marker.txt; L "killed (QMP quit)" ;;
  wait_frees)
    for i in $(seq 1 600); do grep -q 'route=passthrough' $LOG && break; sleep 0.5; done
    L "teardown (passthrough frees) seen=$(grep -c 'route=passthrough' $LOG)" ;;
  decodeold) ps dxg_decode_kept.ps1 ;;
  esac
done
L "END xid_dmesg=$(dmesg | grep -c 'NVRM: Xid')"
