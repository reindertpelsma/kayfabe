#!/usr/bin/env bash
# m540_test_host.sh — ON HOST. Build QEMU (M5.40 COPY-TSG shared-VAS fix), fresh
# restart, run cup2 (full cuCtxCreate→memcpy), sample host GPU util, then print the
# decisive M5.40 / GPFIFO_SCHEDULE / fault / Xid signals.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
QLOG=/tmp/m0_qemu.log

echo "==> build QEMU"
( cd /opt/qemu-src/build && ninja qemu-system-x86_64 && ninja install ) 2>&1 | tail -2 || { echo BUILD_FAIL; exit 1; }

echo "==> fresh restart VM"
pkill -9 qemu-system 2>/dev/null; sleep 3
nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep qemu-system >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }

echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }

echo "==> stage cup2 + run script"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/tests/mode2/cup2.c /workspace/nvkvm/scripts/mode2_diag/step1_userd_guest.sh \
  ubuntu@localhost:/tmp/ 2>/dev/null
# Reuse step1 guest prep (firmware+modules+nodes) but run cup2 in FOREGROUND with timeout.
QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)

echo "==> host GPU util sampler (background, 70s)"
( for t in $(seq 1 35); do echo "util_t$((t*2))s: $(nvidia-smi --query-gpu=utilization.gpu,utilization.memory,memory.used --format=csv,noheader 2>/dev/null)"; sleep 2; done ) > /tmp/m540_util.log 2>&1 &

echo "==> run cup2 (full, timeout 70)"
$SSHG 'bash /tmp/step1_userd_guest.sh' > /tmp/m540_guest.log 2>&1 &
GP=$!
# step1_userd_guest backgrounds cup2 + waits 45s; we let it run, then read its cup2 stdout.
wait $GP 2>/dev/null || true
echo "--- guest cup2 stdout (CTX/MEMALLOC/CE/DONE?) ---"; grep -iE "cuInit|CTX|MEMALLOC|CE |PASS|DONE|error|hang|cuCtx" /tmp/m540_guest.log | head -20

QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/m540_delta.txt

echo ""; echo "============ M5.40 EVIDENCE ============"
echo "--- M5.40 COPY-TSG VAS rewrites ---"; grep -E "M5.40 a06c COPY TSG" /tmp/m540_delta.txt | head
echo "--- M5.41 COPY TSG bind+sched results ---"; grep -E "M5.41 COPY TSG bind" /tmp/m540_delta.txt | sed -E "s/.*(TSG=0x[0-9a-f]+).*(engineType=0x[0-9a-f]+).*(bind rc=[0-9-]+ st=0x[0-9a-f]+).*(sched rc=[0-9-]+ st=0x[0-9a-f]+).*/\1 \2 \3 | \4/" | sort | uniq -c
echo "--- GPFIFO_SCHEDULE results (was st=0x57) ---"; grep -E "GPFIFO_SCHEDULE" /tmp/m540_delta.txt | sed -E "s/.*(TSG=0x[0-9a-f]+).*(st=0x[0-9a-f]+).*/\1 \2/" | sort | uniq -c
echo "--- populate_cvas for COPY TSGs ---"; grep -E "populate_cvas" /tmp/m540_delta.txt | grep -iE "5c000049|5c00003b" | head
echo "--- compute working-set VA FAULTs (0x2002xxxxx) — should shrink/vanish ---"; grep -cE "eva=0x2002[0-9a-f]+ -> FAULT" /tmp/m540_delta.txt | sed 's/^/  fault count: /'
echo "--- M5.42 cursor align (host GP_GET set to consume index) ---"; grep -E "M5.42 align host GP_GET" /tmp/m540_delta.txt | head
echo "--- M5.22 host doorbell rings: hostUSERD put/get for COPY channels (does host FETCH?) ---"
grep -E "M5.22 RANG" /tmp/m540_delta.txt | sed -E "s/.*(client=0x[0-9a-f]+).*(hostUSERD put=[0-9]+ get=[0-9]+).*/\1 \2/" | sort | uniq -c
echo "  (raw last 12 M5.22 lines):"; grep -E "M5.22 RANG" /tmp/m540_delta.txt | tail -12
echo "--- host Xid during/after this run (CE/MMU faults) ---"; sudo dmesg | grep -iE "Xid" | tail -8
echo "--- host GPU util (peak) ---"; sort -t: -k2 /tmp/m540_util.log 2>/dev/null | grep -oE "[0-9]+ %" | sort -rn | head -3; tail -3 /tmp/m540_util.log
echo "--- host dmesg Xid/dmaAlloc tail ---"; sudo dmesg 2>/dev/null | grep -iE "xid|dmaAllocMapping" | tail -6
echo "============ END ============"
