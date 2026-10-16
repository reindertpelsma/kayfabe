#!/bin/bash
# memreset.sh : coordinator item 2: sync; drop reclaimable page cache; only if MemAvailable exceeds swap used by 2 GB, swapoff -a && swapon -a. Never kills anything. Guest RAM size is not touched.
echo "MEMRESET_START $(date -Is)"
sync; echo 1 > /proc/sys/vm/drop_caches
AV=$(awk '/^MemAvailable/{print $2}' /proc/meminfo); SU=$(awk '/^SwapTotal/{t=$2}/^SwapFree/{f=$2}END{print t-f}' /proc/meminfo)
echo "MEMRESET avail_kB=$AV swap_used_kB=$SU"
if [ "$SU" -gt 0 ] && [ "$AV" -gt $((SU + 2097152)) ]; then
  t0=$(date +%s); swapoff -a && swapon -a; echo "MEMRESET swapoff/swapon rc=$? took=$(( $(date +%s) - t0 )) s"
else echo "MEMRESET swap not cycled (swap used $SU kB, avail $AV kB)"; fi
echo "MEMRESET_END $(date -Is)"
