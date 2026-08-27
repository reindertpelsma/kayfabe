#!/usr/bin/env bash
# m571_ctx2_groundtruth_host.sh — ON HOST. cont.25 ground truth for #12: WHERE does
# the guest physically read the CeUtils finishPayload it spins on? Fresh-boots the
# guest, plants the kprobe3 (OBJCHANNEL->pChannelBufferMemdesc->_pteArray walk), runs
# cupctx2_min. The CTX2 target=84 block reports finPA + aperture (+ current value if
# SYSMEM). Compare finPA to the emulator forge's 0x102626004 (cont.24) to settle the fix.
# No NVKVM_M2TRACE (guest-side probe is independent of the forge; keeps log light).
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
D=/workspace/nvkvm

echo "==> fresh restart VM (default mode, no m2trace)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
nohup bash $D/scripts/run_mode2_vm.sh >/tmp/m571_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -8 /tmp/m571_launch.log; exit 1; }

echo "==> wait guest ssh"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; tail -8 /tmp/m571_launch.log; exit 1; }

echo "==> stage cupctx2_min.c"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  $D/tests/mode2/cupctx2_min.c ubuntu@localhost:/tmp/cupctx2_min.c 2>/dev/null && echo "  staged"

echo "==> run kprobe3 ground-truth (memdesc->_pteArray walk -> finPA + aperture)"
$SSHG 'bash -s' < $D/scripts/mode2_diag/cupctx2_min_kprobe3_guest.sh 2>&1

echo ""
echo "============ M571 #12 GROUND-TRUTH VERDICT ============"
echo "  cont.24 emulator forge wrote SYS 0x102626004 (reached 84); guest hung."
echo "  -> compare finPA below. If finPA != 0x102626004: emulator resolves the WRONG"
echo "     page (need the memdesc-PA source). If aperture != SYS: must write FB/other."
echo "============ END ============"
