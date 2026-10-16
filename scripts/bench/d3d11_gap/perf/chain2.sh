#!/bin/bash
# chain.sh LISTFILE : interleaved arms. Each line "TAG|REV|FLAGS|EXTRA(optional: hook=present)". One fresh boot per line, Heaven D3D11 then OpenGL in the same boot.
cd /data/claude-gap; P=/data/claude-gap/perf
export OVL=/data/claude-gap/disk/K.qcow2 VARS=/data/claude-gap/disk/K.vars.fd
echo CHAIN_START $(date -Is) list=$1
one_boot() {   # TAG REV FLAGS HOOK
  local tag=$1 rev=$2 fl=$3 hook=$4
  echo "BOOT_START $tag rev=$rev flags=[$fl] $(date -Is)"
  REV=$rev bash $P/startk.sh $tag "$fl" > ev/start_$tag.out 2>&1
  sleep 40
  if [ "$hook" = present ]; then
    HOOK="bash $P/kfp.sh pi_${tag}_d3d11 interactive $P/jobs/presentinfo.ps1 120" HDELAY=120 bash $P/hv2.sh ${tag,,}_d3d11 direct3d11 > ev/heaven/${tag,,}_d3d11.sum 2>&1
  else
    bash $P/hv2.sh ${tag,,}_d3d11 direct3d11 > ev/heaven/${tag,,}_d3d11.sum 2>&1
  fi
  if grep -q "FPS:" ev/heaven/${tag,,}_d3d11.out 2>/dev/null; then
    if [ "$hook" = present ]; then
      HOOK="bash $P/kfp.sh pi_${tag}_gl interactive $P/jobs/presentinfo.ps1 120" HDELAY=120 bash $P/hv2.sh ${tag,,}_gl opengl > ev/heaven/${tag,,}_gl.sum 2>&1
    else
      bash $P/hv2.sh ${tag,,}_gl opengl > ev/heaven/${tag,,}_gl.sum 2>&1
    fi
  else echo "NO D3D11 RESULT for $tag: skipping GL"; fi
  bash $P/stopk.sh $tag > ev/stop_$tag.out 2>&1
  echo "BOOT_END $tag $(date -Is)"
}
while IFS='|' read -r tag rev fl hook bk; do
  [ -z "$tag" ] && continue
  case "$tag" in \#*) continue;; esac
  BROKER_BACKEND="$bk" one_boot "$tag" "$rev" "$fl" "$hook"
done < "$1"
echo CHAIN_END $(date -Is)
