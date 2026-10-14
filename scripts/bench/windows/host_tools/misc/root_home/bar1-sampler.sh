#!/bin/bash
# BAR1 leak sampler. Appends one row per sample; never truncates.
OUT=/root/bar1-leak.tsv
PHASE=/root/bar1-phase          # write a label here to tag subsequent samples
[ -f "$OUT" ] || printf 'epoch\tphase\tbar1_used_mib\tbar1_total_mib\tvmm\tbroker\tqemu_pids\tx_clients\n' > "$OUT"
while true; do
  used=$(nvidia-smi -q 2>/dev/null | awk '/BAR1 Memory Usage/{f=1} f&&/Used/{gsub(/[^0-9]/,"",$3); print $3; exit}')
  tot=$(nvidia-smi -q 2>/dev/null | awk '/BAR1 Memory Usage/{f=1} f&&/Total/{gsub(/[^0-9]/,"",$3); print $3; exit}')
  ph=$(cat "$PHASE" 2>/dev/null || echo unset)
  vmm=$(docker ps --filter name=nvkvm-steamos-vmm --format '{{.Names}}' 2>/dev/null | wc -l)
  brk=$(docker ps --filter name=nvkvm-steamos-broker --format '{{.Names}}' 2>/dev/null | wc -l)
  qp=$(pgrep -c -f qemu-system-x86_64 2>/dev/null || echo 0)
  xc=$(pgrep -c -f "Xorg|Xwayland" 2>/dev/null || echo 0)
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$(date +%s)" "$ph" "${used:-NA}" "${tot:-NA}" "$vmm" "$brk" "$qp" "$xc" >> "$OUT"
  sleep 20
done
