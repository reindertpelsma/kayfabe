#!/bin/bash
while true; do echo "$(date +%s) up=$(cut -d. -f1 /proc/uptime) $(/root/va_capacity 2>&1 | tail -1)" >> /root/cap_log.txt; sleep 300; done
