#!/usr/bin/env bash
# m555_cudart_payload_host.sh — ON HOST. Localize the cudart-only Mode-2 blocker
# (memory mode2_execfwd_layer2 CORRECTION 3): on a PINNED single-init adapter the
# driver API (dvp) works but cudart (rtp) fails fast at cudaGetDeviceCount with a
# silent in-payload NvStatus rejection. This pass uses the EXISTING per-RPC M4 log
# (nvkvm_m3_service_cmdq, "M4: RPC fn=.. cmd=0x.. status=0x..") — no QEMU change — and
# DIFFs the GSP-RPC stream of the working dvp run vs the failing rtp run on ONE boot.
# Suspects = controls present in rtp but not dvp, and/or any nonzero-status reply in
# the rtp window. If the diff is silent (failing reply is RM-client-side, never a GSP
# RPC), the next pass adds payload-content logging to nvkvm_gpu_emul.c.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
QLOG=/tmp/m0_qemu.log

echo "==> fresh restart VM (DEFAULT mode — NO QEMU change this pass)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }

echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }
echo "==> guest kernel (must be 6.8.0-117):"; $SSHG uname -r 2>/dev/null

echo "==> stage probes + runner"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/tests/mode2/rtp.c /workspace/nvkvm/tests/mode2/reload_probe.c \
  /workspace/nvkvm/scripts/mode2_diag/rtp_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null

echo "==> setup (load modules + PIN adapter + build dvp/rtp)"
$SSHG 'bash /tmp/rtp_run_guest.sh setup' 2>&1 | tail -12

QB1=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
echo "==> run dvp (driver-API, WORKING baseline)"
$SSHG 'bash /tmp/rtp_run_guest.sh dvp' > /tmp/m555_dvp_guest.log 2>&1
sed -n "$((QB1+1)),\$p" "$QLOG" > /tmp/m555_dvp_rpc_all.txt
QB2=$(wc -l < "$QLOG" 2>/dev/null || echo 0)

echo "==> run rtp (cudart, FAILING path)"
$SSHG 'bash /tmp/rtp_run_guest.sh rtp' > /tmp/m555_rtp_guest.log 2>&1
sed -n "$((QB2+1)),\$p" "$QLOG" > /tmp/m555_rtp_rpc_all.txt

echo ""; echo "===================== DVP (driver-API) stdout ====================="
cat /tmp/m555_dvp_guest.log
echo ""; echo "===================== RTP (cudart) stdout ========================="
cat /tmp/m555_rtp_guest.log

# Pull the per-RPC M4 lines for each window.
grep 'M4: RPC' /tmp/m555_dvp_rpc_all.txt > /tmp/m555_dvp_m4.txt 2>/dev/null
grep 'M4: RPC' /tmp/m555_rtp_rpc_all.txt > /tmp/m555_rtp_m4.txt 2>/dev/null
grep 'M4: RPC fn=76' /tmp/m555_dvp_m4.txt | grep -oE 'cmd=0x[0-9a-f]+' | sort -u > /tmp/m555_dvp_ctrls.txt
grep 'M4: RPC fn=76' /tmp/m555_rtp_m4.txt | grep -oE 'cmd=0x[0-9a-f]+' | sort -u > /tmp/m555_rtp_ctrls.txt

echo ""; echo "============ M555 GSP-RPC DIFF (cudart blocker localization) ============"
echo "--- RPC line counts: dvp=$(wc -l < /tmp/m555_dvp_m4.txt) rtp=$(wc -l < /tmp/m555_rtp_m4.txt) ---"
echo ""
echo "--- *** NONZERO-status replies in RTP window (PRIME SUSPECTS) *** ---"
grep -E 'M4: RPC' /tmp/m555_rtp_m4.txt | grep -vE 'status=0x0 rpclen' | head -40
echo "  (rtp nonzero count: $(grep -cE 'M4: RPC' /tmp/m555_rtp_m4.txt | xargs -I{} echo {}; grep -E 'M4: RPC' /tmp/m555_rtp_m4.txt | grep -cvE 'status=0x0 rpclen'))"
echo ""
echo "--- fn=76 control cmds UNIQUE to rtp (in rtp, NOT in dvp) ---"
comm -13 /tmp/m555_dvp_ctrls.txt /tmp/m555_rtp_ctrls.txt
echo ""
echo "--- full rtp fn=76 control list w/ status (for manual scan) ---"
grep 'M4: RPC fn=76' /tmp/m555_rtp_m4.txt | sed -E 's/.*(fn=76 cmd=0x[0-9a-f]+).*(status=0x[0-9a-f]+).*/\1 \2/' | sort | uniq -c
echo ""
echo "--- LAST 15 RPCs in rtp window (what it died right after) ---"
tail -15 /tmp/m555_rtp_m4.txt
echo ""
echo "--- nonzero-status replies in DVP window (baseline noise to subtract) ---"
grep -E 'M4: RPC' /tmp/m555_dvp_m4.txt | grep -vE 'status=0x0 rpclen' | head -20
echo "--- host Xid / health ---"; sudo dmesg | grep -iE "Xid" | tail -3; uptime | grep -oE "load average.*"
echo "============ END (full windows: /tmp/m555_{dvp,rtp}_rpc_all.txt) ============"
