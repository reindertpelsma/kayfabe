#!/usr/bin/env bash
# ★★★★★ w328 — DISTIL ONE BOOT's PUBLICATION PLANE into a committable census.
#
# ⊘ The qemu log is ~3.6 MB per boot and cannot be committed for 15 boots. This writes the
#   ~40 lines that carry the rung's evidence, so the artefact survives the box.
#
# ⚠ EVERY FIELD PRINTS `⊘UNMEASURED` WHEN ITS LINE IS ABSENT. A boot whose instrument did not
#   run must not distil to a file full of zeros that reads exactly like a boot where the
#   breadth cost nothing — this tree's own `absent-artefact-reads-as-favourable` class, and it
#   is the specific way a truncated artefact "reads as PRESENT and looks healthier than an
#   empty one".
#
#   usage: w328_census.sh <tag> [<qemu.log>]
set -uo pipefail
TAG=${1:?usage: w328_census.sh <tag> [<qemu.log>]}
Q=${2:-/workspace/bench/run_${TAG}_qemu.log}

echo "=== W328 CENSUS tag=$TAG log=$Q $(date -Is)"
[ -r "$Q" ] || { echo "⊘ NO LOG AT $Q — EVERYTHING BELOW IS UNMEASURED, ⊘ NOT ZERO"; exit 2; }
echo "=== log bytes = $(stat -c%s "$Q")"

echo
echo "--- ★ THE ARM WORDS, from the boot's own lines (⊘ never from the launcher's env)"
grep -ao 'arm=drain W328SCOPE\[arm=[a-z]* scoped=[a-z]*' "$Q" | tail -1 \
  || echo "⊘ NO W328SCOPE ARM WORD — UNMEASURED / OLD BINARY"
grep -ao 'W328PIN\[arm=[a-z]* scoped=[a-z]*' "$Q" | tail -1 \
  || echo "⊘ NO W328PIN ARM WORD — UNMEASURED / OLD BINARY"
grep -ao 'W321BATCH\[arm=[a-z]*[^]]*\]' "$Q" | grep -av 'chains=0 ' | head -1 \
  || echo "⊘ NO W321BATCH LINE — UNMEASURED"
G=$(grep -ao 'gate=\(on\|off\) this_doorbell\[[^]]*\]' "$Q" | tail -1)
echo "${G:-⊘ NO gate= WORD — UNMEASURED, ⊘ NOT 'off'}"
grep -ao 'DIRTY-GATE [^|]*' "$Q" | tail -1 || echo "⊘ NO DIRTY-GATE CENSUS — UNMEASURED"

echo
echo "--- ★★★★★ THE DRAIN (the DOORBELLED VAS's guest-RAM pin drain — drain #2 of four)"
grep -ao 'DRAIN\[visited=true asked=[0-9]* pinned=[0-9]* refused=[0-9]* DRAIN_MS=[0-9]* W319KNOB\[[^]]*\] complete=[a-z]*[^]]*' "$Q" \
  | grep -av 'asked=0 pinned=0 refused=0 DRAIN_MS=0' | head -3
grep -ao 'DRAIN_MS=[0-9]*' "$Q" | grep -av 'DRAIN_MS=0$' | head -1 \
  || echo "⊘ NO NON-ZERO DRAIN_MS — UNMEASURED, ⊘ not 'the drain was free'"
# ⊘ ONLY the drained VAS's row. A bare `last_pinned_va=` grep also matches the SEMAPIN and
#   fallback rows, whose values are HOST VAs — and w319's discriminator compares this against
#   a GUEST fault VA, so the wrong one is not merely noisy, it is a wrong verdict.
grep -ao '★DRAINED(this doorbell.s VAS) asked=[0-9]* pinned=[0-9]* refused=[0-9]* in [0-9]* ms last_pinned_va=[^ ]*' "$Q" \
  | grep -av 'asked=0 ' | sort -u | head -6 \
  || echo "⊘ NO ★DRAINED ROW WITH asked>0 — last_pinned_va UNMEASURED"
# ⊘⊘ NOTE, so nobody re-chases it: the LATER drains end at host-SHAPED VAs (0x7610_3a9f_f000).
#   That is UVM unified addressing — `shape_cannot_discriminate_origin.md`: the GPU VA IS the
#   process VA, so one range, two producers. It is NOT a leaked host pointer.
grep -ao 'TRAPWITNESS [^"]*' "$Q" | tail -1 || echo "⊘ NO TRAPWITNESS — worst_trap UNMEASURED"

