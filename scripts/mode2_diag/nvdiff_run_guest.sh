#!/usr/bin/env bash
# nvdiff_run_guest.sh — runs ON THE MODE-2 GUEST. Captures the guest half of the
# host<->guest ioctl differential with the SAME instrument the host reference was
# taken with (tests/mode2/nvdiff/).
#
# The symmetry is the whole point: one shim, two subjects. Do not fork this file's
# capture logic -- it calls nvd_capture.sh unchanged.
#
# Stage the three sources into /tmp first (bench_boot.sh wipes the guest overlay
# every boot, so re-scp on EVERY cycle):
#     nvdiff_shim.c  nvd_prog.c  nvd_capture.sh  uvm_sizes.h
#
#   ssh vg 'bash -s' < scripts/mode2_diag/nvdiff_run_guest.sh          # default stage ctx
#   NVD_STAGE=ce NVD_RUNS=2 ssh vg 'bash -s' < .../nvdiff_run_guest.sh
#
# Then copy /tmp/nvd_guest/*.jsonl back to the dev box and:
#   python3 tests/mode2/nvdiff/nvdiff.py diff \
#       traces/host_reference_ga106/<stage>_r1.jsonl  <guest>.jsonl
#
# ⚠ The guest driver must already be loaded. This script does the same module /
# node / libcuda prep as cup2_run_guest.sh so it can be run standalone on a fresh
# overlay; it is idempotent if the driver is already up.
#
# ★★ TWO GUEST LAYOUTS, and the prep below must DETECT which one it is on.
# The C artifact's Mode-2 overlay carries a hand-staged driver (~/nvmods/nvidia.ko) and a
# side-installed userspace (/usr/local/nvidia-guest/lib/libcuda.so.580.159.04). The kayfabe
# bench guest (/workspace/bench/guest.qcow2 on vh/vh2) carries the DISTRIBUTION packages:
# an in-tree /lib/modules/.../nvidia.ko autoloaded at boot, and libcuda in
# /lib/x86_64-linux-gnu. `[measured 2026-08-10, vh2 boot nvd1]` Running the unguarded
# version of this prep on the kayfabe guest would have `ln -sf`'d
# /lib/x86_64-linux-gnu/libcuda.so.1 -> /usr/local/nvidia-guest/lib/libcuda.so.580.159.04,
# a path that does not exist there -- i.e. the prep step would have BROKEN the very library
# the workload links against, and the failure would have read as "the guest cannot run CUDA".
# ⊘ A prep step that is wrong for the layout it runs on is indistinguishable from the
# defect it is meant to prepare for. Hence: detect, never assume.
set -u
NVMODS=${NVMODS:-/home/ubuntu/nvmods}
NVGUEST_LIB=${NVGUEST_LIB:-/usr/local/nvidia-guest/lib}
STAGE=${NVD_STAGE:-ctx}
RUNS=${NVD_RUNS:-2}
SRC=${NVD_SRC:-/tmp}
OUT=${NVD_OUT:-/tmp/nvd_guest}

for f in nvdiff_shim.c nvd_prog.c nvd_capture.sh uvm_sizes.h; do
    [ -f "$SRC/$f" ] || { echo "FATAL: $SRC/$f missing -- re-scp after every fresh boot"; exit 2; }
done

if [ ! -e /dev/nvidiactl ] && [ ! -f "$NVMODS/nvidia.ko" ]; then
    # --- kayfabe bench guest: stock in-tree module, already autoloaded by udev. ---
    # RmInitAdapter runs on the first open() of /dev/nvidia0, and that node does not exist
    # until something creates it; nvidia-modprobe is the sanctioned creator (CLAUDE.md /
    # boot_capture.sh: "modprobe does not run RmInitAdapter").
    echo "== prep: stock in-tree layout (no $NVMODS/nvidia.ko) -- creating nodes via nvidia-modprobe"
    sudo modprobe nvidia 2>/dev/null || true
    sudo nvidia-modprobe -c 0 -u 2>&1 | head -3 || true
    sudo chmod 666 /dev/nvidia* /dev/nvidiactl 2>/dev/null || true
elif [ ! -e /dev/nvidiactl ]; then
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
    sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 \
        NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1 || true
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
fi
# each fresh overlay loses the unversioned dev symlink that `-lcuda` needs.
# ⊘ Guarded: on the kayfabe bench guest $NVGUEST_LIB does not exist and libcuda is already a
# working distribution install -- pointing the loader at an absent file would break it.
if [ -f "$NVGUEST_LIB/libcuda.so.580.159.04" ]; then
    sudo ln -sf "$NVGUEST_LIB/libcuda.so.580.159.04" /lib/x86_64-linux-gnu/libcuda.so.1
    sudo ln -sf "$NVGUEST_LIB/libcuda.so.580.159.04" \
        /usr/lib/x86_64-linux-gnu/libcuda.so 2>/dev/null || true
    sudo ldconfig 2>/dev/null || true
    export LD_LIBRARY_PATH=$NVGUEST_LIB${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
else
    echo "== prep: $NVGUEST_LIB absent -- using the distribution libcuda as-is"
    ldconfig -p | grep -m1 'libcuda\.so\.1' || echo "WARN: no libcuda.so.1 in the cache"
    # -lcuda needs the unversioned name; create it only if it is genuinely missing.
    [ -e /usr/lib/x86_64-linux-gnu/libcuda.so ] || \
        sudo ln -sf /lib/x86_64-linux-gnu/libcuda.so.1 /usr/lib/x86_64-linux-gnu/libcuda.so
fi
# ★ The host reference was captured at NVDIFF_MAXBUF=65536; the default is 8192, which
# TRUNCATES 27 records. A guest capture taken at the default would diff against the
# reference as a wall of SIZE/VALUE divergences that are properties of the INSTRUMENT.
export NVDIFF_MAXBUF=${NVDIFF_MAXBUF:-65536}
export NVD_TIMEOUT=${NVD_TIMEOUT:-180}
cd "$SRC" || exit 2
bash "$SRC/nvd_capture.sh" "$OUT" "$STAGE" "$RUNS"
RC=$?
echo "=== guest capture rc=$RC ==="
# The evidence must be ASSERTED, not assumed: an existing file is not a capture.
for f in "$OUT/${STAGE}"_r*.jsonl; do
    [ -s "$f" ] || { echo "FATAL: $f EMPTY"; RC=1; continue; }
    echo "  $(wc -l < "$f") records  $f"
done
# Persist the guest's own dmesg beside the capture -- the serial log is NOT where
# the driver's output is (CLAUDE.md, measured 2026-08-01).
sudo dmesg > "$OUT/${STAGE}_dmesg.log" 2>/dev/null || true
if ! grep -qi NVRM "$OUT/${STAGE}_dmesg.log" 2>/dev/null; then
    echo "WARN: guest dmesg contains no NVRM lines -- driver output not captured"
fi
exit $RC
