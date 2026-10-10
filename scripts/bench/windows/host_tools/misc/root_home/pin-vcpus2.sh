#!/bin/bash
# Pin ONLY the real vCPU threads 1:1. Worker threads spawned by a vCPU inherit
# its comm until they set their own, so name alone is not enough -- the real
# vCPU for N is the LOWEST tid carrying that name (created at VM start).
# Everything else is left free to float across all CPUs; pinning forwarding
# workers to one core each would serialise them.
set -u
QPID=$(pgrep -x -f '/opt/qemu-nvkvm/bin/qemu-system-x86_64.*' | head -1)
[ -n "$QPID" ] || QPID=$(pgrep -f 'qemu-system-x86_64 -name nvkvm' | head -1)
[ -n "$QPID" ] || { echo "no qemu"; exit 1; }
NCPU=$(nproc)
echo "qemu pid $QPID, host has $NCPU cpus"

declare -A best
for t in /proc/$QPID/task/*; do
    tid=$(basename "$t"); c=$(cat "$t/comm" 2>/dev/null) || continue
    n=$(printf '%s' "$c" | sed -n 's|^CPU \([0-9]\+\)/KVM$|\1|p')
    [ -n "$n" ] || continue
    if [ -z "${best[$n]:-}" ] || [ "$tid" -lt "${best[$n]}" ]; then best[$n]=$tid; fi
done

pinned=0
for n in $(printf '%s\n' "${!best[@]}" | sort -n); do
    if taskset -pc "$n" "${best[$n]}" >/dev/null 2>&1; then pinned=$((pinned+1))
    else echo "FAILED vCPU $n"; fi
done

# everything else floats across all cpus
freed=0
for t in /proc/$QPID/task/*; do
    tid=$(basename "$t")
    skip=0; for n in "${!best[@]}"; do [ "$tid" = "${best[$n]}" ] && skip=1; done
    [ "$skip" = 1 ] && continue
    taskset -pc "0-$((NCPU-1))" "$tid" >/dev/null 2>&1 && freed=$((freed+1))
done

echo "pinned $pinned real vCPU threads 1:1; $freed other threads left floating on 0-$((NCPU-1))"
echo "=== verification ==="
for n in $(printf '%s\n' "${!best[@]}" | sort -n); do
    printf "vCPU %-3s tid %-8s -> cpu %s\n" "$n" "${best[$n]}" \
        "$(taskset -pc "${best[$n]}" 2>/dev/null | sed 's/.*list: //')"
done
