#!/bin/bash
# Can one tenant deny the GPU to another by EXHAUSTING VRAM?
#
# §5.1 refuted the hang shape and §5.2 refuted the fault shape — in both, a second
# tenant kept full liveness and correctness.  Both of those rest on the GPU's
# scheduler.  This third shape rests on nothing: it is ordinary allocation, and
# it needs no preemption story at all.  Q19 named it "probably the easier denial
# vector than either of the two shapes above" and left it untested.
#
# ★ The interesting measurements are NOT "does it deny" (it plainly can) but:
#   - how much can an UNPRIVILEGED process take?
#   - does the victim get a CLEAN, NAMED error, or does it hang?
#   - can the victim even CREATE A CONTEXT — a harsher denial than a failed malloc,
#     because it means the tenant cannot get onto the device at all;
#   - does the victim recover the moment the hog exits, with no reset?
#
#   scp gpu_wedge_probe.c gpu_vram_denial.sh root@BOX:/root/
#   ssh root@BOX 'cd /root && bash gpu_vram_denial.sh 2>&1 | tee vram.log'
set -u
cd "$(dirname "$0")"
PROBE=./gpu_wedge_probe
[ -x "$PROBE" ] || gcc -O2 -o "$PROBE" gpu_wedge_probe.c -ldl || exit 1

echo "### gpu: $(nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader)"
echo "### $(date -u +%FT%TZ)"
pkill -9 -x gpu_wedge_probe 2>/dev/null; sleep 2

echo
echo "=== A. BASELINE — victim on an empty GPU"
$PROBE victim; echo "   rc=$?"
echo "   free: $(nvidia-smi --query-gpu=memory.used,memory.free --format=csv,noheader)"

echo
echo "=== B. the hog takes everything it can"
$PROBE hog 45 > /tmp/hog.log 2>&1 &
HP=$!
sleep 12
cat /tmp/hog.log
if ! kill -0 "$HP" 2>/dev/null; then echo "   HOG EXITED EARLY — experiment invalid"; cat /tmp/hog.log; exit 1; fi
echo "   gpu now: $(nvidia-smi --query-gpu=memory.used,memory.free --format=csv,noheader)"

echo
echo "=== C. ★ can a second tenant get on the device at all?"
for i in 1 2; do
    S=$(date +%s.%N)
    timeout 60 $PROBE victim; RC=$?
    E=$(date +%s.%N)
    W=$(echo "$E-$S" | bc)
    case "$RC" in
      0)   echo "   rc=0 wall=${W}s  victim SURVIVED" ;;
      124) echo "   rc=124 wall=${W}s <== TIMED OUT — denial by HANG, the bad kind" ;;
      *)   echo "   rc=$RC wall=${W}s <== DENIED, but with a NAMED error (see above)" ;;
    esac
done

echo
echo "=== D. AFTERMATH — hog releases"
wait $HP 2>/dev/null
sleep 5
echo "   gpu: $(nvidia-smi --query-gpu=memory.used,memory.free --format=csv,noheader)"
echo "   victim again:"; $PROBE victim; echo "   rc=$?"
echo "   Xid total: $(dmesg 2>/dev/null | grep -ci xid)"
echo "### done $(date -u +%FT%TZ)"
