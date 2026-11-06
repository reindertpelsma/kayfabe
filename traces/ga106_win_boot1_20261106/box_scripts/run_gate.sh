#!/bin/bash
# run_gate.sh TAG CHECKOUT: windows_gate.sh as the Worker payload calls it (tag, /root/prov), with capture around it
tag=$1 src=$2
E=/root/ga106/$tag
mkdir -p "$E" /workspace/cores
echo "/workspace/cores/core.%e.%p.%s.%t" > /proc/sys/kernel/core_pattern
ulimit -c unlimited
echo "RUN_GATE_START tag=$tag src=$src $(date -u -Is) rev=$(git -C "$src" rev-parse --short=9 HEAD) WINVM_QEMU=${WINVM_QEMU:-default}" > "$E/marker"
{ date -u -Is; free -m; nproc; lscpu | head -20; cat /proc/pressure/memory 2>/dev/null; cat /proc/sys/kernel/core_pattern; ulimit -a; cat /proc/sys/vm/overcommit_memory; systemctl show -p MemoryMax -p MemoryHigh user.slice system.slice 2>/dev/null; } > "$E/host_before.txt" 2>&1
dmesg -T > "$E/dmesg_before.txt" 2>&1
dmesg -wT > "$E/dmesg_follow.txt" 2>&1 & DP=$!
journalctl -f -o short-iso > "$E/journal_follow.txt" 2>&1 & JP=$!
bash /root/ga106/observe.sh "$E/observer.txt" & OP=$!
cd "$src" || exit 9
bash scripts/bench/box/windows_gate.sh "$tag" /root/prov 2>&1 | while IFS= read -r l; do printf '%s %s\n' "$(date -u +%T.%N | cut -c1-12)" "$l"; done > "$E/gate_ts.txt"
rc=${PIPESTATUS[0]}
{ date -u -Is; free -m; } > "$E/host_after.txt" 2>&1
sleep 3
kill $DP $JP $OP 2>/dev/null
ls -la /workspace/cores > "$E/cores.txt" 2>&1
echo "RUN_GATE_EXIT tag=$tag rc=$rc $(date -u -Is)" >> "$E/marker"
