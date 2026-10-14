#!/bin/bash
# on the Vast box: live capture with the module engine while the Linux NVIDIA driver is reloaded in the middle
cd /root/kfwc2
rm -f cap1.kfgspcap mb1.txt
./capture_probe /sys/bus/pci/devices/0000:00:08.0 cap1.kfgspcap 80 mb1.txt > cap1.out 2>&1 &
CP=$!
sleep 6
for i in 1 2 3 4 5; do nvidia-smi -q > /dev/null 2>&1; nvidia-smi --query-gpu=clocks.sm,temperature.gpu --format=csv,noheader > /dev/null 2>&1; sleep 1; done
echo "T+$(date +%s) reload start" > phases.txt
systemctl stop nvidia-persistenced 2>/dev/null
fuser -v /dev/nvidia* 2>&1 | head -3 >> phases.txt
rmmod nvidia_uvm 2>>phases.txt; rmmod nvidia_drm 2>>phases.txt; rmmod nvidia_modeset 2>>phases.txt; rmmod nvidia 2>>phases.txt
lsmod | grep -c "^nvidia" >> phases.txt
sleep 3
modprobe nvidia 2>>phases.txt
sleep 3
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader >> phases.txt 2>&1
echo "T+$(date +%s) reload done" >> phases.txt
for i in $(seq 1 12); do nvidia-smi -q > /dev/null 2>&1; sleep 1; done
wait $CP
echo CAPTURE_DONE >> phases.txt
