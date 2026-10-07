#!/usr/bin/env bash
# m549b_writer_histogram_host.sh — ON HOST. DEFAULT config (m2hostsem OFF = the
# M5.48-PASSING path). Goal: build the completion-WRITER HISTOGRAM and pin each
# writer's timing RELATIVE TO `CRASHWIN ARMED`, so we know which writer completes
# the UVM page-table CE scrubs (uvm_page_table_range_vec_init) and whether that
# happens BEFORE or AFTER the 0xc7c0 GR-obj alloc. This sets the real host-only
# boundary (NOT crashwin, per the M5.49 hang). No code change; pure observation.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
QLOG=/tmp/m0_qemu.log

echo "==> build QEMU"
( cd /opt/qemu-src/build && ninja qemu-system-x86_64 && ninja install ) 2>&1 | tail -2 || { echo BUILD_FAIL; exit 1; }

echo "==> fresh restart VM (DEFAULT config, m2hostsem OFF)"
pkill -9 qemu-system 2>/dev/null; sleep 3
nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep qemu-system >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }

echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }

echo "==> guest kernel (must be 6.8.0-117)"; $SSHG uname -r 2>/dev/null

echo "==> stage cup2 + prep"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/tests/mode2/cup2.c /workspace/nvkvm/scripts/mode2_diag/step1_userd_guest.sh \
  ubuntu@localhost:/tmp/ 2>/dev/null
QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)

echo "==> host GPU util sampler (background, 70s)"
( for t in $(seq 1 35); do echo "util_t$((t*2))s: $(nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader 2>/dev/null)"; sleep 2; done ) > /tmp/m549b_util.log 2>&1 &

echo "==> run cup2 (full; should PASS in default config)"
$SSHG 'bash /tmp/step1_userd_guest.sh' > /tmp/m549b_guest.log 2>&1 &
GP=$!; wait $GP 2>/dev/null || true
# pull the FULL cup2 stdout from the guest (the host snapshot only catches the 45s mark)
$SSHG 'cat /tmp/cup2_out.log' > /tmp/m549b_cup2_full.log 2>/dev/null
echo "--- cup2 FULL stdout (CTX/MEMALLOC/CE/PASS/rv) ---"
grep -iE "cuInit|CTX|MEMALLOC|CE |PASS|FAIL|DONE|rv=|want|error" /tmp/m549b_cup2_full.log | head -30

QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/m549b_delta.txt
D=/tmp/m549b_delta.txt

echo ""; echo "============ WRITER HISTOGRAM (default / passing config) ============"
echo "--- CRASHWIN ARMED at delta line: ---"
CWLINE=$(grep -n "CRASHWIN ARMED" $D | head -1 | cut -d: -f1); echo "  line=${CWLINE:-NONE}"
grep -n "CRASHWIN ARMED" $D | head -1
echo "--- DBG-FORGE writes: total, and split BEFORE/AFTER crashwin ---"
TOT=$(grep -c "M5: DBG-FORGE uvm sema" $D); echo "  total DBG-FORGE: $TOT"
if [ -n "$CWLINE" ]; then
  echo "  BEFORE crashwin: $(grep -n 'M5: DBG-FORGE uvm sema' $D | awk -F: -v c=$CWLINE '$1<c' | wc -l)"
  echo "  AFTER  crashwin: $(grep -n 'M5: DBG-FORGE uvm sema' $D | awk -F: -v c=$CWLINE '$1>=c' | wc -l)"
fi
echo "--- CE_SEM_RELEASE writes: total, and split BEFORE/AFTER crashwin ---"
TOTC=$(grep -c "M5: CE_SEM_RELEASE" $D); echo "  total CE_SEM_RELEASE: $TOTC"
if [ -n "$CWLINE" ]; then
  echo "  BEFORE crashwin: $(grep -n 'M5: CE_SEM_RELEASE' $D | awk -F: -v c=$CWLINE '$1<c' | wc -l)"
  echo "  AFTER  crashwin: $(grep -n 'M5: CE_SEM_RELEASE' $D | awk -F: -v c=$CWLINE '$1>=c' | wc -l)"
fi
echo "--- first 3 DBG-FORGE (GPA/payload) BEFORE crashwin (these complete the early UVM scrubs) ---"
grep "M5: DBG-FORGE uvm sema" $D | head -3
echo "--- guest dmesg: ce-push printk fired? (capped 16) + uvm-ext-pte count ---"
$SSHG 'sudo dmesg | grep -c "NVKVM-DIAG ce-push"; echo "ext-pte:"; sudo dmesg | grep -c "uvm-ext-pte"' 2>/dev/null
echo "--- util peak ---"; grep -oE "[0-9]+ %" /tmp/m549b_util.log | sort -rn | head -3
echo "--- host Xid (want none new) ---"; sudo dmesg | grep -iE "Xid" | tail -4
echo "============ END ============"
