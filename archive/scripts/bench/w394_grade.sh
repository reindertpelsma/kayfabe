#!/usr/bin/env bash
# ★★★★★ w394 grader — CORRECTNESS verdicts first, then parity ratios.
#
# usage: w394_grade.sh <native.log> <guest.log>
#
# ## Pre-registered outcomes, per probe
#   (P)  the probe's OWN verification line says it passed AND its RC is 0
#   (F)  the probe ran and its verification says it FAILED — a forged pass caught
#   (E)  ⊘ UNMEASURED — no BEGIN/RC pair, RC=NOTBUILT, or RC=124 (timeout). NOT a failure.
#
# ⊘ A timing probe can never score (P) here. `gpu_bench` verifies nothing; scoring it as a
#   pass because it printed GFLOP/s is precisely the shape this campaign has been bitten by.
set -uo pipefail
NAT="${1:?usage: w394_grade.sh <native.log> <guest.log>}"
GST="${2:?usage: w394_grade.sh <native.log> <guest.log>}"

rc_of()  { grep -m1 "^W394_${2}_RC_${1}=" "$3" 2>/dev/null | cut -d= -f2; }
sec_of() { awk -v n="$1" -v a="$2" '$0 ~ ("^--- W394_" a "_BEGIN " n " ") {p=1;next} $0 ~ ("^--- W394_" a "_END " n " ") {p=0} p' "$3" 2>/dev/null; }

verdict() {   # verdict <name> <ARM> <log> <positive-regex> <negative-regex>
  local name="$1" arm="$2" log="$3" pos="$4" neg="$5"
  local rc; rc="$(rc_of "$name" "$arm" "$log")"
  local body; body="$(sec_of "$name" "$arm" "$log")"
  if [ -z "$rc" ] || [ "$rc" = NOTBUILT ]; then echo "(E) ⊘ UNMEASURED rc=${rc:-ABSENT}"; return; fi
  if [ "$rc" = 124 ]; then echo "(E) ⊘ UNMEASURED — TIMED OUT"; return; fi
  if echo "$body" | grep -qE "$neg"; then
    echo "(F) ★ VERIFICATION FAILED — $(echo "$body" | grep -m1 -E "$neg" | head -1)"; return; fi
  if [ "$rc" = 0 ] && echo "$body" | grep -qE "$pos"; then echo "(P) $(echo "$body" | grep -m1 -E "$pos" | head -1)"; return; fi
  echo "(F) rc=$rc and no verification line — $(echo "$body" | tail -1)"
}

# num <name> <ARM> <log> <regex> <field>   — field is a number, or "last1" for $(NF-1).
# ⊘ awk turns a non-numeric -v into 0, so "NF-1" passed as a field expression silently
#   prints $0. It has to be a named case, not an expression handed to $().
num() {
  sec_of "$1" "$2" "$3" | awk -v re="$4" -v f="$5" '$0 ~ re { print (f=="last1" ? $(NF-1) : $(f+0)); exit }'
}

ratio() { awk -v a="$1" -v b="$2" 'BEGIN{ if (a=="" || b=="" || b+0==0) {print "n/a"} else {printf "%.3f", a/b} }'; }

