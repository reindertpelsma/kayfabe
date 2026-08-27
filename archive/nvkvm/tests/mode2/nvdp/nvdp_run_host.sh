#!/bin/bash
# nvdp_run_host.sh -- run the NATIVE data-plane capture on a real GPU host.
#
# NATIVE ONLY. No QEMU, no emulated GPU, no kayfabe. Contaminating this capture
# with our own device defeats its whole purpose, so the script REFUSES to run if
# a guest boot is in flight.
#
# Traps encoded inline, each measured in this tree:
#  - `pgrep -x qemu-system-x86_64` can NEVER match: /proc/PID/comm truncates to
#    15 chars, so the check passes vacuously. Use qemu-system-x86 AND a port check.
#  - `pgrep -f <literal>` ALWAYS matches the asker, because the pattern is in the
#    searching process's own cmdline. Bracket trick where -f is unavoidable.
#  - A start marker and an explicit exit-status terminator are written, so
#    "file exists but has no terminator" is DETECTABLE. A zero-byte output is a
#    state that needs its own check, not "not yet".
#  - Every log is grepped for ENOSPC from the SAME invocation as the status.
set -u

OUT=${OUT:-/workspace/nvdp_out}
SRC=${SRC:-/workspace/nvdp/nvdp.c}
mkdir -p "$OUT"
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
RUN="$OUT/run_$STAMP"
mkdir -p "$RUN"

echo "NVDP_START $(date -u +%FT%TZ) host=$(hostname)" | tee "$RUN/status.txt"

fail() { echo "NVDP_EXIT rc=$1 reason=$2" | tee -a "$RUN/status.txt"; exit "$1"; }

# ---- serial-resource check: do not race a guest boot -------------------------
if pgrep -x qemu-system-x86 >/dev/null 2>&1; then
    fail 90 "qemu-system-x86 is RUNNING -- the GPU is a serial resource"
fi
if ss -tln 2>/dev/null | grep -qE ':(2222|2223)\b'; then
    fail 91 "guest ssh port 2222/2223 is LISTENING -- a guest may be up"
fi
echo "gpu_free: no qemu-system-x86, no 2222/2223 listener" | tee -a "$RUN/status.txt"

nvidia-smi --query-gpu=name,driver_version,pci.bus_id --format=csv,noheader \
    > "$RUN/gpu.txt" 2>&1 || fail 92 "nvidia-smi failed"
cat "$RUN/gpu.txt" | tee -a "$RUN/status.txt"
uname -r > "$RUN/kernel.txt"

# ---- provenance: any bench claim must carry the source revision it ran at ----
{
  echo "src=$SRC"
  echo "src_sha256=$(sha256sum "$SRC" 2>/dev/null | cut -d' ' -f1)"
  echo "gitrev=${NVDP_GITREV:-<not supplied>}"
  echo "cc=$(cc --version 2>/dev/null | head -1)"
} > "$RUN/provenance.txt"
cat "$RUN/provenance.txt" | tee -a "$RUN/status.txt"

cc -O1 -g -o "$RUN/nvdp" "$SRC" -ldl -lpthread 2> "$RUN/build.log" || {
    cat "$RUN/build.log"; fail 93 "build failed"
}

# perf_event_open for HW breakpoints: the negative control needs it live.
PARANOID_WAS=$(cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || echo "?")
echo 1 > /proc/sys/kernel/perf_event_paranoid 2>/dev/null || true
echo "perf_event_paranoid was=$PARANOID_WAS now=$(cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null)" \
    | tee -a "$RUN/status.txt"

export NVDP_OUT="$RUN/nvdp.log"
export NVDP_RAW="$RUN/raw"
mkdir -p "$NVDP_RAW"

echo "NVDP_RUNNING $(date -u +%FT%TZ)" | tee -a "$RUN/status.txt"
timeout 300 "$RUN/nvdp" > "$RUN/stdout.txt" 2> "$RUN/stderr.txt"
RC=$?
echo "$PARANOID_WAS" > /proc/sys/kernel/perf_event_paranoid 2>/dev/null || true

# dmesg is NOT in any serial log; persist it beside the run and assert content.
dmesg 2>/dev/null | tail -200 > "$RUN/dmesg.log"
if grep -qiE 'NVRM|Xid' "$RUN/dmesg.log"; then
    echo "dmesg: NVRM/Xid present ($(grep -ciE 'NVRM|Xid' "$RUN/dmesg.log") lines)" | tee -a "$RUN/status.txt"
else
    echo "dmesg: NO NVRM/Xid lines in the last 200 -- clean run or ring buffer rolled" | tee -a "$RUN/status.txt"
fi

# ENOSPC must be checked from the SAME invocation as the status.
if grep -rl "No space left on device" "$RUN" 2>/dev/null | head -1 | grep -q .; then
    echo "ENOSPC SEEN in this run's logs" | tee -a "$RUN/status.txt"
else
    echo "no ENOSPC in this run's logs" | tee -a "$RUN/status.txt"
fi
df -h / | tail -1 | tee -a "$RUN/status.txt"

# An empty artefact reads as benign. Distinguish "nothing happened" from
# "nothing was recorded" by inspecting content, not existence.
LOGSZ=$(stat -c%s "$NVDP_OUT" 2>/dev/null || echo 0)
echo "nvdp.log bytes=$LOGSZ lines=$(wc -l < "$NVDP_OUT" 2>/dev/null || echo 0)" | tee -a "$RUN/status.txt"
if [ "$LOGSZ" -eq 0 ]; then
    echo "ZERO-BYTE LOG -- this is a RESULT (nothing recorded), not 'not yet'" | tee -a "$RUN/status.txt"
fi
grep -c . "$NVDP_OUT" >/dev/null 2>&1
tail -3 "$NVDP_OUT" 2>/dev/null | tee -a "$RUN/status.txt"

echo "NVDP_EXIT rc=$RC run=$RUN $(date -u +%FT%TZ)" | tee -a "$RUN/status.txt"
echo "$RUN"
exit $RC
