#!/bin/bash
# sample the guest's scanout every 60s; a frozen fb id == the black-after-idle bug
OUT=/root/gs-test/idle-watch.log
echo "=== started $(date -Is)  COMPOSITE_FORCE=1 ===" >> $OUT
while true; do
  ids=$(cd /root/gs-test/repo && timeout 25 ./steamos-ssh 'for i in 1 2 3 4; do sudo grep -m1 "fb=" /sys/kernel/debug/dri/0/state 2>/dev/null | tr -d "\t "; sleep 0.6; done | tr "\n" " "' 2>/dev/null)
  errs=$(cd /root/gs-test/repo && timeout 25 ./steamos-ssh 'journalctl -b _SYSTEMD_USER_UNIT=gamescope-session.service --no-pager 2>/dev/null | grep -c "Cannot import FB"' 2>/dev/null | tr -d "\r\n ")
  uniq_ids=$(echo "$ids" | tr ' ' '\n' | sort -u | grep -c fb=)
  verdict=$([ "${uniq_ids:-0}" -ge 2 ] && echo FLIPPING || echo FROZEN)
  echo "$(date +%H:%M:%S) $verdict ids=[$ids] distinct=$uniq_ids importerrs=$errs" >> $OUT
  sleep 60
done
