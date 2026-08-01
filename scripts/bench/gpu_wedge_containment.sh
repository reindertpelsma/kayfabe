#!/bin/bash
# Is one tenant's GPU wedge CONTAINABLE?
#
# guest_blast_radius.md §7 named this "the single most consequential unknown for
# the multi-tenant claim", and named it [unknown] because the escalation logic is
# GSP's and GSP is a signed binary — so it is not readable from the open tree and
# only an experiment can answer it.
#
# §5 states the exposure as: "a non-terminating kernel or a malformed pushbuffer
# submitted through [a real doorbell] is a submission that never returns".  This
# script measures the FIRST of those two shapes.  It does not measure the second.
#
# Run it on a box with an NVIDIA GPU and a driver.  No CUDA toolkit is required:
# the probe drives the CUDA *driver* API out of libcuda.so with hand-written PTX.
#
#   scp gpu_wedge_probe.c gpu_wedge_containment.sh root@BOX:/root/
#   ssh root@BOX 'cd /root && bash gpu_wedge_containment.sh 2>&1 | tee wedge.log'
#
# ★ THE TWO ARMS THAT MAKE A GREEN MEAN SOMETHING, both learned the hard way:
#
#   1. THE CONTROL (`burn`) — a long but TERMINATING kernel.  Without it, "the
#      victim was slow" cannot be told apart from "the victim was denied".
#
#   2. OCCUPANCY — the first version of this experiment span 1 block x 32 threads
#      on a 28-SM GA106, so the victim could simply run on another SM and no
#      preemption was ever exercised.  It reported total containment and COULD
#      NOT HAVE SHOWN OTHERWISE.  `nvidia-smi utilization.gpu` reads 100 % in
#      both cases and so cannot discriminate either.  The wedge must oversubscribe
#      the device: 224 x 1024 = 229 376 threads against ~43 008 resident.
set -u
cd "$(dirname "$0")"
PROBE=./gpu_wedge_probe
[ -x "$PROBE" ] || gcc -O2 -o "$PROBE" gpu_wedge_probe.c -ldl || exit 1

echo "### gpu: $(nvidia-smi --query-gpu=name,driver_version,compute_mode --format=csv,noheader)"
echo "### host: $(uname -r)  $(date -u +%FT%TZ)"
echo "### Xid before: $(dmesg 2>/dev/null | grep -ci xid)"
pkill -9 -x gpu_wedge_probe 2>/dev/null; sleep 2

echo
echo "=== A. BASELINE — idle GPU"
$PROBE victim; echo "   rc=$?"
/usr/bin/time -f "   baseline_realwork_wall=%es" $PROBE burn 500000000; echo "   rc=$?"

echo
echo "=== B. CONTROL — attacker saturates but TERMINATES (must NOT deny service)"
$PROBE burn 4000000000 > /tmp/burn.log 2>&1 &
BP=$!
sleep 4
echo "   util=$(nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader)"
for i in 1 2 3; do $PROBE victim; echo "   rc=$?"; done
wait $BP; echo "   control attacker exit=$?"

echo
echo "=== C. THE EXPERIMENT — attacker NEVER terminates and OVERSUBSCRIBES the device"
$PROBE spin 224 1024 > /tmp/spin.log 2>&1 &
SP=$!
sleep 6
cat /tmp/spin.log
if ! kill -0 "$SP" 2>/dev/null; then echo "   ATTACKER DIED — EXPERIMENT INVALID"; exit 1; fi
echo "   attacker alive, util=$(nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader)"

echo "   -- liveness: trivial victim x3 (60 s hard timeout each)"
for i in 1 2 3; do
    timeout 60 $PROBE victim; RC=$?
    [ "$RC" -eq 124 ] && echo "   rc=$RC <== TIMED OUT, VICTIM DENIED" || echo "   rc=$RC"
done

echo "   -- soak: trivial victim every 5 s for 60 s"
OK=0; FAIL=0
for i in $(seq 1 12); do
    if timeout 60 $PROBE victim > /tmp/v.log 2>&1; then OK=$((OK+1)); else FAIL=$((FAIL+1)); cat /tmp/v.log; fi
    sleep 5
done
echo "   SOAK: ok=$OK fail=$FAIL of 12"

echo "   -- fairness: victim doing REAL work under the wedge (compare to baseline above)"
/usr/bin/time -f "   under_wedge_realwork_wall=%es" timeout 300 $PROBE burn 500000000
echo "   rc=$?"

echo
echo "=== D. AFTERMATH — kill the attacker, is the GPU clean?"
kill -9 "$SP" 2>/dev/null; sleep 8
echo "   util=$(nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader)"
echo "   compute apps: $(nvidia-smi --query-compute-apps=pid,used_memory --format=csv,noheader)"
$PROBE victim; echo "   rc=$?"
echo "   Xid after: $(dmesg 2>/dev/null | grep -ci xid)"
# ⊘ Do not grep for a bare 'reset' here: it matches the boot line "preset value".
echo "   recovery lines: $(dmesg 2>/dev/null | grep -icE 'xid|robust channel|channel recovery|fell off the bus')"
echo "### done $(date -u +%FT%TZ)"
