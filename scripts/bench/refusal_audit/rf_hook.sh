#!/usr/bin/env bash
# rf_hook.sh <tag> — POST_CAPTURE_HOOK for boot_capture.sh (kf3 fat guest): run the refusal-audit
# workloads ($RF_WORKLOADS) inside the guest under the nvdiff recorder, one at a time, and for
# EACH persist, under $RF_OUT/<workload>/:
#   guest_r1.jsonl        the guest's ioctl capture (every /dev/nvidia* ioctl, params before+after)
#   guest.log             the workload's own output
#   guest_dmesg.log       the guest dmesg lines this workload added (NVRM / Xid)
#   kf3.log               the kf3 device's stderr lines this workload added — incl. every
#                         `kf3: GSP REFUSED` row first posted during it, and the heartbeats'
#                         gsp_refusals[...] counters (the kernel-side refusals userspace never sees)
# and append `RFRES ... boot=<tag>` to $RF_OUT/guest.res. Stops the boot when the guest dies.
set -uo pipefail
TAG=${1:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../../.." && pwd)"; G="$REPO/scripts/bench/gssh_nv"
OUT=${RF_OUT:?RF_OUT}; mkdir -p "$OUT"
QLOG=${BENCH_DIR:-/workspace/bench}/run_${TAG}_qemu.log
NVD=$REPO/archive/nvkvm/tests/mode2/nvdiff
$G true >/dev/null 2>&1 || { echo "RF_HOOK guest unreachable at start"; exit 0; }
$G 'sudo mkdir -p /opt/rf /var/tmp/rf && sudo chmod 777 /opt/rf /var/tmp/rf'
for f in "$NVD/nvdiff_shim.c" "$NVD/uvm_sizes.h" "$HERE/rf_workloads.sh" "$HERE/torch_train.py" \
         "$REPO/archive/nvkvm/tests/mode2/cup2.c" "$REPO/scripts/bench/cup3.c" "$REPO/scripts/bench/cup8.c" \
         "$REPO/scripts/bench/cuda_min/cuda.h"; do
  $G "cat > /opt/rf/$(basename "$f")" < "$f" || { echo "RF_HOOK push failed: $f"; exit 2; }
done
# the class-A probe is built ON THE HOST (nvcc) and runs unchanged on both sides, like the app bundle
[ -x "${RF_HOST_BIN:-/workspace/rf/bin}/blocksync" ] && $G 'cat > /opt/rf/blocksync; chmod +x /opt/rf/blocksync' < "${RF_HOST_BIN:-/workspace/rf/bin}/blocksync"
$G 'cd /opt/rf && gcc -shared -fPIC -O2 -o nvdiff_shim.so nvdiff_shim.c -ldl -lpthread && echo SHIM_OK;
    for c in cup2 cup3 cup8; do gcc -O0 -I/opt/rf -o $c $c.c -lcuda -lm && echo BUILD_$c=ok; done' 2>&1 | tr '\n' ' '; echo
echo "RF_HOOK tag=$TAG workloads=[$RF_WORKLOADS] guest=$($G 'nvidia-smi --query-gpu=name,driver_version --format=csv,noheader' 2>&1 | head -1)"
# ⊘ boot_capture.sh loads only `nvidia`; EGL / Vulkan need nvidia_modeset + nvidia_drm (as on the host)
echo "RF_GUEST_DRM $($G 'sudo modprobe nvidia_drm; echo rc=$?; lsmod | grep -c ^nvidia' 2>&1 | tr '\n' ' ')"
for w in $RF_WORKLOADS; do
  q0=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
  d0=$($G 'sudo dmesg | wc -l' 2>/dev/null | tr -d '\r'); d0=${d0:-0}
  res=$(timeout "${RF_APP_TMO:-1200}" "$G" "sudo bash /opt/rf/rf_workloads.sh guest /var/tmp/rf/out $w" 2>&1 | tr -d '\r')
  alive=1; $G true >/dev/null 2>&1 || { sleep 20; $G true >/dev/null 2>&1 || alive=0; }
  line=$(grep -a '^RFRES ' <<<"$res" | tail -1)
  [ -n "$line" ] || line="RFRES side=guest w=$w verdict=$([ $alive = 0 ] && echo GUEST_DEAD || echo HANG) note=no-result-line"
  mkdir -p "$OUT/$w"
  if [ $alive = 1 ]; then
    $G "sudo cat /var/tmp/rf/out/$w/guest_r1.jsonl" > "$OUT/$w/guest_r1.jsonl" 2>/dev/null
    $G "sudo cat /var/tmp/rf/out/$w/guest.log" > "$OUT/$w/guest.log" 2>/dev/null
    $G "sudo dmesg | tail -n +$((d0+1))" > "$OUT/$w/guest_dmesg.log" 2>&1
    $G "sudo rm -f /var/tmp/rf/out/$w/guest_r1.jsonl"
  fi
  tail -n +"$((q0+1))" "$QLOG" 2>/dev/null > "$OUT/$w/kf3.log"
  nref=$(grep -c 'kf3: GSP REFUSED' "$OUT/$w/kf3.log"); nx=$(grep -c 'Xid' "$OUT/$w/guest_dmesg.log" 2>/dev/null)
  echo "$line boot=$TAG pulled=$(wc -l < "$OUT/$w/guest_r1.jsonl" 2>/dev/null || echo 0) new_refusal_rows=$nref guest_xid=${nx:-0}" | tee -a "$OUT/guest.res"
  [ $alive = 0 ] && { echo "RF_HOOK guest dead after $w — stopping this boot"; break; }
done
echo "RF_HOOK_DONE"
