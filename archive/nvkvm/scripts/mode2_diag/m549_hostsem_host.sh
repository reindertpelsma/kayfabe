#!/usr/bin/env bash
# m549_hostsem_host.sh — ON HOST. Prove HOST-ONLY completion.
#
# Identical to the M5.48-passing forwarded run EXCEPT NVKVM_M2HOSTSEM=1, which:
#   - turns ON the completion-sema fwd-map (host GPU writes the UVM CE tracking sema), and
#   - gates OFF every QEMU Phase-B simulated writer:
#       * CE_SEM_RELEASE / NVC56F SEM_EXECUTE pushbuffer-parser writes (!m2hostsem)
#       * the M5.38 DBG-FORGE at 0xFFF508 (M5.49: now `!m2hostsem`; emits SUPPRESSED log)
# If cup2's CE round-trip still lands byte-exact + util>0 + guest unblocks, completion is
# provably HOST-ONLY = unambiguous first compute.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
QLOG=/tmp/m0_qemu.log

echo "==> build QEMU"
( cd /opt/qemu-src/build && ninja qemu-system-x86_64 && ninja install ) 2>&1 | tail -2 || { echo BUILD_FAIL; exit 1; }

echo "==> fresh restart VM (NVKVM_M2HOSTSEM=1)"
pkill -9 qemu-system 2>/dev/null; sleep 3
nohup env NVKVM_M2HOSTSEM=1 bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep qemu-system >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }

echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }

echo "==> stage cup2 + guest prep"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/tests/mode2/cup2.c /workspace/nvkvm/scripts/mode2_diag/step1_userd_guest.sh \
  ubuntu@localhost:/tmp/ 2>/dev/null
QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)

echo "==> host GPU util sampler (background, 70s)"
( for t in $(seq 1 35); do echo "util_t$((t*2))s: $(nvidia-smi --query-gpu=utilization.gpu,utilization.memory,memory.used --format=csv,noheader 2>/dev/null)"; sleep 2; done ) > /tmp/m549_util.log 2>&1 &

echo "==> run cup2 (full cuCtxCreate->cuMemAlloc->CE round-trip, timeout via step1)"
$SSHG 'bash /tmp/step1_userd_guest.sh' > /tmp/m549_guest.log 2>&1 &
GP=$!; wait $GP 2>/dev/null || true
echo "--- guest cup2 stdout (CTX/MEMALLOC/CE/PASS?) ---"
grep -iE "cuInit|CTX|MEMALLOC|CE |PASS|FAIL|DONE|error|hang|timeout|rv=|want" /tmp/m549_guest.log | head -25

QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/m549_delta.txt

echo ""; echo "============ M5.49 HOST-ONLY-COMPLETION EVIDENCE ============"
echo "--- [A] Phase-B forge SUPPRESSED (proves the forge WAS the live writer) ---"
grep -cE "M5.49: DBG-FORGE SUPPRESSED" /tmp/m549_delta.txt | sed 's/^/  SUPPRESSED count: /'
grep -E "M5.49: DBG-FORGE SUPPRESSED" /tmp/m549_delta.txt | tail -4
echo "--- [B] any Phase-B writer that LEAKED (must be ZERO under m2hostsem) ---"
grep -cE "M5: DBG-FORGE uvm sema|M5: CE_SEM_RELEASE" /tmp/m549_delta.txt | sed 's/^/  leaked-writer count (want 0): /'
echo "--- [C] completion-sema fwd-map active (host GPU writes it) ---"
grep -E "MAPPED \(host GPU writes completion" /tmp/m549_delta.txt | tail -6
echo "--- [D] host doorbell fetch / GP_GET advance (host actually ran) ---"
grep -E "M5.22 RANG|GP_GET .*-> [1-9]|host GP_GET" /tmp/m549_delta.txt | tail -8
echo "--- [E] host GPU util (peak) ---"
grep -oE "[0-9]+ %" /tmp/m549_util.log 2>/dev/null | sort -rn | head -3; tail -3 /tmp/m549_util.log
echo "--- [F] host Xid (want NONE new) ---"; sudo dmesg | grep -iE "Xid" | tail -8
echo "--- [G] GPU health (load / D-state — detect wedge) ---"
uptime | grep -oE "load average.*"; ps -eo stat,comm | grep -c '^D' | sed 's/^/  D-state procs: /'
echo "============ END ============"
