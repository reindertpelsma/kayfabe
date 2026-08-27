#!/bin/bash
set +e
NVMODS=/home/ubuntu/nvmods
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 1
sudo rmmod nvkvm_guest 2>/dev/null
sudo mkdir -p /lib/firmware/nvidia/580.159.04
sudo mount -t 9p -o trans=virtio,version=9p2000.L,msize=1048576,ro nvfw /mnt 2>/dev/null && sudo cp -an /mnt/. /lib/firmware/nvidia/580.159.04/ 2>/dev/null; sudo umount /mnt 2>/dev/null
sudo modprobe ecdh_generic ecc 2>/dev/null
sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
sudo insmod "$NVMODS/nvidia-uvm.ko" 2>&1 | tail -1
UVMMAJ=$(awk "/nvidia-uvm/{print \$1}" /proc/devices)
sudo mknod /dev/nvidia0 c 195 0 2>/dev/null; sudo mknod /dev/nvidiactl c 195 255 2>/dev/null
sudo mknod /dev/nvidia-uvm c "$UVMMAJ" 0 2>/dev/null; sudo mknod /dev/nvidia-uvm-tools c "$UVMMAJ" 1 2>/dev/null
sudo chmod 666 /dev/nvidia* 2>/dev/null
sudo ln -sf libcuda.so.580.159.04 /lib/x86_64-linux-gnu/libcuda.so.1 2>/dev/null
rm -f /tmp/cup2; nvcc -o /tmp/cup2 /tmp/cup2.c -lcuda -L/lib/x86_64-linux-gnu 2>&1|head -1
echo "===== cup2 (output->file, synced) ====="
rm -f /tmp/cup2_out.txt
( while true; do sync; sleep 1; done ) &  SYNCPID=$!
LD_LIBRARY_PATH=/usr/local/nvidia-guest/lib:/lib/x86_64-linux-gnu timeout 60 stdbuf -oL -eL /tmp/cup2 > /tmp/cup2_out.txt 2>&1; echo "cup2 exit=$?" >> /tmp/cup2_out.txt
kill $SYNCPID 2>/dev/null; sync
echo "--- cup2_out.txt ---"; cat /tmp/cup2_out.txt
