#!/usr/bin/env bash
# m559_cpu_attrib_host.sh — ON HOST. DIAGNOSTIC for the cup5 bulk-copy mechanism
# (no QEMU rebuild assumed beyond the CE-INSTR diag already in-tree). Answers the
# decisive question the byte-attribution left open: WHO physically moves the 64/256 MB
# of cup5's HtoD/DtoH?
#   - guest cup5 process ~100% busy  => libcuda CPU memcpy in the guest (effectively
#       HtoH; the user's hypothesis). Fix is CACHEABILITY (WB), not host-CE forward.
#   - guest cup5 BLOCKED + host QEMU busy + emulated-op count SCALES with MB
#       => QEMU emulates the bulk move (find that path).
#   - host GPU util > 0 => host CE genuinely ran it.
# Uses a LARGER copy (CUP5_MB, default 256) for a multi-second sampling window.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
QLOG=/tmp/m0_qemu.log
MB=${CUP5_MB:-256}

echo "==> fresh restart VM (DEFAULT mode)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }
QEMU_PID=$(pgrep -f qemu-system-x86_64 | head -1)
echo "  qemu pid=$QEMU_PID"
echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }

echo "==> stage + setup (pin adapter)"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/tests/mode2/cup5.c /workspace/nvkvm/tests/mode2/rtp.c \
  /workspace/nvkvm/tests/mode2/reload_probe.c \
  /workspace/nvkvm/scripts/mode2_diag/rtp_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null
$SSHG 'bash /tmp/rtp_run_guest.sh setup' 2>&1 | tail -3

QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)

# ---- guest-side per-second CPU sampler (the DECISIVE signal): top CPU consumers,
#      and cup5's state (R=running/busy, D/S=blocked) ----
echo "==> arm guest CPU sampler (backgrounded in guest)"
$SSHG 'nohup sh -c "for i in \$(seq 1 60); do echo T\$i \$(date +%s.%N); ps -eo pcpu,stat,comm --sort=-pcpu | head -4; sleep 0.5; done" > /tmp/guest_cpu.log 2>&1 &' 2>/dev/null

# ---- host-side samplers: GPU util + QEMU process CPU% (1s) ----
echo "==> arm host samplers (gpu util + qemu cpu)"
( for t in $(seq 1 60); do echo "T${t} $(date +%s.%N) gpu=$(nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader 2>/dev/null) qemucpu=$(ps -o %cpu= -p $QEMU_PID 2>/dev/null)"; sleep 1; done ) > /tmp/m559_host.log 2>&1 &
HSP=$!

echo "==> cup5 CUP5_MB=$MB on pinned adapter (timed)"
$SSHG "CUP5_MB=$MB CUP5_TIMEOUT=180 bash /tmp/rtp_run_guest.sh cup5" 2>&1
kill $HSP 2>/dev/null
QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/m559_delta.txt

echo ""; echo "============ M559 CPU-ATTRIBUTION EVIDENCE (MB=$MB) ============"
echo "--- guest top CPU consumers during run (want: is cup5 R @ ~100%?) ---"
$SSHG 'grep -A3 "^T" /tmp/guest_cpu.log | grep -iE "cup5|T[0-9]" | head -40' 2>/dev/null
echo ""
echo "--- guest: max cup5 %CPU + its stat states seen ---"
$SSHG 'grep cup5 /tmp/guest_cpu.log | awk "{print \$1}" | sort -rn | head -1 | sed "s/^/  max cup5 %cpu: /"; grep cup5 /tmp/guest_cpu.log | awk "{print \$2}" | sort | uniq -c | sed "s/^/  stat: /"' 2>/dev/null
echo ""
echo "--- host: GPU util + qemu %cpu timeline (peaks) ---"
sort -t= -k2 -rn /tmp/m559_host.log 2>/dev/null | grep -oE "gpu=[0-9]+ %" | sort -rn | uniq -c | head -3
echo "  qemu %cpu peak: $(grep -oE "qemucpu=[0-9.]+" /tmp/m559_host.log | sed s/qemucpu=// | sort -rn | head -1)"
echo ""
echo "--- emulated-op SCALING: bytes via CE COPY + DMAW for MB=$MB ---"
echo -n "  CE COPY total MB: "; grep "M5: CE COPY" /tmp/m559_delta.txt | sed -E "s/.*bytes=([0-9]+).*/\1/" | awk "{s+=\$1} END{printf \"%.2f\n\", s/1048576}"
echo -n "  M5.15 DMAW total MB: "; grep "M5.15 DMAW" /tmp/m559_delta.txt | sed -E "s/.*len=([0-9]+).*/\1/" | awk "{s+=\$1} END{printf \"%.2f\n\", s/1048576}"
echo "  (if these stay ~2-3MB while MB=$MB, the bulk path is NOT QEMU emulation)"
echo ""
echo "--- cup5 result line ---"; grep -E "HtoD done|DtoH done|VERIFY" /tmp/m559_delta.txt /dev/null 2>/dev/null
echo "============ END ============"
