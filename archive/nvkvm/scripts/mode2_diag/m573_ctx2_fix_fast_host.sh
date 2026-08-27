#!/usr/bin/env bash
# m573_ctx2_fix_fast_host.sh — ON HOST. cont.27: validate the #12 fix at NORMAL SPEED.
# The forge is now DEFAULT-ON (un-gated from m2trace), so we boot WITHOUT m2trace
# (fast — no per-doorbell logging) and run cupctx2_min under kprobe3. With the forge
# active and fast, CTX2's cuCtxCreate should reach cwfp(84) quickly with CURVAL=84
# and RETURN -> cupctx2_min rc=0.
#   SUCCESS: "[CTX2] CTX OK" + "VERDICT: PASS" + rc=0; CTX2 GT CURVAL=84.
#   FAIL:    rc=124 (still hung) or CURVAL<84.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
D=/workspace/nvkvm
TMO=${MIN_TIMEOUT:-90}

echo "==> fresh restart VM (default mode, NO m2trace -> fast; forge default-on)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
nohup bash $D/scripts/run_mode2_vm.sh >/tmp/m573_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -8 /tmp/m573_launch.log; exit 1; }

echo "==> wait guest ssh"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; tail -8 /tmp/m573_launch.log; exit 1; }

echo "==> stage cupctx2_min.c"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  $D/tests/mode2/cupctx2_min.c ubuntu@localhost:/tmp/cupctx2_min.c 2>/dev/null && echo "  staged"

echo "==> run kprobe3 + cupctx2_min (MIN_TIMEOUT=$TMO)"
$SSHG "MIN_TIMEOUT=$TMO bash -s" < $D/scripts/mode2_diag/cupctx2_min_kprobe3_guest.sh 2>&1 \
  | grep -E "CTX|VERDICT|DONE|rc=|GT:|target=84|planted"

echo ""
echo "============ M573 #12 cont.27 FAST VERDICT ============"
echo "  PASS = cupctx2_min rc=0 + [CTX2] CTX OK + CTX2 GT CURVAL=84"
echo "============ END ============"
