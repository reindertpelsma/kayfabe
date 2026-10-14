#!/bin/bash
# iommu_type.sh TYPE — switch the RTX 4070's IOMMU group (01:00.0 + 01:00.1) to TYPE (identity | DMA-FQ)
# at runtime: GNOME/gdm stopped, nvidia modules unloaded, the audio function unbound, the type written,
# everything restored. Demos must already be stopped. Logs every step; always tries to restore the desktop.
set -u
T=${1:?type}
G=$(basename "$(readlink /sys/bus/pci/devices/0000:01:00.0/iommu_group)")
echo "IOMMU_SWITCH_START group=$G from=$(cat /sys/kernel/iommu_groups/$G/type) to=$T $(date -Is)"
pgrep -a qemu-system && { echo "QEMU still running - refusing"; exit 2; }
systemctl stop gdm; sleep 3
systemctl stop nvidia-persistenced 2>/dev/null
for i in 1 2 3 4 5; do
  users=$(fuser /dev/nvidia* 2>/dev/null | tr -s ' ')
  [ -z "$users" ] && break
  echo "still open by: $users"; fuser -k /dev/nvidia* 2>/dev/null; sleep 2
done
rc=0
for m in nvidia_drm nvidia_modeset nvidia_uvm nvidia; do rmmod $m || { echo "rmmod $m FAILED"; rc=1; }; done
if [ $rc = 0 ]; then
  echo 0000:01:00.1 > /sys/bus/pci/drivers/snd_hda_intel/unbind || echo "audio unbind failed"
  if echo "$T" > /sys/kernel/iommu_groups/$G/type; then echo "TYPE WRITTEN"; else echo "TYPE WRITE FAILED rc=$?"; fi
  echo 0000:01:00.1 > /sys/bus/pci/drivers/snd_hda_intel/bind || echo "audio rebind failed"
fi
echo "type now: $(cat /sys/kernel/iommu_groups/$G/type)"
modprobe nvidia && modprobe nvidia_uvm && modprobe nvidia_modeset && modprobe nvidia_drm || echo "MODPROBE FAILED"
systemctl start nvidia-persistenced 2>/dev/null
systemctl start gdm
sleep 8
nvidia-smi --query-gpu=name,memory.used --format=csv,noheader
systemctl is-active gdm
echo "IOMMU_SWITCH_END type=$(cat /sys/kernel/iommu_groups/$G/type) $(date -Is)"
