#!/bin/bash
# chain5.sh LISTFILE : interleaved arms. Line: TAG|REV|FLAGS|HOOK(present|perf|perfd|)|BACKEND(wayland|test|)|RESET(reset|noreset)
# One fresh boot per line, Heaven D3D11 then OpenGL in the same boot. Per arm: memory reset (if reset), mem records before/after, a timestamped
# per-second display-line log (wall clock prefix on the qemu.log 'display fps' lines), per-run env records (hv2.sh).
cd /data/claude-gap; P=/data/claude-gap/perf
export OVL=/data/claude-gap/disk/K.qcow2 VARS=/data/claude-gap/disk/K.vars.fd
RUNLOG=/var/lib/kf-windows-20261005/boundary-kayfabe-2995/qemu.log
echo CHAIN_START $(date -Is) list=$1
one_boot() {   # TAG REV FLAGS HOOK BACKEND RESET
  local tag=$1 rev=$2 fl=$3 hook=$4 bk=$5 rs=$6
  mkdir -p ev/arm
  echo "BOOT_START $tag rev=$rev flags=[$fl] backend=${bk:-default(wayland)} reset=$rs $(date -Is)"
  if [ "$rs" = reset ]; then bash $P/memreset.sh > ev/arm/$tag.memreset.txt 2>&1; tail -2 ev/arm/$tag.memreset.txt; fi
  bash $P/memrec.sh pre_boot_$tag > ev/arm/$tag.mem_pre.txt 2>&1
  BROKER_BACKEND="$bk" REV=$rev bash $P/startk.sh $tag "$fl" > ev/start_$tag.out 2>&1
  grep -h "broker: backend" ev/launch-$tag.out > ev/arm/$tag.broker.txt
  ( : > ev/arm/$tag.dispts.txt; tail -n0 -F $RUNLOG 2>/dev/null | grep --line-buffered -a "display fps" | while IFS= read -r l; do printf '%s %s\n' "$(date +%s.%N)" "$l"; done >> ev/arm/$tag.dispts.txt ) &
  local TSP=$!
  sleep 40
  bash $P/memrec.sh pre_run_$tag > ev/arm/$tag.mem_prerun.txt 2>&1
  hk() {   # API -> sets HOOK/HDELAY for this run
    unset HOOK HDELAY
    case "$hook:$1" in
      present:*) export HOOK="bash $P/kfp.sh pi_${tag}_$1 interactive $P/jobs/presentinfo.ps1 120" HDELAY=120;;
      perf:*|perfd:d3d11) export HOOK="bash $P/perfall.sh ${tag,,}_$1 30" HDELAY=105;;
    esac
  }
  hk d3d11; bash $P/hv2.sh ${tag,,}_d3d11 direct3d11 > ev/heaven/${tag,,}_d3d11.sum 2>&1
  if [ "$hook" = nogl ]; then :
  elif grep -q "FPS:" ev/heaven/${tag,,}_d3d11.out 2>/dev/null; then
    hk gl; bash $P/hv2.sh ${tag,,}_gl opengl > ev/heaven/${tag,,}_gl.sum 2>&1
  else echo "NO D3D11 RESULT for $tag: skipping GL"; fi
  unset HOOK HDELAY
  bash $P/memrec.sh post_run_$tag > ev/arm/$tag.mem_postrun.txt 2>&1
  bash $P/stopk.sh $tag > ev/stop_$tag.out 2>&1
  pkill -P $TSP 2>/dev/null; kill $TSP 2>/dev/null; pkill -f "tail -n0 -F $RUNLOG" 2>/dev/null
  echo "BOOT_END $tag $(date -Is)"
}
while IFS='|' read -r tag rev fl hook bk rs; do
  [ -z "$tag" ] && continue
  case "$tag" in \#*) continue;; esac
  one_boot "$tag" "$rev" "$fl" "$hook" "$bk" "${rs:-noreset}"
done < "$1"
echo CHAIN_END $(date -Is)
