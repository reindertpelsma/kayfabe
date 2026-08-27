#!/bin/bash
# Clean Mode-2 RmInitAdapter probe: single GSP-boot attempt, gdm stopped,
# device held open by a background fd so nvidia-smi can observe it.
set +e
KO="$HOME/nvmods/nvidia.ko"
sudo systemctl isolate multi-user.target 2>/dev/null
sleep 2
sudo rmmod nvidia 2>/dev/null
sudo modprobe ecdh_generic ecc 2>/dev/null
sudo mknod /dev/nvidia0 c 195 0 2>/dev/null
sudo mknod /dev/nvidiactl c 195 255 2>/dev/null
sudo chmod 666 /dev/nvidia0 /dev/nvidiactl
sudo dmesg -C
echo "=== insmod (retry=1) ==="
sudo insmod "$KO" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
sleep 2
echo "=== open-hold probe (fd 9), 28s timeout on open-hang ==="
( timeout 30 bash -c 'exec 9<>/dev/nvidia0; rc=$?; if [ $rc -eq 0 ]; then echo PROBE_OPEN_OK; else echo PROBE_OPEN_FAIL_rc=$rc; fi; sleep 26' ) > /tmp/probe_hold.out 2>&1 &
sleep 9
echo "--- probe_hold.out ---"; cat /tmp/probe_hold.out
echo "=== nvidia-smi while held ==="
sudo timeout 18 nvidia-smi 2>&1 | head -16
echo "=== dmesg verdict ==="
sudo dmesg | grep -aiE "RmInitAdapter succeeded|RmInitAdapter failed|general protection fault|BUG: unable|rm_init_adapter failed|Cannot initialize GSP" | head -6
wait 2>/dev/null