echo "════════════════════════════════════════════════════════════════════"
echo " ★ w394 — lane (1) CUDA APPS.  native=$(basename "$NAT")  guest=$(basename "$GST")"
echo "════════════════════════════════════════════════════════════════════"
echo
echo "── CORRECTNESS (the probe verifies its own output against a CPU expectation) ──"
printf '%-22s %-46s %s\n' PROBE 'GUEST (Mode-2)' 'NATIVE (control)'
P=0; F=0; E=0
grade_pair() {
  local name="$1" pos="$2" neg="$3"
  local g n
  g="$(verdict "$name" GUEST "$GST" "$pos" "$neg")"
  n="$(verdict "$name" NATIVE "$NAT" "$pos" "$neg")"
  printf '%-22s %-46s %s\n' "$name" "${g:0:46}" "${n:0:40}"
  case "$g" in "(P)"*) P=$((P+1));; "(F)"*) F=$((F+1));; *) E=$((E+1));; esac
}
grade_pair vector_add_test      '^PASS:'                     'VERIFY FAILED|mismatches'
grade_pair matmul_test          '^PASS:'                     'VERIFY:|mismatches'
grade_pair big_memcpy_test      '^PASS: .*round-tripped'     '^FAIL:|mismatches'
grade_pair mem_bandwidth_probe  'correctness: OK'            'correctness FAIL'
echo
echo "W394_CORRECT_PASS=$P  W394_CORRECT_FAIL=$F  W394_CORRECT_UNMEASURED=$E"
if   [ "$F" -gt 0 ]; then echo "W394_OUTCOME=(F) ★★★ a verifying probe reported WRONG BYTES in the guest"
elif [ "$E" -gt 0 ]; then echo "W394_OUTCOME=(E) ⊘ INCOMPLETE — $E of 4 verifying probes produced no verdict"
elif [ "$P" -eq 4 ]; then echo "W394_OUTCOME=(P) ★★★★★ all 4 verifying CUDA apps are byte-correct in the Mode-2 guest"
else echo "W394_OUTCOME=(E) ⊘ the counters do not add up — read the sections, not this line"; fi
echo
echo "── PARITY (timing only; ⊘ none of these verify a result byte) ──"
printf '%-30s %14s %14s %10s  %s\n' METRIC GUEST NATIVE 'g/n' DIRECTION
row() { printf '%-30s %14s %14s %10s  %s\n' "$1" "${2:-ABSENT}" "${3:-ABSENT}" "$(ratio "${2:-}" "${3:-}")" "$4"; }

g_gf=$(num gpu_bench GUEST  "$GST" 'A throughput' last1); n_gf=$(num gpu_bench NATIVE "$NAT" 'A throughput' last1)
g_lr=$(num gpu_bench GUEST  "$GST" 'B launch RTT' 8);      n_lr=$(num gpu_bench NATIVE "$NAT" 'B launch RTT' 8)
g_ar=$(num gpu_bench GUEST  "$GST" 'C alloc RTT'  8);      n_ar=$(num gpu_bench NATIVE "$NAT" 'C alloc RTT'  8)
g_h2=$(num mem_bandwidth_probe GUEST "$GST" '^H2D:' 2);    n_h2=$(num mem_bandwidth_probe NATIVE "$NAT" '^H2D:' 2)
g_dd=$(num mem_bandwidth_probe GUEST "$GST" '^D2D:' 2);    n_dd=$(num mem_bandwidth_probe NATIVE "$NAT" '^D2D:' 2)
g_d2=$(num mem_bandwidth_probe GUEST "$GST" '^D2H:' 2);    n_d2=$(num mem_bandwidth_probe NATIVE "$NAT" '^D2H:' 2)

row 'GEMM GFLOP/s'        "$g_gf" "$n_gf" 'higher is better'
row 'launch RTT us'       "$g_lr" "$n_lr" 'LOWER is better'
row 'alloc RTT us'        "$g_ar" "$n_ar" 'LOWER is better'
row 'H2D GB/s'            "$g_h2" "$n_h2" 'higher is better'
row 'D2D GB/s'            "$g_dd" "$n_dd" 'higher is better'
row 'D2H GB/s'            "$g_d2" "$n_d2" 'higher is better'
echo
echo "── cuda_micro, per subsystem (guest arm verbatim) ──"
sec_of cuda_micro GUEST "$GST" | grep -E '^[0-9]' || echo '(no cuda_micro section in the guest log)'
echo
echo "── ⊘ what this grade CANNOT say ──"
echo "  * nothing about UVM: cuda_micro subtests 5/6 use cuMemAllocManaged, which is lane (4)."
echo "  * nothing about the 10 .cu kernels — they need nvcc and were not built."
echo "  * the parity numbers are FOR THE ARMING OF THIS BOOT. Name the arming when quoting them."
