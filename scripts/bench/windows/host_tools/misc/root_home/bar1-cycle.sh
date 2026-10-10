#!/bin/bash
# Distinguish a genuine leak from non-shrinking retained capacity:
# cycle the VM N times and see whether BAR1 grows per cycle or plateaus.
cd /opt/nvkvm-steamos-latest || exit 1
mark(){ echo "$1" > /root/bar1-phase; sleep "${2:-45}"; }
b1(){ nvidia-smi -q 2>/dev/null | awk '/BAR1 Memory Usage/{f=1} f&&/Used/{gsub(/[^0-9]/,"",$3);print $3;exit}'; }
log(){ echo "$(date +%H:%M:%S) $*" >> /root/bar1-cycle.log; }
log "=== cycle experiment start, BAR1=$(b1) ==="
for i in 1 2 3 4; do
  log "--- cycle $i: stopping stack (BAR1 before stop=$(b1)) ---"
  mark "C${i}_stopping" 5
  docker compose down >/dev/null 2>&1
  mark "C${i}_stack_down" 90
  log "cycle $i: stack DOWN 90s, BAR1=$(b1)"
  log "--- cycle $i: starting stack ---"
  mark "C${i}_starting" 5
  docker compose up -d >/dev/null 2>&1
  mark "C${i}_stack_up" 150
  log "cycle $i: stack UP 150s, BAR1=$(b1)"
done
echo "DONE" > /root/bar1-phase
log "=== cycle experiment done, BAR1=$(b1) ==="
