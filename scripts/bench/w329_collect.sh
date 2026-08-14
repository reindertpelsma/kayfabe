#!/usr/bin/env bash
# w329 — collect the committable evidence. ⊘ EXCERPTS, and each one says which grep made it.
#
# ⚠ A count taken from an excerpt is a count over the excerpt. The whole QEMU log of a
#   `28,31` boot is hundreds of MB and is NOT committed; what is committed is the lines the
#   rung is graded on.
set -uo pipefail
OUT=${1:?usage: w329_collect.sh <outdir>}
mkdir -p "$OUT"
for f in /workspace/w329_all.log /workspace/w329_followup.log /workspace/w329_offline.log \
         /workspace/w329_arm_*.log; do
  [ -f "$f" ] || continue
  gzip -c "$f" > "$OUT/$(basename "$f").gz"
done
# ★ The device-side excerpt, per tag, with the grep recorded in the file itself.
for Q in /workspace/bench/run_w329*_qemu.log; do
  [ -f "$Q" ] || continue
  T=$(basename "$Q" _qemu.log)
  {
    echo "# extracted from $Q by w329_collect.sh — NOT the whole log."
    echo "# grep: JOIN-RELEASE | SUPERSEDE | revoked= | THE INSTALL REFUSED | GUEST-DESCRIBES | Xid | by_kind="
    echo "#--- the release's own lines (deduped with counts)"
    grep -ao 'revoked=[0-9]* released=[0-9]* stranded=[0-9]* drained=[0-9]* joined_ranges=[0-9]*[^|]*' "$Q" | sort | uniq -c | sed 's/^/  /'
    echo "#--- joined_ranges trajectory (run-length encoded, in order)"
    grep -ao 'joined_ranges=[0-9]*' "$Q" | sed 's/.*=//' | uniq -c | sed 's/^/  /'
    echo "#--- SUPERSEDE lines"
    grep -a 'SUPERSEDED\|SUPERSEDE CAPPED\|SUPERSEDE ABORTED\|TABLE/STORE DISAGREE' "$Q" | sed 's/^/  /' | head -60
    echo "#--- install refusals, va+phys only"
    grep -ao 'leaf va=0x[0-9a-f]* fb_phys=0x[0-9a-f]* → ⚠ THE INSTALL REFUSED' "$Q" | sed 's/ → .*//' | sort | uniq -c | sed 's/^/  /'
    echo "#--- the guest's own described runs, last pass"
    grep -a 'GUEST-DESCRIBES' "$Q" | tail -1 | tr ']' '\n' | sed 's/^/  /'
    echo "#--- by_kind"
    grep -ao 'by_kind={[^}]*}' "$Q" | sort | uniq -c | sed 's/^/  /'
    echo "#--- host_rows (last)"
    grep -ao 'host_rows=[0-9]* of [0-9]*' "$Q" | tail -1 | sed 's/^/  /'
    echo "#--- Xid in this qemu log: $(grep -ac Xid "$Q")"
  } | gzip -c > "$OUT/${T}_qemu_excerpt.log.gz"
done
for P in /workspace/bench/run_w329*_probe.log /workspace/bench/run_w329*_hostdmesg.log; do
  [ -f "$P" ] || continue
  gzip -c "$P" > "$OUT/$(basename "$P").gz"
done
echo "W329_COLLECT_TERMINATOR files=$(ls -1 "$OUT" | wc -l) $(date -Is)"
