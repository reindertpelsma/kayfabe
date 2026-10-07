#!/usr/bin/env bash
# w269 GRADER — score the two arms against `docs/design/w269_the_spin_address_prereg.md` §3,
# which was committed at `92aa6da`, BEFORE `guest_spinprobe.c` existed.
#
# ⊘ Every row prints what it MATCHED, not just a verdict, because a grader that prints only
#   PASS/FAIL cannot be told from one whose pattern never matches anything.
# ⚠ `Xid` and the polled ADDRESS are graded as IDENTITY, never as a count (`w265`'s lesson:
#   a count cannot see a substitution).
set -uo pipefail
B=${BENCH_DIR:-/workspace/bench}
say() { printf '%s\n' "$*"; }
row() { printf '  %-6s %-58s %s\n' "$1" "$2" "$3"; }

for arm in refuse pass; do
  P="$B/run_w269_${arm}_probe.log"
  Q="$B/run_w269_${arm}_qemu.log"
  H="$B/run_w269_${arm}_hostdmesg.log"
  say ""
  say "================ ARM: w269_$arm ================"
  if [ ! -s "$P" ]; then
    say "  ★★★ $P is MISSING OR ZERO BYTES. ⊘ That is not 'nothing happened' — it is"
    say "      'nothing was recorded', and the two are different facts. This arm is VOID."
    continue
  fi

  # --- the arming, out of the device's own log -------------------------------------------
  want=refuse; [ "$arm" = pass ] && want=passthrough
  row ARM "GR-ROUTE arm=$want" "$(grep -c "GR-ROUTE arm=$want" "$Q" 2>/dev/null)"

  # --- P1: CUP2_RC and its SIZE ----------------------------------------------------------
  rc=$(grep -o 'CUP2_RC=[A-Z0-9_]*' "$P" | tail -1)
  by=$(grep -o 'CUP2_OUT_BYTES=[0-9]*' "$P" | tail -1)
  last=$(grep -E '^(ok   |devices=|name=|compute=|totalMem=|CTX OK|MEMALLOC|CE rv=|DONE|FAIL)' "$P" | tail -1)
  row P1 "CUP2_RC (predicted 124, p=.93)" "${rc:-ABSENT}  ${by:-NO_BYTE_COUNT}"
  row P1 "last line cup2 printed (predicted totalMem=)" "${last:-ABSENT}"
  row P1 "★ any 'ok   cuCtxCreate' at all?" "$(grep -c 'ok   cuCtxCreate' "$P" 2>/dev/null)"

  # --- P2: is this a memory poll AT ALL? (the owner's item 4) ----------------------------
  say "  P2   thread state / syscall — ⊘ if state is S/D with orig_rax>=0 the whole"
  say "       memory-poll reading is VOID and items 2-3 do not apply:"
  grep -E '^    tid [0-9]+ +state=' "$P" | sed 's/^/       /' | head -12
  grep -E '^    [0-9]+: ' "$P" | sed 's/^/       /' | head -12

  # --- P3/P4: the loop, and whether the snapshot fired -----------------------------------
  row P3 "steps actually taken" "$(grep -o 'steps actually taken = [0-9]*' "$P" | tail -1)"
  say "  P3   RIP histogram, top rows (predicted: mass in libcuda+0x22bd80..0x22c150"
  say "       and +0xf9df90..+0xf9e380 — the loop w215 named):"
  grep -A12 'RIP HISTOGRAM' "$P" | grep -E '^ +[0-9]+ +0x' | head -10 | sed 's/^/     /'
  row P4 "SNAPSHOT fired? (predicted yes, p=.85)" \
      "$(grep -c 'SNAPSHOT at RIP' "$P" 2>/dev/null) fired / $(grep -c 'NO SNAPSHOT' "$P" 2>/dev/null) absent"

  # --- P5/P6: the wait-item array --------------------------------------------------------
  row P5 "N items (predicted 1 or 2, p=.70)" "$(grep -o 'N items = [0-9]*' "$P" | sort -u | tr '\n' ' ')"
  row P6 "KINDs seen (predicted a 3 present, p=.60)" "$(grep -o 'KIND=[0-9]*' "$P" | sort | uniq -c | tr '\n' ' ')"

  # --- P7/P8/P9/P10: THE ADDRESS. Identity, both samples. --------------------------------
  say "  P7-P10 ★★★★★ THE POLLED ADDRESS — the whole rung. Both samples, verbatim:"
  grep -E 'POLLED ADDRESS|VALUE AT IT|SLOT-JOIN|cached value|chain +:|obj\[0x94' "$P" \
    | sed 's/^/     /' | head -40
  say "     --- distinct polled addresses in this arm (identity, not a count):"
  grep -oE 'POLLED ADDRESS *= 0x[0-9a-f]+' "$P" | sort -u | sed 's/^/       /'
  say "     --- distinct values read at them:"
  grep -A1 -E 'POLLED ADDRESS *= 0x' "$P" | grep -oE 'VALUE AT IT = 0x[0-9a-f]+ \([-0-9]+\)' \
    | sort | uniq -c | sed 's/^/       /'
  say "     --- SLOT-JOIN verdicts (P8: predicted NEITHER, p=.65):"
  grep -oE 'SLOT-JOIN: .*' "$P" | sort | uniq -c | sed 's/^/       /'
  say "     --- the mapping the address falls in (P9):"
  grep -oE 'POLLED ADDRESS *= 0x[0-9a-f]+ +pageoff=0x[0-9a-f]+ +.*' "$P" | sort -u \
    | sed 's/^/       /' | head -8

  # --- carried guards --------------------------------------------------------------------
  row CARRY "COMPLETION-WATCH OBSERVED (w268: pass=8, refuse=0)" \
      "$(grep -c 'COMPLETION-WATCH.*OBSERVED' "$Q" 2>/dev/null)"
  row CARRY "GR-REPORT-SEMAPHORE slots written" \
      "$(grep -o 'SEMA-PAGE-SLOT va=0x[0-9a-f]*' "$Q" 2>/dev/null | sort -u | wc -l)"
  say "  CARRY  Xid IDENTITY (⚠ never a count):"
  grep -o 'Xid.*' "$H" 2>/dev/null | cut -c1-170 | sort -u | sed 's/^/       /' \
    || say "       (no host dmesg Xid — check the watermark line before reading this as zero)"
  row GUARD "ENOSPC / LLVM ERROR in this arm's qemu log" \
      "$(grep -c 'No space left on device\|LLVM ERROR' "$Q" 2>/dev/null)"
  row GUARD "probe log bytes (⊘ zero is 'not recorded', not 'nothing')" "$(wc -c < "$P")"
done

say ""
say "================ CROSS-ARM: THE ONE COMPARISON THIS RUNG EXISTS FOR ================"
say "★★★★★ P7: is the polled address THE SAME on both arms? If yes, the guest is not looking"
say "      at the page w268 filled, and w268's eight satisfied slots are irrelevant to this wait."
for arm in refuse pass; do
  P="$B/run_w269_${arm}_probe.log"
  printf '  %-8s addrs: %s\n' "$arm" \
    "$(grep -oE 'POLLED ADDRESS *= 0x[0-9a-f]+' "$P" 2>/dev/null | grep -oE '0x[0-9a-f]+' | sort -u | tr '\n' ' ')"
  printf '  %-8s vals : %s\n' "$arm" \
    "$(grep -oE 'VALUE AT IT = 0x[0-9a-f]+' "$P" 2>/dev/null | sort -u | tr '\n' ' ')"
done
say ""
say "⚠ REMINDER: these are GUEST PROCESS VAs. The join to the 0x2_0440ff80 GPU VA is by PAGE"
say "  OFFSET and mapping name only — suggestive, never conclusive (prereg §4.2)."
