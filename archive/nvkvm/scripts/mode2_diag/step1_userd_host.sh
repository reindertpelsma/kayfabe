#!/usr/bin/env bash
# step1_userd_host.sh — runs ON THE HOST. Rebuild QEMU (picks up the STEP1 probe),
# fresh-restart the VM, run the guest USERD classifier, then dump the QEMU-side
# STEP1 / chan_exec evidence for the compute channel.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 ubuntu@localhost"
QLOG=/tmp/m0_qemu.log

echo "==> build QEMU"
( cd /opt/qemu-src/build && ninja qemu-system-x86_64 && ninja install ) 2>&1 | tail -3 \
    || { echo "BUILD FAILED"; exit 1; }

echo "==> fresh restart VM"
$SSHG 'sync; sync' 2>/dev/null || true
pkill -TERM qemu-system 2>/dev/null; sleep 3
pkill -9 qemu-system 2>/dev/null; sleep 2
# NVKVM_FRESH=0: keep the overlay (guest nvmods/libcuda staging); a plain QEMU
# process restart already resets the emulated-GSP WPR2 state (the only "fresh"
# we need). FRESH=1 would rm the overlay and need qemu-img (not in PATH).
nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3
pgrep qemu-system >/dev/null || { echo "QEMU DIED:"; cat /tmp/m0_launch.log; exit 1; }

echo "==> wait for guest ssh"
up=0
for i in $(seq 1 40); do
    $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up after $((i*5))s"; break; }
    sleep 5
done
[ "$up" = "1" ] || { echo "GUEST DID NOT BOOT"; tail -5 /tmp/m0_launch.log; exit 1; }

echo "==> stage cup2 + guest classifier"
scp -q -P $PORT -o StrictHostKeyChecking=no \
    /workspace/nvkvm/tests/mode2/cup2.c \
    /workspace/nvkvm/scripts/mode2_diag/step1_userd_guest.sh \
    ubuntu@localhost:/tmp/ 2>/dev/null

QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
echo "==> run guest classifier (cup2 bg + mid-hang capture)"
$SSHG 'bash /tmp/step1_userd_guest.sh' 2>&1

echo ""
echo "============ QEMU-SIDE EVIDENCE (this run) ============"
QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/step1_delta.txt
echo "--- USERD-back registrations (ubase/uas/host_qva) ---"
grep -E 'M5.4 USERD-back' /tmp/step1_delta.txt | tail -20
echo "--- STEP1 USERD-WR (BAR1 writes landing in a registered USERD page) ---"
grep -cE 'STEP1 USERD-WR' /tmp/step1_delta.txt | sed 's/^/  total writes: /'
grep -E 'STEP1 USERD-WR' /tmp/step1_delta.txt | grep '0x8c' | tail -30
echo "  (all STEP1 lines, last 30):"
grep -E 'STEP1 USERD-WR' /tmp/step1_delta.txt | tail -30
echo "--- chan_exec gp_put readings (per channel) ---"
grep -E 'M5: chan_exec' /tmp/step1_delta.txt | sort | uniq -c | sort -rn | head -20
echo "--- M5.22 doorbell rings (which clients ring, hostUSERD put) ---"
grep -E 'M5.22 RANG host doorbell' /tmp/step1_delta.txt | sed -E 's/.*(client=0x[0-9a-f]+).*(hostUSERD put=[0-9]+ get=[0-9]+).*/\1 \2/' | sort | uniq -c | sort -rn | head
echo "============ END ============"
