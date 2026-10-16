#!/bin/bash
# usage: sampler.sh <name-pattern> <outfile> ; samples first matching process once it exists (+3s)
pat="$1"; out="$2"
for i in $(seq 1 600); do
  pid=$(pgrep -f -- "$pat" | while read p; do [ "$(readlink /proc/$p/exe)" = "$(readlink -f /data/claude-rootless/inst/bin/qemu-system-x86_64)" ] && echo $p; done | head -1)
  [ -n "$pid" ] && break; sleep 0.2
done
[ -z "$pid" ] && { echo "no process" > "$out"; exit 1; }
sleep 3
{ echo "pid=$pid date=$(date -Is)"; ps -o pid,user,uid,gid,etime,cmd -p $pid | cut -c1-200
  grep -E "^(Name|Uid|Gid|Groups|Cap|NoNewPrivs|Seccomp|VmLck|VmPin|VmRSS|Threads)" /proc/$pid/status
  echo "--- limits"; grep -E "locked|open files|processes" /proc/$pid/limits
  echo "--- fds"; ls -l /proc/$pid/fd 2>/dev/null | awk '{print $9,$10,$11}' | sed 's/^[0-9]* //' | sort -k2 | uniq -c -f1 | head -60
  echo "--- all threads Cap"; for t in /proc/$pid/task/*; do printf '%s %s %s\n' "$(basename $t)" "$(cat $t/comm)" "$(grep CapEff $t/status)"; done | awk '{c[$3" "$4]++} END{for(k in c) print c[k], k}'
  echo "--- children"; pgrep -P $pid -a | cut -c1-200
  echo "--- cgroup"; cat /proc/$pid/cgroup
  echo "--- ns"; ls -l /proc/$pid/ns | awk '{print $9,$10,$11}' | tr '\n' ' '; echo
} > "$out" 2>&1
