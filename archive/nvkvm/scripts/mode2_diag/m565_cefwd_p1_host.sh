#!/usr/bin/env bash
# m565_cefwd_p1_host.sh — ON HOST. CE-forward P1 EMPIRICAL CHECK. Boots with NVKVM_M2CEFWD=1
# and the new window/gpu_only code, runs cup7@64MiB, and tests the ORDERING HYPOTHESIS:
# does the doorbell walk (exec_doorbell :2491) CPU-map the user-buffer leaves BEFORE the
# LAUNCH_DMA decode (chan_execute :2641) registers the dst window? If so the window arrives
# too late => NO [gpu_only] backings, gpga FAILED>0, host BAR1 still exhausted, cup7 hangs
# (rc=124) — same wall as m564. If the window somehow wins => [gpu_only]>0, FAILED=0, PASS.
# Metric is the un-forgeable host side: gpga FAILED count + [gpu_only] count + Xid + hang.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
QLOG=/tmp/m0_qemu.log
MB=${CUP7_MB:-64}

echo "==> build QEMU"
( cd /opt/qemu-src/build && ninja qemu-system-x86_64 && ninja install ) 2>&1 | tail -2 || { echo BUILD_FAIL; exit 1; }

echo "==> fresh restart VM (NVKVM_M2CEFWD=1 — CE-forward P1 window/gpu_only code)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
NVKVM_M2CEFWD=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
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

echo "==> host GPU util+mem sampler (background, 130s)"
( for t in $(seq 1 65); do echo "t$((t*2))s: $(nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader 2>/dev/null)"; sleep 2; done ) > /tmp/m565_util.log 2>&1 &

echo "==> run cup7 (foreground, timeout 130s, ${MB} MiB) — rc=124 => HANG (BAR1 wall)"
$SSHG "CUP7_MB=${MB} CUP7_TIMEOUT=120 bash /tmp/cup7_run_guest.sh" > /tmp/m565_guest.log 2>&1
RC=$?
echo "--- cup7 stdout (rc=$RC) ---"
grep -iE "CUP7|VERDICT|RESULT|FIRST bad|FAIL|exit rc" /tmp/m565_guest.log | head -30

QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/m565_delta.txt
D=/tmp/m565_delta.txt
echo ""; echo "============ M565 CE-FWD P1 EVIDENCE ============"
echo "--- cup7 exit rc (124 = HANG = BAR1 wall, same as m564) ---"; echo "  rc=$RC"
echo "--- VERDICT ---"; grep -E "CUP7 VERDICT|CUP7 RESULT|FIRST bad" /tmp/m565_guest.log
echo "--- *** CE-fwd P1 window registered? (KEY: when, relative to walk) ---"
grep -c "CE-fwd P1: user-CE dst window" $D | sed 's/^/  windows registered: /'
grep "CE-fwd P1: user-CE dst window" $D | head -4
echo "--- *** [gpu_only] backings? (KEY: >0 = window won the race; 0 = too late) ---"
grep -c "gpga_obj\[gpu_only\]" $D | sed 's/^/  gpu_only objs: /'
grep "gpga_obj\[gpu_only\]" $D | tail -4
echo "--- M5.60 user-CE dst events (PHYS back / VIRT re-walk) ---"
grep -cE "M5.60 user-CE" $D | sed 's/^/  M5.60 events: /'
grep -E "M5.60 user-CE" $D | tail -4
echo "--- gpga_obj total + FAILURES (FAILED>0 = D2 BAR1 wall still hit) ---"
grep -c "M7 R2 gpga_obj:" $D | sed 's/^/  gpga_obj count: /'
grep -c "gpga_obj FAILED" $D | sed 's/^/  gpga FAILED count: /'
grep "gpga_obj FAILED" $D | tail -3
echo "--- host util/mem peak ---"
grep -oE "[0-9]+ %" /tmp/m565_util.log | sort -rn | head -1
grep -oE "[0-9]+ MiB" /tmp/m565_util.log | sort -rn | head -1
echo "--- host Xid (PER-RUN only; dmesg not cleared across boots — cross-check timestamp) ---"
sudo dmesg | grep -iE "Xid" | tail -3
echo "--- GPU health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/  D-state: /'
echo "============ END ============"
