#!/bin/bash
# Pin each guest vCPU thread 1:1 to a host hardware thread, and confine QEMU's
# non-vCPU threads to the two host CPUs left over.
set -u
QPID=$(pgrep -f 'qemu-system-x86_64' | head -1)
[ -n "$QPID" ] || { echo "no qemu process found"; exit 1; }
echo "qemu pid $QPID"
HOST_SPARE="22,23"
pinned=0; other=0
for t in /proc/$QPID/task/*; do
    tid=$(basename "$t")
    comm=$(cat "$t/comm" 2>/dev/null) || continue
    case "$comm" in
        "CPU "*"/KVM")
            n=$(printf '%s' "$comm" | sed -n 's|^CPU \([0-9]\+\)/KVM$|\1|p')
            [ -n "$n" ] || continue
            if taskset -pc "$n" "$tid" >/dev/null 2>&1; then
                pinned=$((pinned+1))
            else
                echo "FAILED to pin vCPU $n (tid $tid)"
            fi
            ;;
        *)
            taskset -pc "$HOST_SPARE" "$tid" >/dev/null 2>&1 && other=$((other+1))
            ;;
    esac
done
echo "pinned $pinned vCPU threads 1:1; confined $other non-vCPU threads to CPUs $HOST_SPARE"
echo "=== verification: vCPU -> allowed host CPU ==="
for t in /proc/$QPID/task/*; do
    comm=$(cat "$t/comm" 2>/dev/null) || continue
    case "$comm" in "CPU "*"/KVM")
        printf "%-14s %s\n" "$comm" "$(taskset -pc "$(basename "$t")" 2>/dev/null | sed 's/.*list: //')" ;;
    esac
done | sort -V
