#!/bin/bash
# gl-run.sh N REV — one Windows boot on kf3 (flood OFF; the flags of run 114, KF3_NO_BATCHED_MAP included) with the guest-log collector
# of this directory running through QGA from the first answer (gl-monitor.py). Derived from wr-run.sh (same launch, same screenshots,
# same BAR0 read trace); the difference: no D3D probes, no marker wait, the collector instead.
# The CALLER holds /tmp/kayfabe-fastguest.lock (flock -o) from the IOMMU switch to its restore (gl-launch.sh does).
set -u
W=/var/lib/kf-windows-20261005
N=$1; REV=$2
K=$W/wreset
B=$K/scripts/bench/windows/windows_broker_quiet.sh
G=$W/guestlogs
RUN=$W/boundary-kayfabe-$N
O=$G/run$N
mkdir -p $O
L(){ echo "GLRUN $(date -u +%FT%T.%3NZ) $*" | tee -a $O/gl-run.log; }
EXTRA="KF3_DISPLAY_CORE_AT_VBLANK KF3_DISPLAY_WRITE_TRACE KF3_DISPLAY_HDCP_STATE KF3_DISPLAY_PRIVATE_PROBE KF3_DISPLAY_HOTPLUG_EDID_SEEN KF3_DISPLAY_BLANK_STATE KF3_DISPLAY_ARMED_DEFAULTS KF3_DISPLAY_LOADV KF3_DISPLAY_CAPS_PROBE KF3_DISPLAY_LUT_MIRROR KF3_DISPLAY_ILUT_OFFSET_256 KF3_DISPLAY_TRACE KF3_NO_BATCHED_MAP KF3_PT_STALL_SNAPSHOT${EXTRA_FLAGS:+ $EXTRA_FLAGS}"
L "START run=$N rev=$REV flood=OFF extra=[$EXTRA] pre: qemu=$(pgrep -c qemu-system) xid=$(dmesg | grep -c 'NVRM: Xid')"
if [ "$(pgrep -c qemu-system)" != 0 ]; then L "REFUSED: another QEMU is running"; exit 4; fi
bash $W/pti-iommu_nogdm.sh identity 2>&1 | tee -a $O/gl-run.log
GRP=$(basename "$(readlink /sys/bus/pci/devices/0000:01:00.0/iommu_group)")
T=$(cat /sys/kernel/iommu_groups/$GRP/type)
if [ "$T" != identity ]; then L "REFUSED: group $GRP is $T"; bash $W/pti-iommu_nogdm.sh DMA-FQ; exit 3; fi
export KF3_REV=$REV
export WIN_FLAGS="KF3_PREEMPT_BIND_PROBE KF3_DISPLAY_IMP_ENABLE KF3_DISPLAY_CTRL_PROBE KF3_ZCULL_BIND_PROBE KF3_SW_RUNLIST_HOST_OWNED KF3_WIN_USER_CHANNELS_PASSTHROUGH KF3_TCENSUS KF3_RELAY_PB_PEEK KF3_WIN_TWIN_DEFAPI_OBJECT KF3_RELAY_GET_REFRESH KF3_ASYNC_PREEMPT KF3_USERD_RELAY_OFF $EXTRA"
export WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_OBS_ONLY=${OBS:-0}; export WIN_DROP="KF3_RPC_TRACE KF3_DISPLAY_METHOD_TRACE KF3_BAR0_TRACE KF3_MAPLOG KF3_TCENSUS KF3_DISPLAY_WRITE_TRACE KF3_DISPLAY_TRACE ${DROP_MORE:-}"
unset KF3_BAR0_READ_TRACE KF3_READ_TRACE_RANGES KF3_READ_TRACE_MAX_BYTES
WIN_REUSE=${WIN_REUSE:-0} bash $B run $N > $W/wr-launch-$N.log 2>&1 || { L "LAUNCH FAILED"; cat $W/wr-launch-$N.log; bash $W/pti-iommu_nogdm.sh DMA-FQ; exit 2; }
QPID=$(sed -n 's/^QPID=//p' $W/windows-broker-run$N.state)
L "launched qpid=$QPID"
SHOTS=$RUN/shots-$(date +%H%M%S); mkdir -p $SHOTS
( for i in $(seq 1 0); do
    kill -0 $QPID 2>/dev/null || break
    ts=$(date -u +%H%M%S.%3N)
    timeout 5 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd screendump "{\"filename\":\"$SHOTS/s-$ts.ppm\",\"device\":\"kf0\"}" >/dev/null 2>&1
    sleep 0.7
  done ) &
