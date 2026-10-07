#!/bin/bash
# Is one tenant's GPU FAULT containable?
#
# guest_blast_radius.md §5.1 refuted the *hang* shape: a non-terminating kernel
# does not deny the GPU to another tenant.  But it explicitly did NOT establish
# the other half, and said why:
#
#   "zero Xid means recovery was never ASKED, not that recovery is contained"
#
# §5 names two shapes — "a non-terminating kernel OR a malformed pushbuffer".
# Only the second can FAULT, and only a fault reaches RM's channel-recovery and
# the three escalation hazards in §7 (whole-runlist preempt, node-level
# reboot-required latch, GSP-death halting every channel GPU-wide).
#
# This measures the fault shape at the level a guest can actually reach through
# us: a kernel storing through a wild, unmapped device VA.  ⊘ It is NOT a
# malformed pushbuffer — the pushbuffer itself is well-formed and RM built it.
# What it shares with that shape, and what makes it worth running, is that it
# produces a REAL MMU fault and a REAL Xid, so the escalation path is entered.
#
#   scp gpu_wedge_probe.c gpu_fault_containment.sh root@BOX:/root/
#   ssh root@BOX 'cd /root && bash gpu_fault_containment.sh 2>&1 | tee fault.log'
#
# ★ THE ARM THAT MATTERS is C: the victim HOLDS A LIVE CONTEXT across the
# attacker's fault.  A victim that starts a fresh process afterwards would get a
# fresh context and would therefore survive a context-scoped kill without ever
# noticing it — the same "instrument that could not have shown otherwise" trap
# that made the first version of the wedge experiment worthless.
set -u
cd "$(dirname "$0")"
PROBE=./gpu_wedge_probe
[ -x "$PROBE" ] || gcc -O2 -o "$PROBE" gpu_wedge_probe.c -ldl || exit 1

xid() { dmesg 2>/dev/null | grep -ci xid; }
echo "### gpu: $(nvidia-smi --query-gpu=name,driver_version --format=csv,noheader)"
echo "### $(date -u +%FT%TZ)  Xid before: $(xid)"
pkill -9 -x gpu_wedge_probe 2>/dev/null; sleep 2

echo
echo "=== A. BASELINE — victim on an idle GPU"
$PROBE victim; echo "   rc=$?"

echo
echo "=== B. the attacker FAULTS, alone"
$PROBE fault; echo "   attacker rc=$?"
sleep 5
echo "   Xid after the fault: $(xid)"
dmesg 2>/dev/null | grep -iE "xid|MMU fault|channel|robust" | tail -6

echo
echo "=== B2. a FRESH victim after the fault (weak arm — fresh context)"
$PROBE victim; echo "   rc=$?"

echo
echo "=== C. ★ THE ARM THAT MATTERS — victim holds a LIVE context across the fault"
$PROBE loop 40 > /tmp/loop.log 2>&1 &
LP=$!
sleep 8
if ! kill -0 "$LP" 2>/dev/null; then echo "   VICTIM DIED BEFORE THE FAULT — experiment invalid"; cat /tmp/loop.log; exit 1; fi
echo "   victim is live and looping; faulting now"
$PROBE fault; echo "   attacker rc=$?"
wait $LP; VRC=$?
echo "   --- victim log ---"
cat /tmp/loop.log
echo "   victim exit=$VRC   (0=all correct, 3=wrong bytes, 4=context died)"

echo
echo "=== D. AFTERMATH"
echo "   Xid total: $(xid)"
echo "   util: $(nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader)"
echo "   a brand-new victim:"; $PROBE victim; echo "   rc=$?"
dmesg 2>/dev/null | grep -iE "xid|reboot|fell off the bus|GPU has fallen" | tail -8
echo "### done $(date -u +%FT%TZ)"
