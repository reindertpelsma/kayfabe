#!/bin/bash
# usage: sampler.sh <qemu.log> <out>   host epoch + qemu.log line count every 0.5 s until the log disappears
while [ -e "$1" ]; do echo "$(date +%s.%N) $(wc -l < "$1")" >> "$2"; sleep 0.5; done
