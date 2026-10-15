#!/bin/bash
# summarize.sh : one line per trial dir
cd /data/claude-mctdr/ev/trials
printf "%-8s %-10s %-9s %-5s %-9s %-9s %-7s %-6s %-5s %s\n" trial rev mode PM result t_first153 refused xid javaw note
for d in $(ls -d */ | tr -d / | sort -V); do
  [ -f $d/trial.log ] || continue
  rev=$(sed -n 's/.*rev=\([0-9a-f]*\)\/.*/\1/p' $d/trial.log | head -1)
  mode=$(sed -n 's/^TRIAL .* mode=\([a-z]*\) .*/\1/p' $d/trial.log | head -1)
  pm=no; [ -s $d/pm-start.txt ] && pm=yes; [ -s $d/act-start.txt ] && pm=act
  prof=$(sed -n 's/.*env_prof=\([0-9]\).*/\1/p' $d/trial.log | head -1)
  res=$(sed -n 's/^RESULT=\([A-Z_]*\) at=\(.*\)/\1 \2/p' $d/result.txt | head -1)
  st=$(grep -a -m1 'nvlddmkm' $d/state-final.txt | sed -n 's/^EVT \([0-9:]*\) id=153.*/\1/p')
  ms=$(sed -n 's/.*at \([0-9:]*\)\..*/\1/p' $d/guest-misc.txt | head -1)
  t=-; if [ -n "$st" ] && [ -n "$ms" ]; then t=$(( $(date -u -d "1970-01-01 $st" +%s) - $(date -u -d "1970-01-01 $ms" +%s) )); fi
  ref=$(zcat $d/qemu-slice.log.gz 2>/dev/null | grep -a -c 'ring REFUSED')
  xid=$(grep -c -i xid $d/host-kernel.txt 2>/dev/null)
  ja=$(grep -a -c 'Faulting application name: javaw' $d/state-final.txt)
  printf "%-8s %-10s %-9s %-5s %-9s %-9s %-7s %-6s %-5s prof=%s\n" $d "$rev" "$mode" $pm "${res%% *}" "$t" "$ref" "$xid" "$ja" "$prof"
done
