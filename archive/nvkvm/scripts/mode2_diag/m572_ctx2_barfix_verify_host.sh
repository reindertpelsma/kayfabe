#!/usr/bin/env bash
# m572_ctx2_barfix_verify_host.sh — ON HOST. Verify the #12 cont.25 BAR1-aperture fix:
# the forge now ALSO writes the FB page the bUseBar1 CeUtils channel polls via BAR1
# (bar1_pdb walk of the M5.16-captured per-channel ring off + 0x8004). Fresh-boots with
# NVKVM_M2TRACE=1 (forge active), plants kprobe3 (reports CURVAL the guest sees), runs
# cupctx2_min.
#
# SUCCESS: cupctx2_min rc=0; kprobe3 CTX2 block shows bUseBar1=1 CURVAL=84 (was 0);
#          QEMU log "#12 FORGE finishPayload ... barFB=0x..." (non-FAULT) for client 0xc1e00007.
# FAILURE: rc=124, CTX2 CURVAL still 0, barFB=FAULT (ring not captured) -> inspect ring_off.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
D=/workspace/nvkvm
QLOG=/tmp/m0_qemu.log

echo "==> fresh restart VM (NVKVM_M2TRACE=1)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
NVKVM_M2TRACE=1 nohup bash $D/scripts/run_mode2_vm.sh >/tmp/m572_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -8 /tmp/m572_launch.log; exit 1; }

echo "==> wait guest ssh"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; tail -8 /tmp/m572_launch.log; exit 1; }

echo "==> stage cupctx2_min.c"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  $D/tests/mode2/cupctx2_min.c ubuntu@localhost:/tmp/cupctx2_min.c 2>/dev/null && echo "  staged"

echo "==> run kprobe3 verify (CURVAL the guest sees) + cupctx2_min (MIN_TIMEOUT=${MIN_TIMEOUT:-240})"
$SSHG "MIN_TIMEOUT=${MIN_TIMEOUT:-240} bash -s" < $D/scripts/mode2_diag/cupctx2_min_kprobe3_guest.sh 2>&1 | grep -E "cwfp|GT:|rc=|planted|DONE_KPROBE3|CTX|VERDICT|DONE"

echo ""
echo "============ M572 #12 cont.25 BAR1-FIX VERDICT ============"
echo "--- forge writes (want barFB != FAULT for client 0xc1e00007) ---"
grep -E "#12 FORGE finishPayload ch" "$QLOG" 2>/dev/null | grep -E "client=0xc1e000(07|08)" | tail -10
echo "--- ring captured? (M5.16 per-channel ring_off lines) ---"
grep -E "#12 FORGE finishPayload ch" "$QLOG" 2>/dev/null | grep -oE "ring_off=0x[0-9a-f]+ barFB=0x[0-9a-f]+ client=0x[0-9a-f]+" | sort | uniq -c | tail -20
echo "--- any Xid / fault (must be NONE new) ---"
grep -iE "Xid|FAULT_PTE|MMU_FAULT" "$QLOG" 2>/dev/null | tail -5
echo "  (PASS = cupctx2_min rc=0 above + CTX2 GT CURVAL=84)"
echo "============ END ============"
