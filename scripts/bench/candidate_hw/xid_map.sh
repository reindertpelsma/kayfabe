#!/bin/bash
# xid_map.sh <evdir> <tag> <since> <until> : Xid lines of the kernel journal in [since, until], each mapped by PID to the QEMU
# the sampler saw (arms=...). dmesg's ring is full here (its Xid count stays at 84 while old lines roll off), so the journal is the counter.
EV=$1; T=$2; S=$3; U=$4
journalctl -k -o short-iso --since "$S" --until "$U" 2>/dev/null | grep -a 'NVRM: Xid' > "$EV/xid_journal_$T.txt"
n=$(wc -l < "$EV/xid_journal_$T.txt"); echo "XID_JOURNAL tag=$T lines=$n"
others=0
while read -r l; do
  pid=$(echo "$l" | grep -o 'pid=[0-9]*' | head -1 | cut -d= -f2); x=$(echo "$l" | grep -o 'Xid (PCI:[^)]*): [0-9]*' | awk '{print $NF}')
  arm=$(awk -v p="$pid" '$1=="QEMU" && $2=="pid="p {f=1; next} f && /arms=/ {print $1" "$2; exit} $1=="QEMU"{f=0}' "$EV/qemus_$T.txt")
  [ -n "$arm" ] || { arm="UNMAPPED(pid $pid)"; others=$((others+1)); }
  echo "  xid=$x pid=$pid -> $arm | $(echo "$l" | grep -o 'faulted @ [^.]*\. Fault is of type [A-Z_ ]*' | cut -c1-90)"
done < "$EV/xid_journal_$T.txt"
echo "XID_UNMAPPED=$others"
