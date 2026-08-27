#!/usr/bin/env bash
# m556_cudart_fix_host.sh — ON HOST. Verify the 0x20809009 cudart-init-gate fix
# (replay host reply {0,13} instead of CTRL-UNFILLED {0,0}). Build QEMU, fresh boot,
# PIN the adapter (single init), run the cudart probe (rtp). PASS = cudaGetDeviceCount
# rc=0 n=1 (was rc=3 cudaErrorInitializationError). Also dumps the next CTRL-UNFILLED
# gate if cudart now advances further. DEFAULT mode (must not regress cup4).
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

echo "==> stage probes + runner"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/tests/mode2/rtp.c /workspace/nvkvm/tests/mode2/reload_probe.c \
  /workspace/nvkvm/scripts/mode2_diag/rtp_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null

echo "==> setup (load modules + PIN adapter + build probes)"
$SSHG 'bash /tmp/rtp_run_guest.sh setup' 2>&1 | tail -6

QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
echo "==> run rtp (cudart) — the fix target"
$SSHG 'bash /tmp/rtp_run_guest.sh rtp' 2>&1
QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/m556_rtp_delta.txt
D=/tmp/m556_rtp_delta.txt

echo ""; echo "============ M556 cudart-fix EVIDENCE ============"
echo "--- 0x20809009 handling (want: NO CTRL-UNFILLED for it) ---"
grep -E "20809009|CTRL-UNFILLED" $D | head -20
echo "--- any REMAINING CTRL-UNFILLED gates (next blocker if cudart still fails) ---"
grep -E "CTRL-UNFILLED" $D | grep -oE "cmd=0x[0-9a-f]+ psize=[0-9]+" | sort | uniq -c
echo "--- last 12 fn=76 controls in rtp window ---"
grep "M4: RPC fn=76" $D | tail -12 | sed -E 's/nvkvm-gpu\[GA106\] M4: RPC //'
echo "--- host Xid / health ---"; sudo dmesg | grep -iE "Xid" | tail -3; uptime | grep -oE "load average.*"
echo "============ END ============"
