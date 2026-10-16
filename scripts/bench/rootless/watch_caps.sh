#!/bin/bash
# records uid + CapEff + VmLck of every kf-fastguest qemu seen, once per pid (sampled at first sight and 2 s later)
out=$1; seen=" "
while [ ! -e /data/claude-rootless/ev/STOP_WATCH ]; do
  for p in $(pgrep -f "[k]f-fastguest" 2>/dev/null); do
    [ "$(readlink /proc/$p/exe 2>/dev/null)" = "/data/claude-rootless/inst/bin/qemu-system-x86_64" ] || continue
    case "$seen" in *" $p "*) continue;; esac
    seen="$seen$p "; sleep 2
    echo "$(date -Is) pid=$p $(grep -E "^Uid" /proc/$p/status | tr "\t" " ") $(grep CapEff /proc/$p/status | tr "\t" " ") $(grep VmLck /proc/$p/status | tr -s " ") threads=$(ls /proc/$p/task 2>/dev/null | wc -l)" >> $out
  done; sleep 0.3
done
