#!/usr/bin/env bash
# dispsw_trace.sh start|stop <out.log> — the HOST-side instrument of the x11-dispsw experiment
# (docs/design/V3_DISPLAY.md): builds and loads kfdsw_probe.ko (kretprobes on host RM's display-SW
# release path — see kfdsw_probe/kfdsw_probe.c for what and why), and on `stop` unloads it and
# writes its kernel-log lines and totals to <out.log>. Run as root on the bench HOST, `start` before
# lane.sh and `stop` after it. Bench-only: the module is built on the box and never leaves it; the
# log is TEXT (GPU VAs, release values and flags host RM was handed — nothing else).
# ⊘ Why not kprobe EVENTS: `[measured 2026-10-03, 6.8.0-59]` trace_kprobe refuses every RM-core
#   function of the NVIDIA module ("Could not probe notrace function") — a module's own kprobes can.
set -uo pipefail
CMD=${1:?start|stop}; OUT=${2:-/workspace/bench/dispsw_trace.log}
HERE="$(cd "$(dirname "$0")" && pwd)"; SRC=${KFDSW_SRC:-$HERE/kfdsw_probe}
B=/root/kfdsw_build
case "$CMD" in
start)
    rmmod kfdsw_probe 2>/dev/null
    rm -rf "$B" && mkdir -p "$B" && cp "$SRC"/kfdsw_probe.c "$SRC"/Makefile "$B"/
    make -C "$B" >/dev/null 2>&1 || { echo "DSW_TRACE_START build=FAIL"; exit 0; }
    dmesg | wc -l > "$B/mark"
    insmod "$B/kfdsw_probe.ko"
    echo "DSW_TRACE_START rc=$? $(dmesg | grep 'kfdsw: loaded' | tail -1 | sed 's/^\[[^]]*\] //') $(date -Is)"
    ;;
stop)
    P=/sys/module/kfdsw_probe/parameters
    tot=""; for f in dsw_calls dsw_addr_valid dsw_sem dsw_ntf dsw_err sem_calls sem_err ntf_calls ntf_err map_found map_null map_kva_null map_kva_set drains drain_client_lookups drain_client_fail drain_dev_lookups drain_dev_fail drain_event_lookups drain_event_fail; do
        tot="$tot $f=$(cat $P/$f 2>/dev/null || echo NA)"; done
    rmmod kfdsw_probe 2>/dev/null
    m=$(cat "$B/mark" 2>/dev/null || echo 0)
    { echo "DSW_TRACE_TOTALS$tot"; dmesg | tail -n +$((m + 1)) | grep -a 'kfdsw:'; } > "$OUT"
    echo "DSW_TRACE_TOTALS$tot"
    echo "DSW_TRACE_MAPS $(grep -a 'kfdsw: map ' "$OUT" | sed 's/.*kfdsw: map //; s/dma=0x[0-9a-f]* //' | sort | uniq -c | sort -rn | head -6 | tr '\n' ';')"
    echo "DSW_TRACE_DRAIN_LOOKUPS $(grep -a -o 'kfdsw: drain [a-z]* 0x[0-9a-f]* -> 0x[0-9a-f]*' "$OUT" | sed 's/kfdsw: drain //' | sort | uniq -c | sort -rn | head -8 | tr '\n' ';')"
    echo "DSW_TRACE_FLAGS $(grep -a 'kfdsw: dsw ' "$OUT" | grep -o 'flags=0x[0-9a-f]*' | sort | uniq -c | tr '\n' ' ')"
    echo "DSW_TRACE_LINES $(grep -ac 'kfdsw:' "$OUT") -> $OUT"
    ;;
*) echo "usage: $0 start|stop <out>" >&2; exit 64 ;;
esac
