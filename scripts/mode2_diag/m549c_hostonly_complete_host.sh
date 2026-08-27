#!/usr/bin/env bash
# m549c_hostonly_complete_host.sh — ON HOST. NARROW host-only-completion proof.
# m2hostsem=on + M5.49b per-client gate: ONLY the compute client's (m2_gr_client)
# CE completion is forced host-written; UVM/init scrubs keep simulated completion
# (DBG-FORGE + non-gr CE_SEM_RELEASE stay live). Runs cup2 TO COMPLETION (foreground,
# bounded by timeout — no mid-flight kill). PASS here = the host GPU wrote the user
# CE round-trip's completion sema (rv byte-exact), kernel plumbing still simulated.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
QLOG=/tmp/m0_qemu.log

echo "==> build QEMU"
( cd /opt/qemu-src/build && ninja qemu-system-x86_64 && ninja install ) 2>&1 | tail -2 || { echo BUILD_FAIL; exit 1; }

echo "==> fresh restart VM (NVKVM_M2HOSTSEM=1)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
nohup env NVKVM_M2HOSTSEM=1 bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }

echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }
echo "==> guest kernel (must be 6.8.0-117):"; $SSHG uname -r 2>/dev/null

echo "==> stage cup2 + foreground runner"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/tests/mode2/cup2.c /workspace/nvkvm/scripts/mode2_diag/cup2_run_guest.sh \
  ubuntu@localhost:/tmp/ 2>/dev/null
QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)

echo "==> host GPU util sampler (background, 90s)"
( for t in $(seq 1 45); do echo "util_t$((t*2))s: $(nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader 2>/dev/null)"; sleep 2; done ) > /tmp/m549c_util.log 2>&1 &

echo "==> run cup2 to completion (foreground, timeout 120s)"
$SSHG 'CUP2_TIMEOUT=120 bash /tmp/cup2_run_guest.sh' > /tmp/m549c_guest.log 2>&1
echo "--- cup2 stdout (CTX/MEMALLOC/CE/PASS/rv) ---"
grep -iE "cuInit|cuCtx|MEMALLOC|CE rv|PASS|MISMATCH|FAIL|error|exit rc" /tmp/m549c_guest.log | head -30

QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/m549c_delta.txt
D=/tmp/m549c_delta.txt

echo ""; echo "============ M5.49b NARROW HOST-ONLY EVIDENCE ============"
echo "--- [A0] USER-CE clients recorded (libcuda CE-copy clients) ---"
grep -E "M5.49b USER-CE client" $D
echo "--- [A] user-CE completions SUPPRESSED in sim (host must write) ---"
echo "  host-only sema writes: $(grep -c 'M5.49b host-only sema' $D)"
grep -E "M5.49b host-only sema" $D | tail -5
echo "--- [B] compute-client completion-sema fwd-map (host writes it) ---"
grep -E "M5.19 fwd-map sema .* MAPPED \(host GPU writes" $D | tail -5
grep -c "MAPPED (host GPU writes completion" $D | sed 's/^/  host-writes-completion maps: /'
echo "--- [C] kernel plumbing STILL simulated (must be nonzero = init/UVM intact) ---"
echo "  DBG-FORGE (UVM): $(grep -c 'M5: DBG-FORGE uvm sema' $D)"
echo "  CE_SEM_RELEASE (non-gr): $(grep -c 'M5: CE_SEM_RELEASE' $D)"
echo "--- [D] CRASHWIN ARMED + gr client ---"; grep 'CRASHWIN ARMED' $D | head -1
echo "--- [E] util peak ---"; grep -oE "[0-9]+ %" /tmp/m549c_util.log | sort -rn | head -3; tail -2 /tmp/m549c_util.log
echo "--- [F] host Xid (want NONE new) ---"; sudo dmesg | grep -iE "Xid" | tail -5
echo "--- [G] GPU health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/  D-state: /'
echo "============ END ============"
