#!/bin/bash
# iommu_nogdm.sh TYPE — switch the RTX 4070's IOMMU group to TYPE WITHOUT touching gdm (GNOME runs on
# the iGPU since 2026-10-08). Refuses if any process holds /dev/nvidia* or nvidia_drm is in use.
set -u
T=${1:?type}
G=$(basename "$(readlink /sys/bus/pci/devices/0000:01:00.0/iommu_group)")
echo "IOMMU_SWITCH_START group=$G from=$(cat /sys/kernel/iommu_groups/$G/type) to=$T $(date -Is) (gdm untouched)"
pgrep -a qemu-system && { echo "QEMU still running - refusing"; exit 2; }
users=$(fuser /dev/nvidia* 2>/dev/null | tr -s ' ')
[ -n "$users" ] && { echo "REFUSED: /dev/nvidia* open by: $users"; exit 2; }
[ "$(awk '$1=="nvidia_drm"{print $3}' /proc/modules)" = 0 ] || { echo "REFUSED: nvidia_drm in use"; exit 2; }
systemctl stop nvidia-persistenced 2>/dev/null
rc=0
for m in nvidia_drm nvidia_modeset nvidia_uvm nvidia; do rmmod $m || { echo "rmmod $m FAILED"; rc=1; }; done
if [ $rc = 0 ]; then
  echo 0000:01:00.1 > /sys/bus/pci/drivers/snd_hda_intel/unbind || echo "audio unbind failed"
  if echo "$T" > /sys/kernel/iommu_groups/$G/type; then echo "TYPE WRITTEN"; else echo "TYPE WRITE FAILED rc=$?"; fi
  echo 0000:01:00.1 > /sys/bus/pci/drivers/snd_hda_intel/bind || echo "audio rebind failed"
fi
modprobe nvidia && modprobe nvidia_uvm && modprobe nvidia_modeset && modprobe nvidia_drm || echo "MODPROBE FAILED"
systemctl start nvidia-persistenced 2>/dev/null
nvidia-smi --query-gpu=name,memory.used --format=csv,noheader
echo "IOMMU_SWITCH_END type=$(cat /sys/kernel/iommu_groups/$G/type) $(date -Is)"
