#!/usr/bin/env bash
# m558_bulk_dataplane_host.sh — ON HOST. Classify the LLM model-load hang with a
# controlled, deterministic, llama-free reproducer: a large driver-API HtoD/DtoH
# (cup5) on a PINNED single-init adapter. Reads host GPU util during the copy.
# Outcome classifies the blocker (see cup5.c header). No QEMU rebuild (DEFAULT mode).
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
QLOG=/tmp/m0_qemu.log
MB=${CUP5_MB:-64}

echo "==> fresh restart VM (DEFAULT mode)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }
echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }

echo "==> stage cup5 + runner"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/tests/mode2/cup5.c /workspace/nvkvm/tests/mode2/rtp.c \
  /workspace/nvkvm/tests/mode2/reload_probe.c \
  /workspace/nvkvm/scripts/mode2_diag/rtp_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null

echo "==> setup (modules + PIN adapter)"
$SSHG 'bash /tmp/rtp_run_guest.sh setup' 2>&1 | tail -4

QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
echo "==> host GPU util sampler (1s cadence, 130s)"
( for t in $(seq 1 130); do echo "t${t}s: $(nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader 2>/dev/null)"; sleep 1; done ) > /tmp/m558_util.log 2>&1 &
USP=$!

echo "==> cup5 CUP5_MB=$MB on pinned adapter"
$SSHG "CUP5_MB=$MB CUP5_TIMEOUT=120 bash /tmp/rtp_run_guest.sh cup5" 2>&1
kill $USP 2>/dev/null
QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/m558_delta.txt

echo ""; echo "============ M558 BULK DATA-PLANE EVIDENCE ============"
echo "--- host util peak (>0 = host CE ran the copy; ~0 = QEMU-emulated) ---"
grep -oE "[0-9]+ %" /tmp/m558_util.log | sort -rn | uniq -c | head -5
echo "--- QEMU-emulated CE ops during cup5 (M5.15 DMAW / M5: CE) ---"
grep -cE "M5.15 DMAW|M5: CE" /tmp/m558_delta.txt | sed 's/^/  emulated-op lines: /'
echo "--- host fetch evidence (rings) ---"; grep -cE "RANG|gp_get=" /tmp/m558_delta.txt | sed 's/^/  ring lines: /'
echo "--- faults / NO_MEMORY / st=0x57 ---"; grep -cE "FAULT|NO_MEMORY|st=0x57" /tmp/m558_delta.txt | sed 's/^/  count: /'
echo "--- host Xid ---"; sudo dmesg | grep -iE "Xid" | tail -3
echo "--- health ---"; uptime | grep -oE "load average.*"
echo "============ END ============"
