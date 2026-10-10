# PREEMPTDUMP=1 (H-S, shape S): watch kayfabe's log, line by line in order, for a disable list (DISABLE_CHANNELS(bDisable=true)) of
# >= PD_MIN entries that no bDisable=false follows for PD_QUIET_MS; then dump guest memory at once (the VM resumes after it).
preempt_watch(){
  local lg=$RUN/qemu.log off cur ln burst=0 lastp=0 now
  off=$(stat -c %s $lg); : > $O/pdump-open.txt
  while alive && [ ! -e $O/.pdump_done ]; do
    sleep 0.1
    cur=$(stat -c %s $lg); now=$(date +%s%3N)
    while IFS= read -r ln; do
      case "$ln" in
        *'bDisable=true'*) burst=$((burst+1)); lastp=$now; printf '%s\n' "$ln" >> $O/pdump-open.txt;;
        *'bDisable=false'*) burst=0; : > $O/pdump-open.txt;;
        *'RUNLIST_PREEMPT_COMPLETE posted'*) printf '%s\n' "$ln" >> $O/pdump-open.txt;;
      esac
    done < <(tail -c +$((off+1)) $lg | head -c $((cur-off)) | grep -a 'DISABLE_CHANNELS(bDisable\|RUNLIST_PREEMPT_COMPLETE posted')
    off=$cur
    if [ $burst -ge ${PD_MIN:-6} ] && [ $lastp -gt 0 ] && [ $((now-lastp)) -ge ${PD_QUIET_MS:-1500} ]; then
      L "PREEMPT-OPEN detected: $burst disables open, no enable for $((now-lastp)) ms; dumping"
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
