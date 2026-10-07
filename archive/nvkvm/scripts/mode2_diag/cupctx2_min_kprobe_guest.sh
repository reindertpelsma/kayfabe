#!/usr/bin/env bash
# cupctx2_min_kprobe_guest.sh — ON GUEST. Same module/node setup as
# cupctx2_min_run_guest.sh, then kprobe the open driver's
# channelWaitForFinishPayload(OBJCHANNEL *pChannel /*rdi*/, NvU64 targetPayload /*rsi*/)
# to capture {channel ptr, targetPayload} at entry + a kretprobe for returns.
# Runs cupctx2_min (create->destroy->create) and dumps the trace: the LAST cwfp
# entry with NO matching return = the CTX2 hang's wait target. No module rebuild
# (channelWaitForFinishPayload lives in the precompiled RM core).
set -u
NVMODS=/home/ubuntu/nvmods
GUESTLIB=/usr/local/nvidia-guest/lib
TIMEOUT=${MIN_TIMEOUT:-60}
T=/sys/kernel/tracing
sudo systemctl isolate multi-user.target 2>/dev/null || true
sleep 2
sudo rmmod nvidia_uvm nvidia nvkvm_guest 2>/dev/null || true
sudo modprobe ecdh_generic ecc 2>/dev/null || true
sudo dmesg -C || true
if [ ! -f /lib/firmware/nvidia/580.159.04/gsp_ga10x.bin ]; then
  sudo mkdir -p /lib/firmware/nvidia/580.159.04 /mnt/nvfw
  sudo mount -t 9p -o trans=virtio,version=9p2000.L,ro nvfw /mnt/nvfw 2>/dev/null || true
  sudo cp -n /mnt/nvfw/*.bin /lib/firmware/nvidia/580.159.04/ 2>/dev/null || true
  sudo umount /mnt/nvfw 2>/dev/null || true
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
sudo ln -sf "$GUESTLIB/libcuda.so.580.159.04" /lib/x86_64-linux-gnu/libcuda.so.1

echo "=== kprobe setup (module loaded; symbol in nvidia core) ==="
sudo bash -c "echo > $T/kprobe_events" 2>/dev/null || true
if ! sudo bash -c "echo 'p:cwfp channelWaitForFinishPayload ch=%di target=%si' > $T/kprobe_events" 2>/tmp/kperr; then
  echo "KPROBE ADD FAILED:"; cat /tmp/kperr
  echo "  symbol present?"; sudo grep -i channelWaitForFinishPayload /proc/kallsyms | head -1
  exit 1
fi
sudo bash -c "echo 'r:cwfpr channelWaitForFinishPayload ret=\$retval' >> $T/kprobe_events" 2>>/tmp/kperr || echo "  (kretprobe add warn: $(cat /tmp/kperr))"
sudo bash -c "echo 65536 > $T/buffer_size_kb" 2>/dev/null || true
sudo bash -c "echo 1 > $T/events/kprobes/cwfp/enable"
sudo bash -c "echo 1 > $T/events/kprobes/cwfpr/enable" 2>/dev/null || true
sudo bash -c "echo > $T/trace"
echo "  kprobes armed:"; sudo cat $T/kprobe_events

gcc -O0 -g -o /tmp/cupctx2_min /tmp/cupctx2_min.c -lcuda 2>&1 | tail -3
echo "=== cupctx2_min (timeout ${TIMEOUT}s, ITERS=${ITERS:-2}) ==="
LD_LIBRARY_PATH=$GUESTLIB ITERS=${ITERS:-2} timeout --signal=INT "$TIMEOUT" stdbuf -oL -eL /tmp/cupctx2_min
echo "=== cupctx2_min exit rc=$? ==="

sudo bash -c "echo 0 > $T/events/kprobes/cwfp/enable" 2>/dev/null || true
sudo bash -c "echo 0 > $T/events/kprobes/cwfpr/enable" 2>/dev/null || true
echo "=== cwfp/cwfpr counts ==="
echo "  entries (cwfp): $(sudo grep -c ' cwfp:' $T/trace 2>/dev/null)"
echo "  returns (cwfpr): $(sudo grep -c ' cwfpr:' $T/trace 2>/dev/null)"
echo "=== LAST 30 cwfp/cwfpr trace lines (last cwfp w/o matching cwfpr = the hang) ==="
sudo grep -E ' cwfp:| cwfpr:' $T/trace 2>/dev/null | tail -30
echo "=== distinct (ch,target) entry values seen ==="
sudo grep ' cwfp:' $T/trace 2>/dev/null | grep -oE 'ch=0x[0-9a-f]+ target=0x[0-9a-f]+' | sort | uniq -c | tail -20
sudo bash -c "echo > $T/kprobe_events" 2>/dev/null || true
echo "=== DONE_KPROBE ==="
