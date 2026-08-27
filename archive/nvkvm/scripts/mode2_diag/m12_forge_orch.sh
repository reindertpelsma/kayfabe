#!/usr/bin/env bash
# m12_forge_orch.sh — ON THE HOST (vh). Validate the #12 finishPayload forge:
#   copy deploy src -> build tree, rebuild QEMU, fresh QEMU process (resets the
#   emulated GSP WPR2; keep the overlay so the guest's nvmods/test setup persist),
#   run cupctx2 (2-context #12 repro — expect rc=0, was hanging rc=124), then
#   cup8 (single-context — expect NO regression). Captures the #12 FORGE lines.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 ubuntu@localhost"
QLOG=/tmp/m0_qemu.log

echo "==> [1/7] copy deploy src -> build tree"
cp /workspace/nvkvm/src/qemu/nvkvm_gpu_emul.c /opt/qemu-src/hw/misc/nvkvm_gpu_emul.c
cp /workspace/nvkvm/src/qemu/mode2_regs_ga10x.h /opt/qemu-src/hw/misc/mode2_regs_ga10x.h
echo "  #12 FORGE markers in build src (expect >=1): $(grep -c '#12 FORGE finishPayload' /opt/qemu-src/hw/misc/nvkvm_gpu_emul.c)"

echo "==> [2/7] rebuild QEMU"
( cd /opt/qemu-src/build && ninja qemu-system-x86_64 && ninja install ) 2>&1 | tail -2 \
    || { echo "BUILD FAILED"; exit 1; }

echo "==> [3/7] fresh QEMU process (sync guest, TERM, KILL, relaunch — overlay kept)"
$SSHG 'sync; sync' </dev/null 2>/dev/null || true
pkill -TERM qemu-system 2>/dev/null; sleep 3
pkill -9 qemu-system 2>/dev/null; sleep 2
NVKVM_M2CEFWD=1 NVKVM_M2TRACE=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh \
    >/tmp/m0_launch.log 2>&1 &
sleep 3
pgrep qemu-system >/dev/null || { echo "QEMU DIED:"; cat /tmp/m0_launch.log; exit 1; }

echo "==> [4/7] wait for guest ssh"
up=0
for i in $(seq 1 48); do
    $SSHG echo OK </dev/null 2>/dev/null | grep -q OK && { up=1; echo "  up after $((i*5))s"; break; }
    sleep 5
done
[ "$up" = "1" ] || { echo "GUEST DID NOT BOOT"; tail -8 /tmp/m0_launch.log; exit 1; }

echo "==> [5/7] push cupctx2 + cup8 + harnesses to guest"
scp -q -P $PORT -o StrictHostKeyChecking=no \
    /workspace/nvkvm/tests/mode2/cupctx2.c \
    /workspace/nvkvm/tests/mode2/cup8.c \
    ubuntu@localhost:/tmp/ 2>/dev/null

QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)

echo "==> [6/7] RUN cupctx2 (2-context #12 repro) — expect rc=0 (hang was rc=124)"
CUPCTX2_N=${CUPCTX2_N:-256} CUPCTX2_TIMEOUT=${CUPCTX2_TIMEOUT:-90} \
    $SSHG "CUPCTX2_N=${CUPCTX2_N:-256} CUPCTX2_TIMEOUT=${CUPCTX2_TIMEOUT:-90} bash -s" \
    < /workspace/nvkvm/scripts/mode2_diag/cupctx2_run_guest.sh 2>&1 | tail -40
QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)

echo ""
echo "==> #12 LIFECYCLE overlay-release lines during cupctx2 (de-alias on client free):"
sed -n "$((QB+1)),${QE}p" "$QLOG" | grep -E '#12 LIFECYCLE release' | head -20
echo "  (count: $(sed -n "$((QB+1)),${QE}p" "$QLOG" | grep -c '#12 LIFECYCLE release'))"
echo ""
echo "==> #12 FORGE lines emitted during cupctx2 (the decisive evidence):"
sed -n "$((QB+1)),${QE}p" "$QLOG" | grep -E '#12 FORGE finishPayload' | head -40
echo "  (count: $(sed -n "$((QB+1)),${QE}p" "$QLOG" | grep -c '#12 FORGE finishPayload'))"
echo "==> ceutils/scrub timeout or assert during cupctx2 (expect NONE if fixed):"
sed -n "$((QB+1)),${QE}p" "$QLOG" | grep -iE 'scrub.*time|timed out|ce_utils|finishPayload.*time|RmGsp.*timeout' | head -10 || true

echo ""
echo "==> DONE_CUPCTX2 — (cup8 regression check only runs if you re-invoke with RUN_CUP8=1)"
if [ "${RUN_CUP8:-0}" = "1" ]; then
    echo "==> [7/7] fresh QEMU + RUN cup8 (single-context, no-regression)"
    $SSHG 'sync; sync' </dev/null 2>/dev/null || true
    pkill -TERM qemu-system 2>/dev/null; sleep 3; pkill -9 qemu-system 2>/dev/null; sleep 2
    NVKVM_M2CEFWD=1 NVKVM_M2TRACE=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh \
        >/tmp/m0_launch.log 2>&1 &
    sleep 3
    for i in $(seq 1 48); do $SSHG echo OK </dev/null 2>/dev/null | grep -q OK && break; sleep 5; done
    $SSHG "bash -s" < /workspace/nvkvm/scripts/mode2_diag/cup8_run_guest.sh 2>&1 | tail -25
fi
echo "==> DONE_ORCH"
