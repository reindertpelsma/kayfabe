#!/usr/bin/env bash
# m567_maptouch_host.sh — ON HOST. Map-on-touch CE-forward verification (supersedes m565's
# window probe). Boots with NVKVM_M2CEFWD=1 and the map-on-touch code, runs cup8@64MiB, and
# tests the D2 fix: leaf_flush now requests gpu_only by DEFAULT for compute-client vidmem
# leaves; gpga_obj_ex keeps a leaf gpu_only only if BLANK at walk time (real host vidmem +
# GPU-side map_dma, ZERO host BAR1), else eager-maps+copies; a blank gpu_only leaf promotes
# to a CPU view on the first guest touch. Expected on the WIN: gpu_only objs > 0 (gate
# engaged, off-BAR1 dst), gpga FAILED = 0 (no BAR1 wall), no Xid, no hang, cup8 VERDICT
# PASS=D1 byte-exact. PROMOTED appears iff the guest CPU directly touches a gpu_only leaf.
# Metric is the un-forgeable host side: gpu_only / PROMOTED / GIVE-UP / gpga FAILED / Xid.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
QLOG=/tmp/m0_qemu.log
MB=${CUP8_N:-2048}

echo "==> build QEMU"
( cd /opt/qemu-src/build && ninja qemu-system-x86_64 && ninja install ) 2>&1 | tail -2 || { echo BUILD_FAIL; exit 1; }

echo "==> fresh restart VM (NVKVM_M2CEFWD=1 — map-on-touch code)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
NVKVM_M2CEFWD=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }

echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }
echo "==> guest kernel (must be 6.8.0-117):"; $SSHG uname -r 2>/dev/null

echo "==> stage cup8 + runner"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/tests/mode2/cup8.c /workspace/nvkvm/scripts/mode2_diag/cup8_run_guest.sh \
  ubuntu@localhost:/tmp/ 2>/dev/null
QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)

echo "==> host GPU util+mem sampler (background, 130s)"
( for t in $(seq 1 65); do echo "t$((t*2))s: $(nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader 2>/dev/null)"; sleep 2; done ) > /tmp/m567_util.log 2>&1 &

echo "==> run cup8 (foreground, timeout 130s, N=${MB}) — rc=124 => HANG (BAR1 wall)"
$SSHG "CUP8_N=${MB} CUP8_TIMEOUT=120 bash /tmp/cup8_run_guest.sh" > /tmp/m567_guest.log 2>&1
# NOTE: this RC is ssh's, NOT cup8's; trust the inline 'exit rc=NNN' in the cup8 stdout below.
echo "--- cup8 stdout ---"
grep -iE "CUP8|VERDICT|RESULT|FIRST bad|FAIL|exit rc|CTX OK|SYNC OK|DONE" /tmp/m567_guest.log | head -30

QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/m567_delta.txt
D=/tmp/m567_delta.txt
echo ""; echo "============ M566 MAP-ON-TOUCH EVIDENCE ============"
echo "--- cup8 VERDICT (PASS=D1 byte-exact = THE WIN) ---"
grep -E "CUP8 VERDICT|CUP8 RESULT|FIRST bad|exit rc" /tmp/m567_guest.log
echo "--- *** gpu_only backings? (KEY: >0 = map-on-touch gate engaged, dst off host BAR1) ---"
grep -c "gpga_obj\[gpu_only\]" $D | sed 's/^/  gpu_only objs: /'
grep "gpga_obj\[gpu_only\]" $D | tail -4
echo "--- *** PROMOTED (guest CPU touched a gpu_only leaf -> lazy CPU map) ---"
grep -c "map-on-touch PROMOTED" $D | sed 's/^/  promoted: /'
grep "map-on-touch PROMOTED" $D | tail -4
echo "--- GIVE-UP (RM_MAP_MEMORY failed at touch = host BAR1 full even lazily) ---"
grep -c "map-on-touch GIVE-UP" $D | sed 's/^/  give-up: /'
grep "map-on-touch GIVE-UP" $D | tail -3
echo "--- M5.60 user-CE dst events (PHYS back / VIRT re-walk) ---"
grep -cE "M5.60 user-CE" $D | sed 's/^/  M5.60 events: /'
grep -E "M5.60 user-CE" $D | tail -3
echo "--- gpga_obj total + FAILURES (FAILED>0 = D2 BAR1 wall still hit) ---"
grep -c "M7 R2 gpga_obj:" $D | sed 's/^/  gpga_obj (cpu-mapped) count: /'
grep -c "gpga_obj\[gpu_only\]" $D | sed 's/^/  gpga_obj (gpu_only) count: /'
grep -c "gpga_obj FAILED" $D | sed 's/^/  gpga FAILED count: /'
grep "gpga_obj FAILED" $D | tail -3
echo "--- host util/mem peak ---"
grep -oE "[0-9]+ %" /tmp/m567_util.log | sort -rn | head -1
grep -oE "[0-9]+ MiB" /tmp/m567_util.log | sort -rn | head -1
echo "--- host Xid (PER-RUN only; dmesg not cleared across boots — cross-check timestamp) ---"
sudo dmesg | grep -iE "Xid" | tail -3
echo "--- GPU health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/  D-state: /'
echo "============ END ============"
