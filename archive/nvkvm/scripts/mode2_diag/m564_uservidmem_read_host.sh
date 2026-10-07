#!/usr/bin/env bash
# m564_uservidmem_read_host.sh — ON HOST. The D1/D2 discriminator boot. DEFAULT mode (NO
# m2cefwd / NO m2hostsem) — this tests the EXISTING walk-driven gpga backing (the working
# milestone 90271e8), NOT Phase A. Fresh boot, run cup7 (host GR READ of a large user buffer,
# default 64 MiB), byte-verify, and tie the result to whether cup7's buffer leaves got
# gpga_obj-backed. PASS => D1 (correctness holds at scale). read0/garbage => D2 (named leaf).
# Env: CUP7_MB (default 64).
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
QLOG=/tmp/m0_qemu.log
MB=${CUP7_MB:-64}

echo "==> build QEMU"
( cd /opt/qemu-src/build && ninja qemu-system-x86_64 && ninja install ) 2>&1 | tail -2 || { echo BUILD_FAIL; exit 1; }

echo "==> fresh restart VM (DEFAULT mode — testing existing backing, not Phase A)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }

echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }
echo "==> guest kernel (must be 6.8.0-117):"; $SSHG uname -r 2>/dev/null

echo "==> stage cup7 + runner"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/tests/mode2/cup7.c /workspace/nvkvm/scripts/mode2_diag/cup7_run_guest.sh \
  ubuntu@localhost:/tmp/ 2>/dev/null
QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)

echo "==> host GPU util+mem sampler (background, 120s) — D1 vs D2 cross-check"
( for t in $(seq 1 60); do echo "t$((t*2))s: $(nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader 2>/dev/null)"; sleep 2; done ) > /tmp/m564_util.log 2>&1 &

echo "==> run cup7 to completion (foreground, timeout 130s, ${MB} MiB)"
$SSHG "CUP7_MB=${MB} CUP7_TIMEOUT=120 bash /tmp/cup7_run_guest.sh" > /tmp/m564_guest.log 2>&1
echo "--- cup7 stdout ---"
grep -iE "CUP7|CTX|MODULE|FUNC|MEMALLOC|LAUNCH|SYNC|RESULT|VERDICT|FIRST bad|FAIL|exit rc|ptxjit" /tmp/m564_guest.log | head -40

QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/m564_delta.txt
D=/tmp/m564_delta.txt
echo ""; echo "============ M564 D1/D2 EVIDENCE ============"
echo "--- VERDICT ---"; grep -E "CUP7 VERDICT|CUP7 RESULT|FIRST bad" /tmp/m564_guest.log
echo "--- host util/mem peak (D1: mem flat+util>0 on read; D2 may fault) ---"
grep -oE "[0-9]+ %" /tmp/m564_util.log | sort -rn | head -1
grep -oE "[0-9]+ MiB" /tmp/m564_util.log | sort -rn | head -1
echo "--- gpga_obj backings of user buffer leaves (M7 R2) — count + last few ---"
grep -c "M7 R2 gpga_obj:" $D | sed 's/^/  gpga_obj count: /'
grep "M7 R2 gpga_obj:" $D | tail -4
echo "--- gpga_obj FAILURES (M5.51 — un-backable leaves => D2 root) ---"
grep -c "gpga_obj FAILED" $D | sed 's/^/  gpga FAILED count: /'
grep "gpga_obj FAILED" $D | tail -4
echo "--- doorbell re-sweeps (M5.10/M5.48c) ---"; grep -c "doorbell re-sweep" $D | sed 's/^/  sweeps: /'
echo "--- host Xid / FAULT_PTE (want NONE) ---"; sudo dmesg | grep -iE "Xid" | tail -4; grep -iE "FAULT_PTE|FAULT" $D | tail -4
echo "--- GPU health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/  D-state: /'
echo "============ END ============"
