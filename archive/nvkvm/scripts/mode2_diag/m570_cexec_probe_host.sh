#!/usr/bin/env bash
# m570_cexec_probe_host.sh — ON HOST. CE-EXEC forward (approach A) PROBE. Boots map-on-touch
# WITH NVKVM_M2CEXEC=1 so the host GPU's CE executes the user-CE channel's LAUNCH_DMA for real
# (channel-forward + ring, CPU byte-copy + CPU sema suppressed for fully-VIRTUAL user-CE copies).
# This re-treads the previously-abandoned host-CE path; the question is whether map-on-touch +
# dirty-tracked residency sweep close the CE-VAS hole that caused the old "Xid 31 CE2 VIRT_WRITE".
# Runs the small Qwen2 LLM. SIGNALS:
#   correctness — coherent generated text + rc=0 (NOT garbage / hang)
#   forward engaged — "exec_doorbell CE" lines appear; ce_emul bucket call-count DROPS vs m569
#   FAIL mode    — new "Xid ... CE2 ... VIRT_WRITE" (residency hole) OR garbage text (sema/data race)
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
QLOG=/tmp/m0_qemu.log
NGEN=${LLM_NGEN:-32}
TIMEOUT=${LLM_TIMEOUT:-240}

echo "==> install QEMU (already built by ninja)"
( cd /opt/qemu-src/build && ninja install ) 2>&1 | tail -1 || { echo INSTALL_FAIL; exit 1; }

XBASE=$(sudo dmesg | grep -c "Xid")          # baseline Xid count (dmesg not cleared across boots)
echo "==> Xid baseline lines: $XBASE"

echo "==> fresh restart VM (NVKVM_M2CEFWD=1 + NVKVM_M2CEXEC=1 — CE-exec forward, approach A)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
NVKVM_M2CEFWD=1 NVKVM_M2CEXEC=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }

echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }
echo "==> guest kernel (must be 6.8.0-117):"; $SSHG uname -r 2>/dev/null

echo "==> stage runner"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/scripts/mode2_diag/llm_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null
QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)

echo "==> host GPU util sampler (background)"
( for t in $(seq 1 $((TIMEOUT/2))); do echo "util_t$((t*2))s: $(nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader 2>/dev/null)"; sleep 2; done ) > /tmp/m570_util.log 2>&1 &

echo "==> run llm to completion (foreground, timeout $((TIMEOUT+20))s)"
$SSHG "LLM_NGEN=$NGEN LLM_TIMEOUT=$TIMEOUT bash /tmp/llm_run_guest.sh" > /tmp/m570_guest.log 2>&1
echo "--- llm exit rc + token rate ---"; grep -iE "exit rc|Generation:|Prompt:" /tmp/m570_guest.log | tail -4
echo "--- generated text (tail; COHERENT = correctness OK) ---"; tail -6 /tmp/m570_guest.log | grep -vE "^\s*$"

QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/m570_delta.txt
D=/tmp/m570_delta.txt

echo ""; echo "============ M570 CE-EXEC PROBE EVIDENCE ============"
echo "--- *** CE channel forwards engaged? (>0 = host CE running guest LAUNCH_DMA) ---"
grep -c "exec_doorbell CE " $D | sed 's/^/  CE exec_doorbell forwards: /'
grep "exec_doorbell CE " $D | tail -3
echo "--- *** TIME-SHARE (ce_emul call-count should DROP vs m569 if copies forwarded) ---"
grep "NVKVM-TIMESHARE" $D | tail -3
echo "--- DPLANE final volumes (ce_bytes_total should drop if host did the copies) ---"; grep "NVKVM-DPLANE SUMMARY" $D | tail -1
echo "--- map-on-touch signals ---"
grep -c "gpga_obj\[gpu_only\]" $D | sed 's/^/  gpu_only objs: /'
grep -c "map-on-touch PROMOTED" $D | sed 's/^/  promoted: /'
grep -c "gpga_obj FAILED" $D | sed 's/^/  gpga FAILED: /'
echo "--- *** NEW host Xid since baseline (CE2 VIRT_WRITE = residency hole = FAIL) ---"
XNOW=$(sudo dmesg | grep -c "Xid"); NEW=$((XNOW - XBASE))
echo "  new Xid lines: $NEW"; [ "$NEW" -gt 0 ] && sudo dmesg | grep "Xid" | tail -n "$NEW"
echo "--- util peak / sustained ---"; grep -oE "[0-9]+ %" /tmp/m570_util.log | sort -rn | head -3; tail -2 /tmp/m570_util.log
echo "--- GPU health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/  D-state: /'
echo "============ END ============"