SHOTPID=$!
PW=${KF_GUEST_PW:?set KF_GUEST_PW (the throwaway test-guest password)}
Q(){ timeout 15 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock "$@"; }
G(){ timeout 40 python3 $W/boundary-tools/qmp.py $RUN/qga.sock "$@"; }
key(){ Q cmd send-key "{\"keys\":[{\"type\":\"qcode\",\"data\":\"$1\"}]}" >/dev/null 2>&1; sleep 0.25; }
alive(){ kill -0 $QPID 2>/dev/null; }
SIGNIN_DELAY=${SIGNIN_DELAY:-30}
TA=0; for j in $(seq 1 90); do alive || break; G qga-ping >/dev/null 2>&1 && { TA=$(date +%s); break; }; sleep 2; done
L "qga answered: ta=$TA alive=$(alive && echo 1 || echo 0)"
if [ "$TA" != 0 ] && alive; then
  G qga-exec net.exe user vast $PW >/dev/null 2>&1; L "password set rc=$?"
  if [ "${DUMPCFG:-0}" = 1 ] && alive; then
    PS=$(cat $W/dumpcfg/setdump.ps1)
    G qga-exec powershell.exe -NoProfile -Command "$PS" > $O/dumpcfg-before.txt 2>&1; L "dumpcfg set rc=$?"
    G qga-exec shutdown.exe /s /f /t 0 >/dev/null 2>&1; L "guest shutdown requested (phase 1 ends)"
    for j in $(seq 1 90); do alive || break; sleep 2; done
    L "phase 1 qemu alive=$(alive && echo 1 || echo 0)"
    TA=0
  fi
  while alive && [ $(( $(date +%s) - TA )) -lt $SIGNIN_DELAY ]; do sleep 1; done
  if alive; then
    Q cmd screendump "{\"filename\":\"$O/pre-signin.ppm\",\"device\":\"kf0\"}" >/dev/null 2>&1
    L "SIGNIN keys at +$(( $(date +%s) - TA ))s"
    key spc; sleep 3
    for c in k f s i g n 7; do key $c; done; key ret
    L "SIGNIN sent"
    if [ "${EDGE_CLICK:-0}" = 1 ]; then
      sleep ${EDGE_DELAY:-20}
      ev(){ Q cmd input-send-event "{\"events\":$1}" >/dev/null 2>&1; sleep 0.15; }
      ev '[{"type":"abs","data":{"axis":"x","value":802}},{"type":"abs","data":{"axis":"y","value":4854}}]'
      for k in 1 2; do ev '[{"type":"btn","data":{"down":true,"button":"left"}}]'; ev '[{"type":"btn","data":{"down":false,"button":"left"}}]'; done
      L "EDGE double-click sent (abs 802,4854)"
      sleep 2; key ret; L "EDGE Enter sent"
      sleep 25; Q cmd screendump "{\"filename\":\"$O/after-edge-click.ppm\",\"device\":\"kf0\"}" >/dev/null 2>&1
    fi
    if [ "${EDGE:-0}" = 1 ]; then
      sleep ${EDGE_DELAY:-25}
      BASE=$(grep -a -c "PT-SNAP BEGIN stall" $RUN/qemu.log)
      ( while alive; do
          NOW=$(grep -a -c "PT-SNAP BEGIN stall" $RUN/qemu.log)
          if [ "$NOW" -gt "$BASE" ]; then
            L "STALL snapshot #$NOW seen: dumping guest memory"
            timeout 5 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd dump-guest-memory "{\"paging\":false,\"protocol\":\"file:$W/dumps/run$N-stall.elf\"}" >/dev/null 2>&1
            for k in $(seq 1 60); do
              st=$(timeout 8 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd query-dump 2>&1 | tr -d "\n ")
              case "$st" in *completed*|*failed*) break;; esac; sleep 3
            done
            L "STALL dump $st"; break
          fi
          sleep 0.3
        done ) &
      Q cmd send-key '{"keys":[{"type":"qcode","data":"meta_l"},{"type":"qcode","data":"r"}]}' >/dev/null 2>&1; sleep 2
      for c in m s e d g e; do key $c; done; key ret
      L "EDGE launched by keys (stall base=$BASE)"
    fi
  fi
fi
for j in $(seq 1 120); do alive || break; [ -e /tmp/kf-stop-$N ] && break; sleep 2; done
if alive; then Q cmd screendump "{\"filename\":\"$O/post-signin.ppm\",\"device\":\"kf0\"}" >/dev/null 2>&1; fi
L "RESULT alive_at_end=$(alive && echo 1 || echo 0) t_since_qga=$(( $(date +%s) - TA ))s"
echo quiet-wait-done > $O/monitor.log
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
