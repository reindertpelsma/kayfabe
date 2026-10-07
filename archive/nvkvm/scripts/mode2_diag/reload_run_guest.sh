#!/usr/bin/env bash
NVMODS=/home/ubuntu/nvmods; GUESTLIB=/usr/local/nvidia-guest/lib
stage_fw(){
  if [ ! -f /lib/firmware/nvidia/580.159.04/gsp_ga10x.bin ]; then
    sudo mkdir -p /lib/firmware/nvidia/580.159.04 /mnt/nvfw
    sudo mount -t 9p -o trans=virtio,version=9p2000.L,ro nvfw /mnt/nvfw 2>/dev/null || true
    sudo cp -n /mnt/nvfw/*.bin /lib/firmware/nvidia/580.159.04/ 2>/dev/null || true
    sudo umount /mnt/nvfw 2>/dev/null || true
  fi
  ls /lib/firmware/nvidia/580.159.04/gsp_ga10x.bin >/dev/null 2>&1 && echo "fw OK" || echo "fw MISSING"
}
load_mods(){
  echo -n "insmod nvidia: "; sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_DynamicPowerManagement=0 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 && echo ok
  echo -n "insmod uvm: ";    sudo insmod "$NVMODS/nvidia-uvm.ko" 2>&1 && echo ok
  UVM_MAJ=$(awk '$2=="nvidia-uvm"{print $1}' /proc/devices)
  sudo mknod /dev/nvidia0 c 195 0 2>/dev/null; sudo mknod /dev/nvidiactl c 195 255 2>/dev/null
  [ -n "$UVM_MAJ" ] && { sudo rm -f /dev/nvidia-uvm /dev/nvidia-uvm-tools; sudo mknod /dev/nvidia-uvm c "$UVM_MAJ" 0; sudo mknod /dev/nvidia-uvm-tools c "$UVM_MAJ" 1; sudo chmod 666 /dev/nvidia-uvm*; }
  sudo chmod 666 /dev/nvidia0 /dev/nvidiactl
  echo "lsmod: $(lsmod | grep -E '^nvidia ' | awk '{print $1,$3}')"
}
case "$1" in
  setup) sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
         sudo rmmod nvidia_uvm nvidia nvkvm_guest 2>/dev/null; sudo modprobe ecdh_generic ecc 2>/dev/null; sudo dmesg -C
         stage_fw; load_mods
         gcc -O0 -o /tmp/dvp /tmp/reload_probe.c -lcuda -L"$GUESTLIB" 2>&1 | tail -2
         [ -x /tmp/dvp ] && echo "probe built" || echo "PROBE BUILD FAILED"; echo SETUP_DONE;;
  dvp)   LD_LIBRARY_PATH="$GUESTLIB" timeout 45 /tmp/dvp;;
  reload) echo "-- rmmod --"; sudo rmmod nvidia_uvm nvidia 2>&1; echo "-- reinsmod --"; load_mods;;
  dmesg) sudo dmesg | grep -iE "WPR2|RmInit|Booter|nvAssert|GSP|UNLOAD" | tail -20;;
esac
