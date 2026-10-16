#!/bin/bash
# memrec.sh LABEL : host memory state, one block of text (coordinator items 2026-10-16): free, swap, memory PSI (some/full avg10/60/300 + totals),
# vmstat counters (pgmajfault, workingset_refault_file, pswpin, pswpout), MemAvailable.
echo "MEMREC $1 $(date -Is) epoch=$(date +%s.%N)"
free -m | sed 's/^/MEMREC free: /'
swapon --show --noheadings | sed 's/^/MEMREC swap: /'
grep -E "^(MemFree|MemAvailable|SwapTotal|SwapFree|Shmem|Cached|Dirty):" /proc/meminfo | tr '\n' ' ' | sed 's/^/MEMREC meminfo: /'; echo
sed 's/^/MEMREC psi.memory: /' /proc/pressure/memory
grep -E "^(pgmajfault|workingset_refault_file|workingset_refault_anon|pswpin|pswpout) " /proc/vmstat | tr '\n' ' ' | sed 's/^/MEMREC vmstat: /'; echo
