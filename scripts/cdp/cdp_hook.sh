#!/usr/bin/env bash
# cdp_hook.sh TAG — POST_CAPTURE_HOOK of cdp_guest.sh (guest up, driver loaded): copy the probe
# and the nvdiff shim into the guest and run one process per "$CDP_RUNS" entry. Per run it keeps
# the output, the kf3 lines the run added, the guest dmesg lines it added, and (spec `…:t`) the
# ioctl trace — copied out even if the probe hangs (the shim appends as each ioctl happens).
set -uo pipefail
TAG=${1:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
G="$REPO/scripts/bench/gssh_nv"
QLOG=${BENCH_DIR:-/workspace/bench}/run_${TAG}_qemu.log
OUT=${CDP_OUT:?}; BIN=${CDP_BINDIR:?}
$G true >/dev/null 2>&1 || { echo "CDPH guest unreachable"; exit 0; }
$G 'cat > /tmp/cdp_probe; chmod +x /tmp/cdp_probe' < "$BIN/cdp_probe"
$G 'cat > /tmp/nvdiff_shim.so' < "$BIN/nvdiff_shim.so"
echo "CDPH guest=$($G 'nvidia-smi --query-gpu=name,driver_version,persistence_mode --format=csv,noheader' 2>&1 | head -1)"
for spec in ${CDP_RUNS:-4:auto}; do
  q0=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
  d0=$($G 'sudo dmesg | wc -l' 2>/dev/null | tr -d '\r'); d0=${d0:-0}
  if [ "${spec%%:*}" = qs ]; then
    # `qs` = the app matrix's invocation; `qs:N` = -num_items=N (deeper recursion, more launches)
    n=${spec#qs}; n=${n#:}; name=qs${n:+_$n}; arg=${n:+-num_items=$n}
    echo "=== CDPH cdpSimpleQuicksort ${arg:-(default)} $(date -Is)"
    timeout 90 "$G" "sudo timeout -k 5 60 /opt/apps/bundle/samples/cdpSimpleQuicksort $arg 2>&1; echo rc=\$?" 2>&1 | sed 's/^/QS /' | tee "$OUT/$name.log"
  else
    IFS=: read -r m s t <<<"$spec"
    name=probe_${m}_${s}${t:+_t}
    pre=""; [ "${t:-0}" = 1 ] && pre="env NVDIFF_OUT=/tmp/$name.jsonl LD_PRELOAD=/tmp/nvdiff_shim.so"
    echo "=== CDPH run mode=$m sched=$s trace=${t:-0} $(date -Is)"
    timeout 70 "$G" "sudo rm -f /tmp/$name.jsonl; sudo timeout -k 5 45 $pre /tmp/cdp_probe $m $s 2>&1; echo rc=\$?" 2>&1 | tee "$OUT/$name.log"
    [ "${t:-0}" = 1 ] && $G "sudo cat /tmp/$name.jsonl" > "$OUT/$name.jsonl" 2>/dev/null
  fi
  tail -n +"$((q0+1))" "$QLOG" 2>/dev/null > "$OUT/$name.kf3.log"
  $G "sudo dmesg | tail -n +$((d0+1))" > "$OUT/$name.guest_dmesg.log" 2>&1
  echo "  kf3 lines=$(wc -l < "$OUT/$name.kf3.log") guest dmesg lines=$(wc -l < "$OUT/$name.guest_dmesg.log") xid=$(grep -c Xid "$OUT/$name.guest_dmesg.log")"
done
echo "CDPH_DONE"
