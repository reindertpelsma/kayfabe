#!/bin/bash
while true; do echo "$(date +%s) up=$(cut -d. -f1 /proc/uptime) failed_unmap=$(journalctl -k -b 0 --no-pager | grep -c "Failed to auto-unmap") va_err=$(journalctl -k -b 0 --no-pager | grep -c "alloc VA space") bar1_used=$(nvidia-smi -q -d MEMORY 2>/dev/null | grep -A2 "BAR1 Memory Usage" | grep Used | grep -oE "[0-9]+")" >> /root/leak_log.txt; sleep 60; done
