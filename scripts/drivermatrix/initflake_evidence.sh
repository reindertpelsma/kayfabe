#!/usr/bin/env bash
# ★ EVIDENCE FOR ONE ADAPTER-INIT FLAKE (`V3_DRIVER_MATRIX.md` §6): the guest's RmInitAdapter
# failed while kayfabe forwarded its CeUtils work. Prints, from one boot's logs, everything the
# current instrumentation records about the completion that never arrived:
#   - the guest's own assertion chain (NVRM lines: memmgrMemSet / ce_utils / RmInitAdapter),
#   - every translated token's RETIRED line (forwarded / submissions / serves / last GP_PUT / GP_GET),
#   - the device's last channel/interrupt counters before the first retire (host non-stall events
#     observed per engine vs raised to the guest, IRQ writes/raised, CPU views armed/held).
#
#   usage: initflake_evidence.sh <boot tag>        (reads fast_<tag>_{ttyS0,qemu}.log in $BENCH_DIR)
#   or:    for every failed boot of a run:  initflake_evidence.sh --all <tag prefix>
#
# ⊘ What it CANNOT show, and why the hunt needs more than this: the semaphore VALUE the guest polled
# vs the one the host CE wrote. kayfabe does not parse pushbuffers (owner rule), so the completion
# address is not in any log line today — see the triage note in V3_DRIVER_MATRIX.md §6.
set -uo pipefail
BENCH=${BENCH_DIR:-/workspace/bench}
one() {
    local t=$1 g=$BENCH/fast_${1}_ttyS0.log q=$BENCH/fast_${1}_qemu.log
    echo "=== INITFLAKE $t"
    echo "-- guest (NVRM)"
    grep -a -E "memmgrMemSet|ce_utils|memmgrInitCeUtils|RmInitAdapter|kernel_fifo.c" "$g" 2>/dev/null | cut -c1-230
    echo "-- tokens"
    grep -a -E "act birth translated|GPFIFO_SCHEDULE enable=true \(token|RETIRED, forwarded|DOORBELL-LEDGER" "$q" 2>/dev/null | cut -c1-230
    echo "-- counters before the first retire"
    local l; l=$(grep -an "RETIRED, forwarded" "$q" 2>/dev/null | head -1 | cut -d: -f1)
    head -n "${l:-999999}" "$q" 2>/dev/null | grep -a "phase=Running" | tail -1 \
        | grep -oE "nsi=\[[^]]*\]|irq\[[^]]*\]|views\[[^]]*\]|tokens=\[[^]]*\]|rc\[[^]]*\]"
}
if [ "${1:-}" = "--all" ]; then
    for f in $(grep -l "RmInitAdapter failed" "$BENCH"/fast_${2:?prefix}*_ttyS0.log 2>/dev/null); do
        t=$(basename "$f"); t=${t#fast_}; one "${t%_ttyS0.log}"
    done
else
    one "${1:?usage: initflake_evidence.sh <boot tag> | --all <prefix>}"
fi
