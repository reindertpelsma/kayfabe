#!/usr/bin/env bash
# POST_CAPTURE_HOOK (runs on the box HOST with the kf3 guest up): the vkcts row of run_apps.sh (a copy that takes a trusted --caselist), once per case list of $CASES
# ("name=listfile ..."), each in its own deqp process, in ONE boot, in the order given. Around each case: the QEMU (kf3) log lines, the host dmesg lines and the guest dmesg
# lines it added are saved whole. Same environment as the matrix (sudo bash run_apps.sh guest vkcts).
set -uo pipefail
TAG=${1:?tag}; R=/workspace/kf-ci-src; HERE=$R/scripts/apps; G=$R/scripts/bench/gssh_nv
OUT=${APPS_OUT:?APPS_OUT}; mkdir -p "$OUT"; QLOG=/workspace/bench/run_${TAG}_qemu.log
$G true >/dev/null 2>&1 || { echo "OGR_HOOK guest unreachable at start"; exit 0; }
$G 'sudo tee /opt/apps/bundle/run_apps.sh >/dev/null' < /workspace/ogr/run_apps_ogr.sh
tar -C "$HERE/vkcts" -cf - vkcts.py vkcts_lib.py pins.json caselist.tsv | $G 'sudo mkdir -p /opt/apps/bundle/vkcts_tool && sudo tar -C /opt/apps/bundle/vkcts_tool -xf -'
tar -C "$(readlink -f /workspace/apps/vkcts/current)" -cf - . | $G 'sudo mkdir -p /opt/apps/bundle/vkcts && sudo tar -C /opt/apps/bundle/vkcts -xf -'
$G 'sudo rm -rf /opt/apps/out/guest; sudo modprobe nvidia_drm; nvidia-smi --query-gpu=name,driver_version --format=csv,noheader' 2>&1 | head -2
echo "OGR_HOOK tag=$TAG kf3=$(basename "$(dirname "${QEMU_BIN:-x}")") cases=[$CASES]"
for spec in $CASES; do
  name=${spec%%=*}; list=${spec#*=}
  $G 'sudo tee /opt/apps/bundle/vkcts_tool/caselist.tsv >/dev/null' < "$list"
  q0=$(wc -l < "$QLOG"); h0=$(dmesg | wc -l); d0=$($G 'sudo dmesg | wc -l' | tr -d '\r')
  echo "OGR_CASE_START $name $(date -u +%T) q0=$q0 h0=$h0"
  res=$(timeout 1700 "$G" "sudo bash /opt/apps/bundle/run_apps.sh guest vkcts" 2>&1 | tr -d '\r'); rc=$?
  echo "$res" | grep -a '^APPRES ' | tail -1 | sed "s/^/OGR_APPRES $name /"
  $G 'sudo cat /opt/apps/out/guest/vkcts.log' > "$OUT/$name.guest.log" 2>&1
  sleep 3
  dmesg | tail -n +$((h0+1)) > "$OUT/$name.hostdmesg.txt"
  $G "sudo dmesg | tail -n +$((d0+1))" > "$OUT/$name.guest_dmesg.txt" 2>&1
  tail -n +$((q0+1)) "$QLOG" > "$OUT/$name.kf3.full.log"
  echo "OGR_CASE_END $name rc=$rc $(date -u +%T) host_xid_lines=$(grep -c Xid "$OUT/$name.hostdmesg.txt") kf3_lines=$(wc -l < "$OUT/$name.kf3.full.log")"
done
echo "OGR_HOOK_DONE"
