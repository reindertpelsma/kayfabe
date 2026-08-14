#!/usr/bin/env bash
# ★★★★★ w311 — THE DELIVERABLE. Computes guest ÷ native per size, from the two arms' own logs.
#
#   usage: scripts/bench/w311_ratio.sh <guest_probe_log> <native_log>
#
# ## ⊘ Why this is a separate script and takes BOTH logs as arguments
#
# A ratio is only a ratio if BOTH numerators and denominators came from a measurement in this
# campaign. If either log is missing this script prints UNMEASURED and exits non-zero — it
# NEVER falls back to a remembered figure, a published figure, or 1.0. That fallback is how a
# guest-only number silently acquires a reference nobody measured.
#
# ## What it reads
#
# Both arms emit the same `BSUM N=<n> ... med_ms=<x> gflops=<y> batch_per_launch_ms=<z> ...`
# line from the SAME source file. The guest arm's copies are re-emitted by the hook with a
# `GUEST_` prefix; the native arm's are bare. That prefix is the only thing distinguishing
# them, which is why the hook adds it rather than the reader inferring it from the filename.
set -uo pipefail
GP=${1:-}
NL=${2:-}

fail() { echo "    ⊘⊘ $*"; exit 2; }
[ -n "$GP" ] && [ -s "$GP" ] || fail "GUEST LOG MISSING OR EMPTY [$GP] — the ratio is UNMEASURED."
[ -n "$NL" ] && [ -s "$NL" ] || fail "NATIVE LOG MISSING OR EMPTY [$NL] — (F): guest-only, EXPLICITLY UNREFERENCED."

TMPG=$(mktemp); TMPN=$(mktemp)
trap 'rm -f "$TMPG" "$TMPN"' EXIT
grep -hE '^GUEST_BSUM N=' "$GP" 2>/dev/null | sed 's/^GUEST_//' > "$TMPG"
grep -hE '^BSUM N='       "$NL" 2>/dev/null                      > "$TMPN"

GN=$(wc -l < "$TMPG"); NN=$(wc -l < "$TMPN")
echo "    guest BSUM lines = $GN   native BSUM lines = $NN"
[ "$GN" -gt 0 ] || fail "NO GUEST BSUM LINE — the guest never completed a size. UNMEASURED."
[ "$NN" -gt 0 ] || fail "NO NATIVE BSUM LINE — the native arm never completed a size. (F)."

# field extractor: BSUM lines are `k=v` pairs after the leading token
val() { printf '%s\n' "$1" | tr ' ' '\n' | sed -n "s/^$2=//p" | head -1; }

echo ""
printf "    %-6s | %-28s | %-28s | %-9s | %-9s\n" \
       "N" "GUEST med_ms / GFLOPs" "NATIVE med_ms / GFLOPs" "RATIO" "BATCHRAT"
printf "    %-6s-+-%-28s-+-%-28s-+-%-9s-+-%-9s\n" \
       "------" "----------------------------" "----------------------------" "---------" "---------"

ANY=0
while read -r gl; do
  [ -n "$gl" ] || continue
  N=$(val "$gl" N)
  nl=$(grep -E "^BSUM N=$N " "$TMPN" | head -1)
  if [ -z "$nl" ]; then
    printf "    %-6s | %-28s | %-28s | %-9s | %-9s\n" "$N" \
      "$(val "$gl" med_ms) / $(val "$gl" gflops)" "⊘ NO NATIVE RUN AT THIS N" "UNMEAS" "UNMEAS"
    continue
  fi
  ANY=1
  GM=$(val "$gl" med_ms); GF=$(val "$gl" gflops); GB=$(val "$gl" batch_per_launch_ms)
  NM=$(val "$nl" med_ms); NF=$(val "$nl" gflops); NB=$(val "$nl" batch_per_launch_ms)
  R=$(awk -v a="$GF" -v b="$NF" 'BEGIN{ if(b>0) printf "%.5f", a/b; else printf "UNDEF" }')
  RB=$(awk -v a="$GB" -v b="$NB" 'BEGIN{ if(a>0&&b>0) printf "%.5f", b/a; else printf "UNDEF" }')
  printf "    %-6s | %-28s | %-28s | %-9s | %-9s\n" "$N" \
    "$GM / $GF" "$NM / $NF" "$R" "$RB"
done < "$TMPG"

