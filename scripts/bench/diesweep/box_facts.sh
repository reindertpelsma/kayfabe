#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# box_facts.sh — the rented box's GPU facts that matter to the die sweep, WITHOUT identifiers
# (no UUID, serial, VBIOS version, MAC, hostname): PCI ids, BAR region sizes, Resizable-BAR
# capability, link, what the host driver exposes as BAR1, kernel, driver, kvm. 2026-10-14.
set -uo pipefail
echo "FACTS_START $(date -Is)"
echo "KERNEL $(uname -r)"
echo "DRIVER $(head -1 /proc/driver/nvidia/version 2>/dev/null | sed 's/(.*//')"
echo "KVM $(ls -l /dev/kvm 2>&1 | awk '{print $1, $NF}') nested=$(cat /sys/module/kvm_*/parameters/nested 2>/dev/null | head -1)"
echo "CPU $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2- | sed 's/^ //') nproc=$(nproc) mem_gib=$(free -g | awk '/Mem:/{print $2}')"
echo "DISK $(df -h / | tail -1 | awk '{print "size="$2, "avail="$4}')"
for d in $(lspci -D -d 10de: | awk '{print $1}'); do
    echo "PCI $d $(lspci -nn -s "$d" | cut -d' ' -f2-)"
    lspci -vv -s "$d" 2>/dev/null | grep -E 'Region [0-9]|Resizable|BAR [0-9]: current size|LnkCap:|LnkSta:|Kernel driver' | sed "s/^/PCIV $d /"
done
nvidia-smi --query-gpu=name,pci.device_id,driver_version,memory.total,pcie.link.gen.max,pcie.link.width.max --format=csv 2>&1 | sed 's/^/SMIQ /'
nvidia-smi -q -d MEMORY 2>&1 | sed -n '/FB Memory Usage/,$p' | sed 's/^/SMIMEM /'
echo "FACTS_EXIT $(date -Is)"
