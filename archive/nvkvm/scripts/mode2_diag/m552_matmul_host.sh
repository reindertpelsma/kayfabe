#!/usr/bin/env bash
# m552_matmul_host.sh — ON HOST. North-star step 1: GR KERNEL LAUNCH (cup4).
# DEFAULT mode (NOT m2hostsem). Fresh boot, run cup4 (cuLaunchKernel out=in*3+1) to
# completion. PASS here = host GR engine genuinely ran the shader (un-forgeable; the CE
# emulator can't do arithmetic). Captures where the launch blocks if it hangs.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
QLOG=/tmp/m0_qemu.log

echo "==> build QEMU"
( cd /opt/qemu-src/build && ninja qemu-system-x86_64 && ninja install ) 2>&1 | tail -2 || { echo BUILD_FAIL; exit 1; }

echo "==> fresh restart VM (DEFAULT mode)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }

echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }
echo "==> guest kernel (must be 6.8.0-117):"; $SSHG uname -r 2>/dev/null

echo "==> stage cup4 + runner"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/tests/mode2/cup4.c /workspace/nvkvm/scripts/mode2_diag/cup4_run_guest.sh \
  ubuntu@localhost:/tmp/ 2>/dev/null
QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)

echo "==> host GPU util sampler (background, 90s)"
( for t in $(seq 1 45); do echo "util_t$((t*2))s: $(nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader 2>/dev/null)"; sleep 2; done ) > /tmp/m552_util.log 2>&1 &

echo "==> run cup4 to completion (foreground, timeout 120s)"
$SSHG 'CUP4_TIMEOUT=120 bash /tmp/cup4_run_guest.sh' > /tmp/m552_guest.log 2>&1
echo "--- cup4 stdout ---"
grep -iE "ptxjit|CTX|MODULE|FUNC|MEMALLOC|LAUNCH|SYNC|MATMUL|MEMALLOC|KERNEL|PASS|MISMATCH|FAIL|exit rc" /tmp/m552_guest.log | head -40

QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/m552_delta.txt
D=/tmp/m552_delta.txt
echo ""; echo "============ M552 KERNEL-LAUNCH EVIDENCE ============"
echo "--- util peak ---"; grep -oE "[0-9]+ %" /tmp/m552_util.log | sort -rn | head -3; tail -2 /tmp/m552_util.log
echo "--- host Xid (want NONE new) ---"; sudo dmesg | grep -iE "Xid" | tail -5
echo "--- new fault VAs / faults in QEMU log ---"; grep -nE "FAULT|fault|st=0x57|backed=0|enum_gr_sysmem" $D | tail -15
echo "--- compute report-sema (NVC7C0) releases (kernel completion) ---"; grep -cE "cr_sem|SET_REPORT_SEMAPHORE|1b0c" $D | sed 's/^/  cr_sem hits: /'
echo "--- GPU health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/  D-state: /'
echo "============ END ============"
