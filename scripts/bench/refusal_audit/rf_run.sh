#!/usr/bin/env bash
# rf_run.sh host|guest <run> [workload...|all] — the refusal-audit recording on a bench box
# (docs/design/V3_REFUSAL_AUDIT.md). Strictly serial (the bench lock), like apps_matrix.sh.
#   host : build the recorder + probes on the host, run every workload on bare metal under it
#   guest: fat-guest boots on kf3 (boot_capture.sh + rf_hook.sh), continuing in a fresh boot
#          after a dead guest, until every workload has a guest row
# results: /workspace/rf/<run>/<workload>/{host_r1.jsonl,guest_r1.jsonl,...}, host.res, guest.res
# env: KF3_BIN (default: kf3-bins/<this tree's HEAD>), RF_PER_BOOT (default 6)
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../../.." && pwd)"
SIDE=${1:?host|guest}; RUN=${2:?run}; shift 2
R=/workspace/rf/$RUN; mkdir -p "$R" /workspace/rf/bin
say(){ echo "[rf_run $(date -Is)] $*"; }
ALL=$(bash "$HERE/rf_workloads.sh" x x list)
[ "${1:-all}" = all ] && set -- $ALL
REV=$(git -C "$REPO" rev-parse --short=8 HEAD)
say "START side=$SIDE run=$RUN rev=$REV workloads=$#"
# the class-A probe, built once on the host with the bundle's toolkit (static cudart)
if [ ! -x /workspace/rf/bin/blocksync ] || [ "$HERE/blocksync.cu" -nt /workspace/rf/bin/blocksync ]; then
  /usr/local/cuda-12.6/bin/nvcc -O2 -arch=sm_86 -cudart static -o /workspace/rf/bin/blocksync "$HERE/blocksync.cu" && say "blocksync built"
fi
if [ "$SIDE" = host ]; then
  mkdir -p /opt/rf
  NVD=$REPO/archive/nvkvm/tests/mode2/nvdiff
  cp -f "$NVD/nvdiff_shim.c" "$NVD/uvm_sizes.h" "$HERE/rf_workloads.sh" "$HERE/torch_train.py" /opt/rf/
  cp -f /workspace/rf/bin/blocksync /opt/rf/
  ( cd /opt/rf && gcc -shared -fPIC -O2 -o nvdiff_shim.so nvdiff_shim.c -ldl -lpthread ) || { say "shim build failed"; exit 2; }
  I="-I$REPO/scripts/bench/cuda_min"; LC=-lcuda; [ -e /usr/lib/x86_64-linux-gnu/libcuda.so ] || LC=/usr/lib/x86_64-linux-gnu/libcuda.so.1
  gcc -O0 $I -o /opt/rf/cup2 "$REPO/archive/nvkvm/tests/mode2/cup2.c" $LC && gcc -O0 $I -o /opt/rf/cup3 "$REPO/scripts/bench/cup3.c" $LC \
    && gcc -O0 $I -o /opt/rf/cup8 "$REPO/scripts/bench/cup8.c" $LC -lm || { say "cup build failed"; exit 2; }
  exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9
  bash /opt/rf/rf_workloads.sh host /workspace/rf/host_tmp "$@" | tee -a "$R/host.res"
  flock -u 9
  for w in "$@"; do mkdir -p "$R/$w"; mv -f /workspace/rf/host_tmp/$w/host_r1.jsonl /workspace/rf/host_tmp/$w/host.log "$R/$w/" 2>/dev/null; done
  dmesg | grep -E 'Xid|NVRM' | tail -50 > "$R/host_dmesg_tail.log"
  say "HOST_DONE $(grep -c 'verdict=PASS' "$R/host.res") pass / $(grep -c RFRES "$R/host.res") rows"
  exit 0
fi
QB=${KF3_BIN:-/workspace/bench/kf3-bins/$REV/qemu-system-x86_64}
[ -x "$QB" ] || { say "⊘ no kf3 binary at $QB"; exit 2; }
say "kf3 binary: $QB"
PER=${RF_PER_BOOT:-6}; todo="$*"; n=0
while [ -n "$(echo $todo)" ] && [ $n -lt 40 ]; do
  n=$((n+1)); tag="rf_${RUN}_b$n"; batch=$(echo $todo | cut -d' ' -f1-"$PER")
  exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9
  env KF_DEVICE=kf3 QEMU_BIN="$QB" NVKVM_RAM_MB=${NVKVM_RAM_MB:-16384} KF_SMP=${KF_SMP:-6} GQ_TIMEOUT=300 \
      RF_WORKLOADS="$batch" RF_OUT="$R" POST_CAPTURE_HOOK="$HERE/rf_hook.sh" \
      bash "$REPO/scripts/bench/boot_capture.sh" "$tag" > "$R/boot_$tag.driver.log" 2>&1
  rc=$?; flock -u 9; exec 9>&-
  for x in dmesg dmesg_after probe hostdmesg serial; do cp -f "/workspace/bench/run_${tag}_$x.log" "$R/boot_$tag.$x.log" 2>/dev/null; done
  zstd -q -f "/workspace/bench/run_${tag}_qemu.log" -o "$R/boot_$tag.qemu.log.zst" 2>/dev/null
  done_w=$(grep -a "boot=$tag " "$R/guest.res" 2>/dev/null | sed -n 's/.* w=\([^ ]*\) .*/\1/p')
  [ -z "$done_w" ] && { first=$(echo $todo | cut -d' ' -f1); echo "RFRES side=guest w=$first verdict=BOOT_FAIL boot=$tag" | tee -a "$R/guest.res"; done_w=$first; }
  say "boot $tag rc=$rc did=[$(echo $done_w | tr '\n' ' ')]"
  new=""; for a in $todo; do grep -qx "$a" <<<"$done_w" || new="$new $a"; done; todo=$new
  while pgrep -x qemu-system-x86 >/dev/null; do sleep 3; done
done
say "GUEST_DONE $(grep -c 'verdict=PASS' "$R/guest.res") pass / $(grep -c RFRES "$R/guest.res") rows"
