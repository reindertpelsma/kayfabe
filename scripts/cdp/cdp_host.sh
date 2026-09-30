#!/usr/bin/env bash
# cdp_host.sh TAG "RUNS" — the BARE-METAL control on the bench box: the CDP probe (and, with
# spec `qs`, cdpSimpleQuicksort) run directly on the host GPU, serialized on the bench lock.
#   RUNS: space-separated `mode:sched[:t]` (t = record every /dev/nvidia* ioctl with the nvdiff
#         shim) or `qs`. Results: /workspace/apps/results/cdpp/<TAG>/ (host_*.log, *.jsonl).
# Writes CDPH_START … CDPH_EXIT rc= lines; a file without the EXIT line is a killed job.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
TAG=${1:?tag}; RUNS=${2:-0:auto 4:auto}
OUT=/workspace/apps/results/cdpp/$TAG; mkdir -p "$OUT"
echo "CDPH_START $(date -Is) tag=$TAG runs=[$RUNS] scripts=$(git -C "$REPO" rev-parse --short=8 HEAD) host_driver=$(cat /sys/module/nvidia/version 2>/dev/null)"
bash "$HERE/cdp_build.sh" "$OUT/bin" || { echo "CDPH_EXIT rc=3 $(date -Is)"; exit 3; }
T0=$(date +%s)
exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9
for spec in $RUNS; do
  if [ "$spec" = qs ]; then
    echo "=== host cdpSimpleQuicksort $(date -Is)"
    timeout -k 5 60 /workspace/apps/bundle/samples/cdpSimpleQuicksort 2>&1 | tee "$OUT/host_qs.log"; echo "rc=${PIPESTATUS[0]}" | tee -a "$OUT/host_qs.log"
    continue
  fi
  IFS=: read -r m s t <<<"$spec"
  name=host_${m}_${s}${t:+_t}
  echo "=== host mode=$m sched=$s trace=${t:-0} $(date -Is)"
  if [ "${t:-0}" = 1 ]; then
    rm -f "$OUT/$name.jsonl"
    NVDIFF_OUT="$OUT/$name.jsonl" LD_PRELOAD="$OUT/bin/nvdiff_shim.so" timeout -k 5 45 "$OUT/bin/cdp_probe" "$m" "$s" > "$OUT/$name.log" 2>&1
  else
    timeout -k 5 45 "$OUT/bin/cdp_probe" "$m" "$s" > "$OUT/$name.log" 2>&1
  fi
  echo "rc=$?" >> "$OUT/$name.log"; cat "$OUT/$name.log"
done
flock -u 9; exec 9>&-
journalctl -k --since=@"$T0" --no-pager -o short-iso > "$OUT/host_kernel.log" 2>&1
echo "host kernel log since start: $(wc -l < "$OUT/host_kernel.log") lines, $(grep -c 'Xid' "$OUT/host_kernel.log") Xid"
echo "CDPH_EXIT rc=0 $(date -Is)"
