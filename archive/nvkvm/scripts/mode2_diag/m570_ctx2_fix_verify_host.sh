#!/usr/bin/env bash
# m570_ctx2_fix_verify_host.sh — ON HOST. Verify the #12 cont.24 aperture fix:
# the CeUtils finishPayload forge now resolves through the channel's OWN VAS
# (the aperture pbCpuVA reads) instead of the BAR1->FB shortcut. Fresh-boots the
# guest with NVKVM_M2TRACE=1 (the forge is gated behind m2trace), plants the
# register_kprobe on channelWaitForFinishPayload, and runs cupctx2_min
# (create->destroy->create, NO compute).
#
# SUCCESS (the fix works):
#   - cupctx2_min exits rc=0 with "[CTX2] CTX OK" + "VERDICT: PASS"  (the
#     target=84 wait on CTX2's re-created CeUtils channel now RETURNS).
#   - QEMU log shows "#12 FORGE-RESOLVE(VAS) ... SYS" for the CeUtils channel
#     (ideally ",upgraded-from-BAR1") and "#12 FORGE finishPayload ... SYS 0->84".
# FAILURE (unchanged hang): cupctx2_min rc=124, last cwfp entry target=84, and
#   the forge still resolves via BAR1->FB only.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
D=/workspace/nvkvm
QLOG=/tmp/m0_qemu.log

echo "==> fresh restart VM (NVKVM_M2TRACE=1 so the #12 forge is active)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
NVKVM_M2TRACE=1 nohup bash $D/scripts/run_mode2_vm.sh >/tmp/m570_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -8 /tmp/m570_launch.log; exit 1; }

echo "==> wait guest ssh"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; tail -8 /tmp/m570_launch.log; exit 1; }

echo "==> stage cupctx2_min.c + kprobe2 guest script"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  $D/tests/mode2/cupctx2_min.c ubuntu@localhost:/tmp/cupctx2_min.c 2>/dev/null && echo "  staged .c"

echo "==> run kprobe2 verify (build module, plant probe, run cupctx2_min)"
$SSHG 'bash -s' < $D/scripts/mode2_diag/cupctx2_min_kprobe2_guest.sh 2>&1

echo ""
echo "============ M570 #12 cont.24 sem_wr32-ROUTED FORGE VERDICT ============"
echo "--- forge routed through chan_sem_wr32 (payload + redir BAR1 page) ---"
grep -E "#12 FORGE finishPayload\(sem_wr32\)" "$QLOG" 2>/dev/null | tail -10
echo "--- internal resolver decision for the CeUtils finishPayload VA (res=cli_vas/own/translate, phys) ---"
echo "    [c1e00007 = the hanging CTX2 CeUtils; watch which phys + aperture it lands on]"
grep -E "#12-L3c SEMW" "$QLOG" 2>/dev/null | grep -E "client=0xc1e000(07|08)" | tail -16
echo "--- distinct (client -> resolved phys, res) the forge VA landed on ---"
grep -E "#12-L3c SEMW" "$QLOG" 2>/dev/null | grep -oE "phys=0x[0-9a-f]+\([a-z]+\) .*client=0x[0-9a-f]+ .*res=[a-z_]+" | sed -E 's/old=0x[0-9a-f]+ new=0x[0-9a-f]+ //' | sort | uniq -c | tail -25
echo "--- CE-SEM BACKWARD events (collision symptom; want NONE for c1e00007) ---"
grep -E "#12-L3 CE-SEM BACKWARD" "$QLOG" 2>/dev/null | tail -6
echo "--- any Xid / fault (must be NONE new) ---"
grep -iE "Xid|FAULT_PTE|MMU_FAULT" "$QLOG" 2>/dev/null | tail -5
echo "============ END ============"
