#!/bin/bash
# chainc.sh LISTFILE : the candidate Windows Heaven matrix. Adapted from scripts/bench/d3d11_gap/perf/chain7.sh (run id 3071, own overlay, own paths).
# Line: TAG|REV|FLAGS|HOOK(shot|shot-nogl|)|BACKEND(wayland)|RESET(noreset)
# One fresh boot per line, Heaven D3D11 then OpenGL in the same boot (HOOK shot: screen statistics at +130 s of the D3D11 run).
# Per boot: memory records, KF3_* environment of the QEMU from /proc/<pid>/environ, journal Xid lines mapped to the QEMU pid, the lazy / lifecycle lines of the QEMU log.
# Caller holds the fastguest lock: CHAIN_LOCKED=1 flock -o /tmp/kayfabe-fastguest.lock bash chainc.sh LIST
G=/data/claude-candhw/gap; cd $G; P=$G/perf
export OVL=$G/disk/K.qcow2 VARS=$G/disk/K.vars.fd RAM_MB=${RAM_MB:-12288} FB_MB=${FB_MB:-8192} BAR1=${BAR1:-8589934592}
W=/var/lib/kf-windows-20261005
RUNLOG=$W/boundary-kayfabe-3071/qemu.log
echo CHAIN_START $(date -Is) list=$1
one_boot() {   # TAG REV FLAGS HOOK BACKEND RESET
  local tag=$1 rev=$2 fl=$3 hook=$4 bk=$5
  mkdir -p ev/arm
  # memory gate
  for i in $(seq 90); do f=$(awk '/^full/{split($2,a,"=");print a[2]}' /proc/pressure/memory); m=$(awk '/MemAvailable/{print int($2/1048576)}' /proc/meminfo); [ "${f%.*}" -lt 8 ] && [ "$m" -ge 14 ] && break; sleep 10; done
  echo "BOOT_START $tag rev=$rev flags=[$fl] backend=${bk:-default(wayland)} mem_avail_gb=$m psi_full10=$f $(date -Is)"
  local JS; JS=$(date '+%Y-%m-%d %H:%M:%S')
  bash $P/memrec.sh pre_boot_$tag > ev/arm/$tag.mem_pre.txt 2>&1
  BROKER_BACKEND="$bk" REV=$rev bash $P/startk.sh $tag "$fl" > ev/start_$tag.out 2>&1
  grep -h "broker: backend" ev/launch-$tag.out > ev/arm/$tag.broker.txt
  local QP; QP=$(sed -n "s/^QPID=//p" $W/windows-broker-run3071.state)
  echo "QPID=$QP exe=$(readlink /proc/$QP/exe) sha256=$(sha256sum /proc/$QP/exe | cut -c1-64)" > ev/arm/$tag.environ.txt
  tr '\0' '\n' < /proc/$QP/environ | grep -E '^(KF3_|WIN_RAM|WIN_FB|WIN_BAR1)' | sort >> ev/arm/$tag.environ.txt
  ( : > ev/arm/$tag.dispts.txt; tail -n0 -F $RUNLOG 2>/dev/null | grep --line-buffered -a "display fps" | while IFS= read -r l; do printf '%s %s\n' "$(date +%s.%N)" "$l"; done >> ev/arm/$tag.dispts.txt ) &
  local TSP=$!
  sleep 40
  bash $P/memrec.sh pre_run_$tag > ev/arm/$tag.mem_prerun.txt 2>&1
  hk() {   # API -> sets HOOK/HDELAY for this run
    unset HOOK HDELAY
    case "$hook:$1" in
      shot*:d3d11) export HOOK="bash $P/kfp.sh shot_${tag}_d3d11 interactive $P/jobs/shot.ps1.in 150 > ev/heaven/shot_${tag}_d3d11.out 2>&1" HDELAY=130;;
    esac
  }
  hk d3d11; bash $P/hv2.sh ${tag,,}_d3d11 direct3d11 > ev/heaven/${tag,,}_d3d11.sum 2>&1
  if [[ "$hook" == *nogl* ]]; then :
  elif grep -q "FPS:" ev/heaven/${tag,,}_d3d11.out 2>/dev/null; then
    hk gl; bash $P/hv2.sh ${tag,,}_gl opengl > ev/heaven/${tag,,}_gl.sum 2>&1
  else echo "NO D3D11 RESULT for $tag: skipping GL"; fi
  unset HOOK HDELAY
  bash $P/memrec.sh post_run_$tag > ev/arm/$tag.mem_postrun.txt 2>&1
  bash $P/stopk.sh $tag > ev/stop_$tag.out 2>&1
  pkill -P $TSP 2>/dev/null; kill $TSP 2>/dev/null; pkill -f "tail -n0 -F $RUNLOG" 2>/dev/null
  # journal Xid lines of this boot, each with the pid it names; mapped to the QEMU pid of this boot
  journalctl -k -o short-iso --since "$JS" 2>/dev/null | grep -a 'NVRM: Xid' > ev/arm/$tag.xid_journal.txt
  echo "XID_JOURNAL tag=$tag qpid=$QP lines=$(wc -l < ev/arm/$tag.xid_journal.txt) for_this_qemu=$(grep -c "pid=$QP[ ,)]" ev/arm/$tag.xid_journal.txt) $(date -Is)" | tee ev/arm/$tag.xid_summary.txt
  zcat ev/qemulog/$tag.qemu.log.gz 2>/dev/null | grep -a -E 'USER_RING_LAZY|USER_RING_LIFECYCLE|USER_RING_REFUSED|USER_RING_POINTER|USER_RING_PUT_BOUNDARY|USER_RING_WINDOW_FRESH|USER_RING_MIGRATED|USER_RING_SOFTWARE_OBJECT_GONE|pointer_pb_read_violation|USER_RING_LOOP' | cut -c1-4000 > ev/arm/$tag.lines.txt
  echo "BOOT_END $tag $(date -Is)"
}
while IFS='|' read -r tag rev fl hook bk rs pin; do
  [ -z "$tag" ] && continue
  case "$tag" in \#*) continue;; esac
  one_boot "$tag" "$rev" "$fl" "$hook" "$bk" "${rs:-noreset}"
done < "$1"
echo CHAIN_END $(date -Is)
