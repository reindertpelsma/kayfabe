#!/usr/bin/env bash
# m560_copy_discriminator_host.sh — ON HOST. Clean fresh-boot run of cup6 as the
# SOLE client (avoids the multi-client / post-Xid contamination that hung cup6 on a
# reused adapter). cup6 times 3 HtoD to the SAME buffer:
#   #1~#2~#3 all ~slow  => UC-persistent (cacheability). WB RAM memcpy would be GB/s.
#   #2/#3 >> #1         => first-touch fault/residency overhead (the per-page path).
# Also dumps the live pat_memtype_list memtype summary while the buffer is held.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
MB=${CUP6_MB:-64}

echo "==> fresh restart VM (DEFAULT mode)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -x qemu-system-x86_64 >/dev/null || pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }
echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }

echo "==> stage + setup (pin adapter)"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/tests/mode2/cup5.c /workspace/nvkvm/tests/mode2/cup6.c \
  /workspace/nvkvm/tests/mode2/rtp.c /workspace/nvkvm/tests/mode2/reload_probe.c \
  /workspace/nvkvm/scripts/mode2_diag/rtp_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null
$SSHG 'bash /tmp/rtp_run_guest.sh setup' 2>&1 | tail -3

echo "==> host GPU util sampler (1s, 40s)"
( for t in $(seq 1 40); do echo "t${t}s: $(nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader 2>/dev/null)"; sleep 1; done ) > /tmp/m560_util.log 2>&1 &
USP=$!

echo "==> cup6 CUP6_MB=$MB on pinned adapter (3 timed copies + pat dump)"
$SSHG "CUP6_MB=$MB CUP6_TIMEOUT=180 bash /tmp/rtp_run_guest.sh cup6" 2>&1
kill $USP 2>/dev/null

echo ""; echo "============ M560 COPY DISCRIMINATOR (MB=$MB) ============"
echo "--- host util peak ---"; grep -oE "[0-9]+ %" /tmp/m560_util.log | sort -rn | uniq -c | head -3
echo "============ END ============"
