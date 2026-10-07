#!/usr/bin/env bash
# step1_userd_guest.sh — runs ON THE GUEST. Boots cup2 in the background, then
# mid-cuCtxCreate-hang classifies every nvidia CPU mapping as RAM-backed vs
# PCI-BAR(MMIO) via /proc/PID/pagemap, and reads each candidate USERD page's
# GP_GET(+0x88)/GP_PUT(+0x8c). This settles, with zero inference, whether
# libcuda's GP_PUT write lands in a trapped BAR1 page or an untrapped RAM window.
set -u
NVMODS=/home/ubuntu/nvmods
BDF=0000:00:07.0
sudo systemctl isolate multi-user.target 2>/dev/null || true
sleep 2
sudo rmmod nvidia_uvm nvidia nvkvm_guest 2>/dev/null || true
sudo modprobe ecdh_generic ecc 2>/dev/null || true
sudo dmesg -C || true
# Stage GSP firmware .04 (base ships .03; the .04 driver needs .04 or cuInit 999).
if [ ! -f /lib/firmware/nvidia/580.159.04/gsp_ga10x.bin ]; then
  sudo mkdir -p /lib/firmware/nvidia/580.159.04 /mnt/nvfw
  sudo mount -t 9p -o trans=virtio,version=9p2000.L,ro nvfw /mnt/nvfw 2>/dev/null || true
  sudo cp -n /mnt/nvfw/*.bin /lib/firmware/nvidia/580.159.04/ 2>/dev/null || true
  sudo umount /mnt/nvfw 2>/dev/null || true
  echo "  staged GSP firmware .04: $(ls /lib/firmware/nvidia/580.159.04/ | tr '\n' ' ')"
fi
sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1 || true
sudo insmod "$NVMODS/nvidia-uvm.ko" 2>&1 | tail -1 || true
UVM_MAJ=$(awk '$2=="nvidia-uvm"{print $1}' /proc/devices)
sudo mknod /dev/nvidia0 c 195 0 2>/dev/null || true
sudo mknod /dev/nvidiactl c 195 255 2>/dev/null || true
if [ -n "$UVM_MAJ" ]; then
  sudo rm -f /dev/nvidia-uvm /dev/nvidia-uvm-tools
  sudo mknod /dev/nvidia-uvm c "$UVM_MAJ" 0 2>/dev/null || true
  sudo mknod /dev/nvidia-uvm-tools c "$UVM_MAJ" 1 2>/dev/null || true
fi
sudo chmod 666 /dev/nvidia* /dev/nvidiactl 2>/dev/null || true
sudo ln -sf /usr/local/nvidia-guest/lib/libcuda.so.580.159.04 /lib/x86_64-linux-gnu/libcuda.so.1
gcc -O0 -g -o /tmp/cup2 /tmp/cup2.c -lcuda 2>&1 | tail -2

echo "=== PCI BAR GPA ranges ($BDF) ==="
# resource lines: start end flags ; BAR0=line0, BAR1=line1(+2 64-bit), BAR3=BAR2 aperture
awk 'NR<=6{printf "  bar%d: %s\n",NR-1,$0}' /sys/bus/pci/devices/$BDF/resource 2>/dev/null

echo "=== launching cup2 in background ==="
GUESTLIB=/usr/local/nvidia-guest/lib
LD_LIBRARY_PATH=$GUESTLIB stdbuf -oL -eL /tmp/cup2 > /tmp/cup2_out.log 2>&1 &
CUP=$!
echo "cup2 pid=$CUP; waiting 45s for cuCtxCreate to park..."
sleep 45
echo "=== cup2 stdout so far ==="; cat /tmp/cup2_out.log
if ! kill -0 $CUP 2>/dev/null; then echo "cup2 EXITED"; fi

echo "=== nvidia CPU mappings + RAM/BAR classification ==="
# pagemap entry: 8 bytes/page; bit63=present, bit62=swapped, bits0-54=PFN.
python3 - "$CUP" <<'PY'
import sys,os,re
pid=sys.argv[1]
def cls(va):
    try:
        with open(f"/proc/{pid}/pagemap","rb") as f:
            f.seek((va>>12)*8)
            ent=int.from_bytes(f.read(8),"little")
        present=(ent>>63)&1
        pfn=ent&((1<<55)-1)
        if present and pfn:
            return f"RAM present pfn=0x{pfn:x} GPA~0x{pfn<<12:x}"
        if present:
            return "present pfn=0 (special/MMIO)"
        return "NOT-present (likely MMIO/PCI-BAR or unfaulted)"
    except Exception as e:
        return f"pagemap-err {e}"
try:
    maps=open(f"/proc/{pid}/maps").read().splitlines()
except Exception as e:
    print("  no maps:",e); sys.exit(0)
for ln in maps:
    if "nvidia" not in ln: continue
    rng=ln.split()[0]; lo,hi=[int(x,16) for x in rng.split("-")]
    sz=hi-lo
    print(f"  {ln}")
    print(f"      size=0x{sz:x} first-page: {cls(lo)}")
PY

echo "=== read GP_GET(+0x88)/GP_PUT(+0x8c) on each small /dev/nvidia0 map (USERD candidates) ==="
# small (<=0x10000) /dev/nvidia0 mappings are USERD/doorbell-class; gdb read also
# tells us if the page faults on CPU read (pure MMIO) vs reads cleanly.
MAPS=$(grep -E "/dev/nvidia0" /proc/$CUP/maps 2>/dev/null | awk '{split($1,a,"-"); lo=strtonum("0x"a[1]); hi=strtonum("0x"a[2]); if(hi-lo<=0x10000) print a[1]}')
for base in $MAPS; do
  echo "--- /dev/nvidia0 map @0x$base ---"
  sudo gdb -batch -p $CUP -ex "set pagination off" \
    -ex "x/2xw 0x$base+0x88" \
    -ex "x/4xw 0x$base" 2>&1 | grep -vE "^\[|Reading|no debugging|warning|Download|Using host" | head -6
done

echo "=== guest dmesg (nvrm/xid) tail ==="; sudo dmesg 2>/dev/null | grep -iE 'nvrm|nvidia|xid' | tail -10
sudo kill -9 $CUP 2>/dev/null || true
