#!/usr/bin/env bash
# m561_cache_probe_host.sh — ON HOST. Confirmation probe for the cup5/llama bulk-copy
# UC slowness: rebuild the guest nvidia.ko from the 9p ogkm source (now carrying a
# debug printk in nv-mmap.c's nv_encode_caching call sites — "NVKVM-CACHE FB/SYS ...")
# then run cup6 (64MB cuMemAlloc + 3 timed HtoD). The printk reveals, for any >=1MB
# userspace mapping, whether the driver picks cache_type UNCACHED(1)/WB(0)/WC(2) and
# via the FB (mmap_context->caching) or SYSMEM (at->cache_type) branch — definitively
# settling whether dp's slow copy is a UC userspace mapping (and which lever sets it),
# or kernel-side (no >=1MB userspace map appears => copy is in the prebuilt RM core).
# Fresh boot + sole client (cup6 hangs on a reused/Xid adapter). Guest-debug patch only.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
MB=${CUP6_MB:-64}
D=/workspace/nvkvm

echo "==> fresh restart VM (DEFAULT mode)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
nohup bash $D/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }

echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }

echo "==> stage probes + rebuild script"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  $D/tests/mode2/cup5.c $D/tests/mode2/cup6.c $D/tests/mode2/rtp.c $D/tests/mode2/reload_probe.c \
  $D/scripts/mode2_diag/rtp_run_guest.sh $D/scripts/mode2_diag/rebuild_guest_mods.sh \
  ubuntu@localhost:/tmp/ 2>/dev/null && echo "  staged"

echo "==> REBUILD guest nvidia.ko from 9p ogkm (with NVKVM-CACHE probe) — a few minutes"
$SSHG 'bash /tmp/rebuild_guest_mods.sh' 2>&1 | tail -8

echo "==> setup (rmmod->insmod rebuilt nvmods, pin adapter, build cup6)"
$SSHG 'bash /tmp/rtp_run_guest.sh setup' 2>&1 | tail -4

echo "==> cup6 CUP6_MB=$MB on pinned adapter"
$SSHG "CUP6_MB=$MB CUP6_TIMEOUT=180 bash /tmp/rtp_run_guest.sh cup6" 2>&1

echo ""; echo "============ M561 NVKVM-CACHE PROBE (MB=$MB) ============"
echo "--- NVKVM-CACHE lines from guest dmesg (cache_type: 0=WB 1=UC 2=WC) ---"
$SSHG 'sudo dmesg | grep "NVKVM-CACHE"' 2>&1
echo "============ END ============"
