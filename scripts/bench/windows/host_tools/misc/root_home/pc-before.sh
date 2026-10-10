#!/bin/bash
# Boot the EXISTING (pre-fix) guest and record its state. Read-only.
cd /opt/nvkvm-steamos-latest
{
echo "### repo state"; git log --oneline -1; git status --porcelain | head -3
echo "### TRIM_RE in this checkout"; grep -n "^TRIM_RE=" boot/steamos_boot.sh
echo "### host driver"; cat /proc/driver/nvidia/version | head -1
echo "### bringing the existing guest up (no reinstall -- qcow2 exists)"
docker compose up -d 2>&1 | tail -5
for i in $(seq 1 60); do
  if ./steamos-ssh true >/dev/null 2>&1; then echo "guest up after $((i*15))s"; break; fi
  sleep 15
done
echo "### guest identity"
./steamos-ssh "grep -E '^(VARIANT_ID|BUILD_ID)' /etc/os-release; uname -r" 2>&1 | tail -4
echo "### libcuda in the PRE-FIX guest (expect 0 / absent)"
./steamos-ssh "ldconfig -p | grep -c libcuda; ls -la /usr/lib/libcuda.so* 2>&1 | head -3" 2>&1 | tail -4
echo "### nvidia userspace version"
./steamos-ssh "cat /proc/driver/nvidia/version 2>/dev/null | head -1" 2>&1 | tail -2
echo "### BEFORE-DONE"
} > /root/pc-before.log 2>&1
