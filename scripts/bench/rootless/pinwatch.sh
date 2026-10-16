#!/bin/bash
out=$1; : > $out
for i in $(seq 1 300); do
  for p in $(pgrep -f "[k]f-fastguest"); do
    if [ "$(readlink /proc/$p/exe)" = "/data/claude-rootless/inst/bin/qemu-system-x86_64" ]; then
      echo "$(date +%T) $(grep -E "VmPin|VmLck|VmRSS" /proc/$p/status | tr -s " \t" " " | tr "\n" " ") uid=$(stat -c %u /proc/$p)" >> $out
    fi
  done; sleep 1
done