echo ""
echo "    ★ RATIO       = guest GFLOP/s ÷ native GFLOP/s, from the per-launch MEDIAN (steady"
echo "                    state, first launch discarded). THIS IS THE HEADLINE."
echo "    ★ BATCHRAT    = the same ratio from the BATCHED per-launch cost (K launches, ONE"
echo "                    sync). BATCHRAT >> RATIO ⇒ the cost is in the per-launch SUBMIT/"
echo "                    COMPLETION round trip and an LLM could amortise it. BATCHRAT ≈ RATIO"
echo "                    ⇒ the cost is in the ENGINE or the work itself and batching will not"
echo "                    help."
echo ""
echo "=== ★★★ ABSOLUTE OVERHEAD PER LAUNCH — the (D)/(E) discriminator"
echo "    ⊘ A RATIO CANNOT TELL (D) FROM (E). If the guest is a constant FACTOR slower the"
echo "      ratio is flat and the ABSOLUTE gap grows with N; if the guest pays a fixed"
echo "      NUMBER OF MILLISECONDS per launch the ratio IMPROVES with N while the gap stays"
echo "      flat. Those two have opposite implications for an LLM, so the gap is printed."
printf "    %-6s | %-14s | %-14s | %-14s\n" "N" "guest med_ms" "native med_ms" "gap_ms (g-n)"
while read -r gl; do
  [ -n "$gl" ] || continue
  N=$(val "$gl" N); nl=$(grep -E "^BSUM N=$N " "$TMPN" | head -1); [ -n "$nl" ] || continue
  GM=$(val "$gl" med_ms); NM=$(val "$nl" med_ms)
  GAP=$(awk -v a="$GM" -v b="$NM" 'BEGIN{printf "%.3f", a-b}')
  printf "    %-6s | %-14s | %-14s | %-14s\n" "$N" "$GM" "$NM" "$GAP"
done < "$TMPG"
echo "    ⇒ (D) GAP roughly CONSTANT across N ⇒ FIXED per-launch overhead dominates. ★ GOOD"
echo "          NEWS for large models: bigger kernels amortise it, and the ratio improves."
echo "    ⇒ (E) GAP GROWS with N ⇒ we are in the DATA PATH. Name where."

echo ""
echo "=== ★ FIRST LAUNCH vs STEADY STATE — reported separately, or the result is unreadable"
echo "    ⊘ The first launch carries publication and first-touch backing. A large first-launch"
echo "      cost that does NOT recur is a STARTUP fact, not a throughput fact."
printf "    %-6s | %-12s | %-12s | %-12s | %-12s\n" "N" "guest first" "guest med" "native first" "native med"
while read -r gl; do
  [ -n "$gl" ] || continue
  N=$(val "$gl" N); nl=$(grep -E "^BSUM N=$N " "$TMPN" | head -1); [ -n "$nl" ] || continue
  printf "    %-6s | %-12s | %-12s | %-12s | %-12s\n" "$N" \
    "$(val "$gl" first_ms)" "$(val "$gl" med_ms)" "$(val "$nl" first_ms)" "$(val "$nl" med_ms)"
done < "$TMPG"

echo ""
echo "=== ★ COPY LEGS — H2D once per size, D2H median per verified iteration"
printf "    %-6s | %-16s | %-16s | %-16s | %-16s\n" "N" "guest h2d_ms" "native h2d_ms" "guest d2h_ms" "native d2h_ms"
while read -r gl; do
  [ -n "$gl" ] || continue
  N=$(val "$gl" N); nl=$(grep -E "^BSUM N=$N " "$TMPN" | head -1); [ -n "$nl" ] || continue
  printf "    %-6s | %-16s | %-16s | %-16s | %-16s\n" "$N" \
    "$(val "$gl" h2d_ms)" "$(val "$nl" h2d_ms)" "$(val "$gl" d2h_med_ms)" "$(val "$nl" d2h_med_ms)"
done < "$TMPG"
echo "    ⊘ The copies are OUTSIDE the timed launch window in both arms. They are reported"
echo "      because an LLM's weights are resident but its activations are not — but they are"
echo "      NOT part of the headline ratio."

echo ""
echo "=== ★★★★★ THE DISTRIBUTION — the spread, not just the middle"
echo "    ⚠ Timing on a shared bench is noisy. A median with a p90 far above it is a stalled"
echo "      measurement, not a slow plane."
printf "    %-8s %-6s | %-9s %-9s %-9s %-9s\n" "arm" "N" "min" "med" "p90" "max"
while read -r gl; do
  [ -n "$gl" ] || continue
  printf "    %-8s %-6s | %-9s %-9s %-9s %-9s\n" "GUEST" "$(val "$gl" N)" \
    "$(val "$gl" min_ms)" "$(val "$gl" med_ms)" "$(val "$gl" p90_ms)" "$(val "$gl" max_ms)"
done < "$TMPG"
while read -r nl; do
  [ -n "$nl" ] || continue
  printf "    %-8s %-6s | %-9s %-9s %-9s %-9s\n" "NATIVE" "$(val "$nl" N)" \
    "$(val "$nl" min_ms)" "$(val "$nl" med_ms)" "$(val "$nl" p90_ms)" "$(val "$nl" max_ms)"
done < "$TMPN"

[ "$ANY" = 1 ] || fail "NO SIZE WAS MEASURED ON BOTH ARMS — there is no ratio, only two lists."
exit 0
