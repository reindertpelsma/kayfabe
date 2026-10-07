#!/bin/bash
# Rebuild the open nvidia modules from the 9p OGKM source (one-time per fresh overlay),
# then run cup2 under nvtrace to check if the c7c0 paramsSize=0 fix gets the guest past ioctl 129.
set +e
NVMODS=/home/ubuntu/nvmods
if [ ! -f "$NVMODS/nvidia.ko" ] || [ ! -f "$NVMODS/nvidia-uvm.ko" ]; then
  echo "=== building open nvidia modules from 9p (one-time) ==="
  sudo mkdir -p /mnt/ogkm /mnt/build
  mountpoint -q /mnt/ogkm  || sudo mount -t 9p -o trans=virtio,version=9p2000.L,msize=1048576,ro ogkm /mnt/ogkm
  mountpoint -q /mnt/build || sudo mount -t tmpfs -o size=6G tmpfs /mnt/build
  sudo cp -aL /mnt/ogkm/. /mnt/build/ 2>/dev/null
  ( cd /mnt/build && sudo timeout 900 make modules -j"$(nproc)" >/tmp/ogkm_build.log 2>&1 ); echo "make rc=$? (124=build-timed-out)"
  mkdir -p "$NVMODS"
  for m in nvidia.ko nvidia-uvm.ko nvidia-modeset.ko nvidia-drm.ko; do
    k=$(find /mnt/build -name "$m" | head -1); [ -n "$k" ] && cp "$k" "$NVMODS/"
  done
  echo "built: $(ls $NVMODS/*.ko 2>/dev/null | wc -l) modules"; tail -3 /tmp/ogkm_build.log
fi
[ -f "$NVMODS/nvidia.ko" ] || { echo "BUILD FAILED"; tail -25 /tmp/ogkm_build.log; exit 1; }
gcc -O2 -o /tmp/nvtrace /tmp/mode2_diag/nvtrace.c 2>&1 | head -3
sudo ln -sf libcuda.so.580.159.04 "/usr/local/nvidia-guest/lib/libcuda.so.1"; sudo rm -f "/lib/x86_64-linux-gnu/libcuda.so.1"; sudo ln -sf libcuda.so.580.159.04 "/lib/x86_64-linux-gnu/libcuda.so.1"
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
# The emulated GPU presents as an NVIDIA device, so the distro nvidia driver auto-BINDS to it on
# boot (the deleted overlay used to blacklist it). Unbind + unload it so OUR built nvidia.ko loads.
sudo systemctl stop nvidia-persistenced gdm display-manager 2>/dev/null
for d in /sys/bus/pci/drivers/nvidia/0000:*; do b=$(basename "$d"); echo "$b" | sudo tee /sys/bus/pci/drivers/nvidia/unbind >/dev/null 2>&1; done
sudo rmmod nvidia_drm nvidia_modeset nvidia_uvm nvidia 2>/dev/null
# blacklist distro nvidia so future boots of this overlay are clean (restore deleted-overlay cfg)
printf 'blacklist nvidia\nblacklist nvidia_uvm\nblacklist nvidia_drm\nblacklist nvidia_modeset\n' | sudo tee /etc/modprobe.d/nvkvm-blacklist.conf >/dev/null
sudo modprobe ecdh_generic ecc 2>/dev/null
echo "nvidia loaded after unbind: $(lsmod | grep -c '^nvidia ')"
for n in "nvidia0 c 195 0" "nvidiactl c 195 255" "nvidia-uvm c 235 0" "nvidia-uvm-tools c 235 1"; do sudo mknod /dev/${n% *} ${n#* } 2>/dev/null; done
sudo chmod 666 /dev/nvidia* /dev/nvidiactl 2>/dev/null
sudo sysctl -w kernel.yama.ptrace_scope=0 2>/dev/null
sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
sudo insmod "$NVMODS/nvidia-uvm.ko" 2>&1 | tail -1; sudo chmod 666 /dev/nvidia-uvm* 2>/dev/null
rm -f /tmp/cup2; nvcc -o /tmp/cup2 /tmp/cup2.c -lcuda -L"/lib/x86_64-linux-gnu" 2>&1 | head -2
echo "=== cup2 under nvtrace ==="
LD_LIBRARY_PATH="/usr/local/nvidia-guest/lib:/lib/x86_64-linux-gnu" /tmp/nvtrace -o /tmp/guest_nvtrace.txt -- /tmp/cup2 2>&1 | tail -8
echo "guest ioctls=$(grep -c '^IOCTL' /tmp/guest_nvtrace.txt) (was 129 before fix)"
