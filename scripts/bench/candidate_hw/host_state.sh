#!/bin/bash
# host_state.sh <label> <outdir> : Xid list (full lines), driver, GPU, qemu/lock/lsmod state.
L=$1; O=$2; mkdir -p "$O"
dmesg 2>/dev/null | grep -a 'NVRM: Xid' > "$O/xid_$L.txt"
{ echo "STATE $L $(date -Is)"
  echo "xid_count=$(wc -l < "$O/xid_$L.txt")"
  echo "driver=$(cat /sys/module/nvidia/version) $(head -1 /proc/driver/nvidia/version | cut -c1-120)"
  nvidia-smi --query-gpu=name,driver_version,memory.used,pstate --format=csv,noheader
  echo "qemu_procs=$(pgrep -cx qemu-system-x86)"
  lsmod | awk '$1 ~ /^(nvidia|nvidia_uvm|kvm|vfio_pci)$/ {print "lsmod",$1,$2,$3}'
  echo "lock_holders: $(fuser /tmp/kayfabe-fastguest.lock 2>&1 | tr '\n' ' ')"
} > "$O/state_$L.txt"
cat "$O/state_$L.txt"
