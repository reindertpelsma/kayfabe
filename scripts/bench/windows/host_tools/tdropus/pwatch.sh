# PREEMPTDUMP=1 (H-S, shape S): watch kayfabe's log for a preempt-all burst the guest never re-enables: >= PD_MIN (6)
# DISABLE_CHANNELS(bDisable=true) since the last bDisable=false, and no bDisable=false for PD_QUIET_MS (500) after the last one.
# Then: one LAPIC/MSI-X sample, info registers -a, and a full guest-memory dump (the VM resumes after it); the hold continues
# 60 s more. The open list's eventData (KEVENT addresses) are saved from the RUNLIST_PREEMPT_COMPLETE lines.
preempt_watch(){
  local lg=$RUN/qemu.log off cur chunk p m burst=0 lastp=0 now
  off=$(stat -c %s $lg); : > $O/pdump-open.txt
  while alive && [ ! -e $O/.pdump_done ]; do
    sleep 0.1
    cur=$(stat -c %s $lg); chunk=$(tail -c +$((off+1)) $lg | head -c $((cur-off))); off=$cur
    p=$(printf '%s' "$chunk" | grep -a -c 'DISABLE_CHANNELS(bDisable=true')
    m=$(printf '%s' "$chunk" | grep -a -c 'DISABLE_CHANNELS(bDisable=false')
    now=$(date +%s%3N)
    if [ "$m" -gt 0 ]; then burst=0; : > $O/pdump-open.txt; fi
    if [ "$p" -gt 0 ]; then burst=$((burst+p)); lastp=$now; printf '%s\n' "$chunk" | grep -a 'RUNLIST_PREEMPT_COMPLETE posted\|DISABLE_CHANNELS(bDisable=true' >> $O/pdump-open.txt; fi
    if [ $burst -ge ${PD_MIN:-6} ] && [ $((now-lastp)) -ge ${PD_QUIET_MS:-500} ]; then
      L "PREEMPT-OPEN detected: $burst disables, no enable for $((now-lastp)) ms; sampling + dumping"
      ( timeout 3 python3 $W/tdrhunt/irq_sampler.py $RUN/qmp.sock $QPID $O/pdump-irq.txt $O/.pdump_irq_stop 0.2 8 & SP=$!; sleep 0.7; touch $O/.pdump_irq_stop; wait $SP ) 2>/dev/null
      { echo "== $(date -u +%FT%T.%3N)"; Q cmd human-monitor-command '{"command-line":"info registers -a"}' 2>&1; } > $O/pdump-regs.txt
      L "PREEMPT-OPEN: dump start"
      timeout 20 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd dump-guest-memory "{\"paging\":false,\"protocol\":\"file:$W/dumps/run$N-preempt.elf\"}" >/dev/null 2>&1
      local st=""
      for k in $(seq 1 120); do
        st=$(timeout 8 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd query-dump 2>&1 | tr -d '\n ')
        case "$st" in *completed*|*failed*) break;; esac; sleep 2
      done
      L "PREEMPT-OPEN: dump $st"
      date +%s > $O/.pdump_done
    fi
  done
}
