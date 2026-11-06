#!/bin/bash
# hold_lock.sh TAG [SECONDS=30]: take the run loop's GPU lock while the gate builds probes, and hold it until
# SECONDS after "GATE_STAGE boot1 START" (the detached run loop waits up to WINVM_LOCK_WAIT_S=120 s for it)
tag=$1 s=${2:-30} log=/root/prov/${1}_windows.log out=/root/ga106/$1/hold_lock.txt
mkdir -p /root/ga106/$1
until grep -q 'GATE_STAGE probes_build START' "$log" 2>/dev/null; do sleep 0.2; done
flock /tmp/kayfabe-fastguest.lock bash -c "echo LOCK_TAKEN \$(date -u +%T.%N); until grep -q 'GATE_STAGE boot1 START' '$log'; do sleep 0.2; done; echo BOOT1_SEEN \$(date -u +%T.%N); sleep $s; echo LOCK_RELEASE \$(date -u +%T.%N)" > "$out" 2>&1