echo
echo "--- ★★★★★ THE BREADTH, CUMULATIVE over every publication pass"
grep -ao 'W328SCOPE\[[^]]*\]' "$Q" \
  | sed -E 's/.*scoped_out=([0-9]+) target_us=([0-9]+) target_published=([0-9]+) other_vases=([0-9]+) other_us=([0-9]+) other_published=([0-9]+).*/\1 \2 \3 \4 \5 \6/' \
  | awk '{so+=$1; t+=$2; tp+=$3; ov=$4; o+=$5; op+=$6; n++}
         END {if(n==0) print "⊘ NO W328SCOPE PASS — UNMEASURED, ⊘ NOT zero breadth";
              else printf "passes=%d scoped_out=%d target_us=%d target_published=%d other_vases=%d other_us=%d other_published=%d breadth_share=%.4f%%\n", n, so, t, tp, ov, o, op, (t+o>0)?100*o/(t+o):0}'
grep -ao 'W328PIN\[[^]]*\]' "$Q" \
  | sed -E 's/.*scoped_out=([0-9]+) other_vases=([0-9]+) other_us=([0-9]+) other_pinned=([0-9]+) drain_ms=([0-9]+).*/\1 \2 \3 \4 \5/' \
  | awk '{so+=$1; ov=$2; o+=$3; p+=$4; if($5>d) d=$5; n++}
         END {if(n==0) print "⊘ NO W328PIN PASS — UNMEASURED";
              else printf "pin passes=%d scoped_out=%d other_vases(last)=%d other_us=%d other_pinned=%d max_drain_ms=%d\n", n, so, ov, o, p, d}'

echo
echo "--- ★★★ THE PER-PASS DISTRIBUTION of the DOORBELLED VAS's own census+join cost"
echo "---     ⊘ The MEAN is a lie here: this is BIMODAL. Quantiles, then the tail."
grep -ao 'W328SCOPE\[[^]]*\]' "$Q" | sed -E 's/.*target_us=([0-9]+).*/\1/' | sort -n \
  | awk '{a[NR]=$1} END {if(NR==0){print "⊘ UNMEASURED"; exit}
          printf "n=%d min=%d p25=%d median=%d p75=%d p95=%d max=%d\n", NR, a[1], a[int(NR*0.25)+1], a[int(NR/2)+1], a[int(NR*0.75)+1], a[int(NR*0.95)+1], a[NR]}'
grep -ao 'W328SCOPE\[[^]]*\]' "$Q" | sed -E 's/.*target_us=([0-9]+).*/\1/' \
  | awk '{if($1>10000){n++;s+=$1}else{m++;t+=$1}}
         END {printf "expensive(>10ms): n=%d sum=%dus    cheap(<=10ms): n=%d sum=%dus\n", n, s, m, t}'

echo
echo "--- ★★★★★ WHAT THE EXPENSIVE PASSES WERE DOING — the candidate/published/refused census"
echo "---     of the DOORBELLED VAS, one row per distinct shape, with its multiplicity."
# ⊘ Narrowed to the DOORBELLED pdb, read out of the boot's own W328SCOPE `target=` field.
#   A bare `proc=2 pdb=0x*` also matches the proc's OTHER, always-empty VAS and would report
#   229 phantom `candidates=0` rows — a count inflated by a VAS nobody doorbelled.
TPDB=$(grep -ao 'W328SCOPE\[[^]]*target=proc=[0-9]* pdb=0x[0-9a-f]*' "$Q" | sed -E 's/.*pdb=(0x[0-9a-f]*)$/\1/' | tail -1)
echo "---     doorbelled pdb = ${TPDB:-⊘UNMEASURED}"
grep -ao "proc=[0-9]* pdb=${TPDB:-NOSUCH} total=[0-9]* [^]]*" "$Q" \
  | sed -E 's/.*candidates=([0-9]+)\(.*published=([0-9]+) refused=([0-9]+).*/candidates=\1 published=\2 refused=\3/' \
  | sort | uniq -c | sort -rn | head -12 \
  || echo "⊘ NO PER-VAS CENSUS ROW — UNMEASURED"

echo
echo "=== W328 CENSUS TERMINATOR tag=$TAG rc=0 $(date -Is)"
