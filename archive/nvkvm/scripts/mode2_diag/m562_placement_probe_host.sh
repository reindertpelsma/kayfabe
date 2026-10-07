#!/usr/bin/env bash
# m562_placement_probe_host.sh — ON HOST. First Mode-2 boot on the OPEN-580 RM core
# built FROM SOURCE (research_clones/open-580-guest), not the prebuilt DKMS blob.
#
# Two goals in one fresh boot:
#  (1) REGRESSION: does the from-source open RM core insmod + cuInit + reach
#      cuMemAlloc/HtoD in Mode-2 at all (faked GSP is tuned to 580.159.04)?
#  (2) PLACEMENT: capture the NVKVM-FBREG (FB region table the guest RM derives
#      from our faked GspStaticConfigInfo) and NVKVM-PLACE (how a >=1MB userspace
#      cuMemAlloc is placed: pmaInit / bIsPmaAlloc / attr) printks compiled into
#      the open RM core (src/nvidia mem_mgr_gsp_client.c + video_mem.c). These
#      settle WHY libcuda sees dp as CPU-accessible (slow CPU HtoD) instead of
#      real vidmem (fast CE) the way the host oracle does.
#
# OGKM points at the lean self-contained open tree (kernel-open contents +
# materialized nv-kernel.o_binary carrying the printks + the 0xFFF500 uvm patch).
# Guest rebuilds only the kernel-open kbuild stage against 6.8.0-117 (== DKMS flow).
# Fresh boot + sole client. Guest-debug build only; never NVKVM_FRESH=1.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
MB=${CUP6_MB:-64}
D=/workspace/nvkvm
export OGKM="${OGKM:-/workspace/nvkvm/research_clones/open-580-guest}"

echo "==> OGKM (open-from-source tree) = $OGKM"
ls "$OGKM/Kbuild" >/dev/null || { echo "OGKM tree missing Kbuild — abort"; exit 1; }
grep -qc NVKVM-FBREG "$OGKM/nvidia/nv-kernel.o_binary" 2>/dev/null
strings "$OGKM/nvidia/nv-kernel.o_binary" | grep -q NVKVM-PLACE && echo "  o_binary carries placement printks" || echo "  WARN: printks missing in o_binary"

echo "==> fresh restart VM (DEFAULT mode, OGKM exported into run_mode2_vm.sh)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
OGKM="$OGKM" nohup bash $D/scripts/run_mode2_vm.sh >/tmp/m562_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -5 /tmp/m562_launch.log; exit 1; }

echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }

echo "==> confirm guest sees the open tree over 9p (Kbuild + o_binary with printks)"
$SSHG 'sudo mkdir -p /mnt/ogkm; mountpoint -q /mnt/ogkm || sudo mount -t 9p -o trans=virtio,version=9p2000.L,ro ogkm /mnt/ogkm; ls -la /mnt/ogkm/nvidia/nv-kernel.o_binary; grep -c 0xFFF500 /mnt/ogkm/nvidia-uvm/uvm_channel.c' 2>&1 | tail -3

echo "==> stage probes + scripts"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  $D/tests/mode2/cup6.c $D/tests/mode2/rtp.c \
  $D/scripts/mode2_diag/rtp_run_guest.sh $D/scripts/mode2_diag/rebuild_guest_mods.sh \
  ubuntu@localhost:/tmp/ 2>/dev/null && echo "  staged"

echo "==> REBUILD guest modules from OPEN-580 source (kernel-open stage vs 6.8.0-117)"
$SSHG 'bash /tmp/rebuild_guest_mods.sh' 2>&1 | tail -12

echo "==> setup (insmod from-source open nvidia+uvm, pin adapter, build cup6)"
$SSHG 'bash /tmp/rtp_run_guest.sh setup' 2>&1 | tail -6

echo "==> cup6 CUP6_MB=$MB on pinned adapter (cuInit->cuCtxCreate->cuMemAlloc->3x HtoD)"
$SSHG "CUP6_MB=$MB CUP6_TIMEOUT=180 bash /tmp/rtp_run_guest.sh cup6" 2>&1

echo ""; echo "============ M562 OPEN-580 PLACEMENT PROBE (MB=$MB) ============"
echo "--- NVKVM-FBREG (FB region table guest RM derived from faked GSP) ---"
$SSHG 'sudo dmesg | grep "NVKVM-FBREG"' 2>&1
echo "--- NVKVM-PLACE (cuMemAlloc placement: pmaInit/bIsPmaAlloc/attr) ---"
$SSHG 'sudo dmesg | grep "NVKVM-PLACE"' 2>&1
echo "--- NVRM init/boot health (regression check) ---"
$SSHG 'sudo dmesg | grep -iE "NVRM|GSP|WPR2|Booter|Xid|assert" | tail -15' 2>&1
echo "============ END ============"
