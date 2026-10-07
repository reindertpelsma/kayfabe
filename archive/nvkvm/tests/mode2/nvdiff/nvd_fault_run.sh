#!/usr/bin/env bash
# nvd_fault_run.sh — drive the DELIBERATE-FAULT arms of the differential.
#
# Runs identically on a real host and inside a Mode-2 guest; that symmetry is the point.
# One instrument, two subjects — and here the subject is "does a GPU fault get REPORTED",
# a question this project has only ever answered from ABSENCE of code.
#
#   ./nvd_fault_run.sh <outdir> [stages]        stages default: "faultce faultgr"
#
# For each stage it produces, in <outdir>:
#   <stage>.stdout      the program's own transcript (FAULT= / VICTIM_STATE / VERDICT)
#   <stage>.jsonl       the full ioctl+mmap trace around the fault  (the MECHANISM)
#   <stage>.dmesg       ONLY the kernel lines that appeared during that stage
#   <stage>.xid         the Xid lines, if any
#   <stage>.byst        a CONCURRENT, SEPARATE-PROCESS benign workload's transcript
#   <stage>.smi         nvidia-smi -q taken after the fault
#   RESULT              one line per stage + a terminator
#
# ★★★ THE TERMINATOR IS LOAD-BEARING. This harness writes START and EXIT lines because
# an absent or empty artefact READS AS FAVOURABLE: a killed job and a running job are
# indistinguishable if absence-of-result is the only check, and `143` (the work was
# SIGTERMed) and `124` (the LAUNCHER timed out while the work ran fine) arrive as the
# same word. If RESULT has no `EXIT ` line, this script did not finish — do not read a
# zero out of anything in this directory.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${1:?usage: nvd_fault_run.sh <outdir> [stages]}"
STAGES="${2:-faultce faultgr}"
BYST_SECS="${BYST_SECS:-25}"
mkdir -p "$OUT"
RES="$OUT/RESULT"

: > "$RES"
say() { echo "$*" | tee -a "$RES"; }

say "START $(date -u +%FT%TZ) host=$(hostname) uid=$(id -u)"
say "ENV driver=$(sed -n 1p /proc/driver/nvidia/version 2>/dev/null || echo NONE)"
say "ENV gpu=$(nvidia-smi --query-gpu=name,pci.bus_id --format=csv,noheader 2>/dev/null || echo NONE)"
say "ENV prog_md5=$(md5sum "$HERE/nvd_prog.c" | cut -d' ' -f1) shim_md5=$(md5sum "$HERE/nvdiff_shim.c" | cut -d' ' -f1)"

# ⚠ dmesg is the ONLY witness for the Xid identity, and a harness that cannot read it
#   must say so LOUDLY rather than record "zero faults". w270 nearly booked a MISSING
#   hostdmesg file as evidence of no faults.
if ! dmesg >/dev/null 2>&1; then
    say "FATAL dmesg is NOT READABLE (uid=$(id -u), kernel.dmesg_restrict=$(sysctl -n kernel.dmesg_restrict 2>/dev/null || echo '?'))"
    say "FATAL ⇒ the Xid identity CANNOT be measured from this account. Not a zero — UNMEASURED."
    say "EXIT rc=3"
    exit 3
fi
say "ENV dmesg_lines_at_start=$(dmesg | wc -l)"

echo "== build (runs=0 is build-only, not an empty capture)"
"$HERE/nvd_capture.sh" "$OUT" ce 0 > "$OUT/build.log" 2>&1 || {
    say "FATAL build failed — see $OUT/build.log"; say "EXIT rc=1"; exit 1; }
say "BUILD ok $(grep -c '^   ok' "$OUT/build.log") symbols gated"

rc_all=0
for st in $STAGES; do
    say "---- stage $st ----"
    before=$(dmesg | wc -l)

    # cross-process bystander, started FIRST so it is demonstrably running when the
    # fault lands, and left running afterwards so it can be observed to survive it.
    ( "$OUT/nvd_prog" bystander "$BYST_SECS" > "$OUT/$st.byst" 2>&1; \
      echo "BYST_EXIT rc=$?" >> "$OUT/$st.byst" ) &
    byst_pid=$!
    sleep 4
    # ⚠ bracket trick NOT needed here (we hold the pid) but liveness IS checked: a
    #   bystander that already died makes its "survived" verdict meaningless.
    if kill -0 "$byst_pid" 2>/dev/null; then say "BYST running pid=$byst_pid"; \
    else say "BYST ⊘ ALREADY DEAD before the fault — its verdict is VOID"; fi

    NVDIFF_OUT="$OUT/$st.jsonl" NVDIFF_MAXBUF=65536 \
        LD_PRELOAD="$OUT/nvdiff_shim.so" \
        timeout "${NVD_TIMEOUT:-180}" "$OUT/nvd_prog" "$st" \
        > "$OUT/$st.stdout" 2>&1
    prc=$?
    say "PROG $st rc=$prc  (⚠ a fault stage is EXPECTED to end in a failing CUDA call; rc is not the verdict)"

    wait "$byst_pid" 2>/dev/null
    nvidia-smi -q > "$OUT/$st.smi" 2>&1

    dmesg | tail -n +$((before + 1)) > "$OUT/$st.dmesg"
    grep -iE 'xid|nvrm' "$OUT/$st.dmesg" > "$OUT/$st.xid" 2>/dev/null
    say "DMESG $st new_lines=$(wc -l < "$OUT/$st.dmesg") xid_lines=$(wc -l < "$OUT/$st.xid")"

    # ASSERT every artefact exists and is non-empty, and say which are legitimately empty.
    for f in "$st.stdout" "$st.jsonl" "$st.byst"; do
        if [ ! -s "$OUT/$f" ]; then say "FATAL artefact EMPTY: $f"; rc_all=1; fi
    done
    if ! grep -q '"nr":42' "$OUT/$st.jsonl" 2>/dev/null; then
        say "FATAL $st.jsonl has no RM_CONTROL — the shim did not attach"; rc_all=1
    fi

    say "VERDICT  $(grep -m1 '^VERDICT'  "$OUT/$st.stdout" || echo 'VERDICT (absent)')"
    say "FAULT    $(grep -m1 '^FAULT '   "$OUT/$st.stdout" || echo 'FAULT (absent)')"
    say "VICTIM   $(grep -m1 '^VICTIM_STATE' "$OUT/$st.stdout" || echo 'VICTIM_STATE (absent)')"
    say "BYST_IN  $(grep -m1 '^BYSTANDER_INPROC' "$OUT/$st.stdout" || echo '(absent)')"
    say "BYST_X   $(grep -m1 '^BYSTANDER_XPROC'  "$OUT/$st.byst"   || echo '(absent)')"
    say "XID      $(grep -m1 -i 'xid' "$OUT/$st.xid" || echo '(no Xid line)')"
done

say "EXIT rc=$rc_all"
exit $rc_all
