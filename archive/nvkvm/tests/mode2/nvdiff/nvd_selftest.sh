#!/usr/bin/env bash
# nvd_selftest.sh — validate the differential instrument OFFLINE, with NO GPU.
#
# It runs entirely against the committed reference captures in
# traces/host_reference_ga106/, so the claim "the noise floor is zero" is
# reproducible from the repository by anyone, forever -- not only from a
# session transcript on a rented box.
#
# Two halves, and BOTH must pass:
#   NEGATIVE CONTROL — two runs of the SAME program on the SAME host must diff to
#                      zero unexplained divergences. If they do not, the instrument
#                      is noisy and every guest diff it produces is meaningless.
#   POSITIVE CONTROL — two runs of DIFFERENT programs must diff to a specific,
#                      expected divergence. A differ that reports "no divergence"
#                      for everything is a green test holding a wall in place.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REF="${1:-$HERE/../../../traces/host_reference_ga106}"
PY="python3 $HERE/nvdiff.py"
fail=0

say() { printf '%-46s %s\n' "$1" "$2"; }

echo "=== NEGATIVE CONTROL: host r1 vs host r2, same stage (noise floor) ==="
for s in init dev ctx alloc ce launch; do
    a="$REF/${s}_r1.jsonl.zst"; b="$REF/${s}_r2.jsonl.zst"
    [ -f "$a" ] && [ -f "$b" ] || { say "$s" "SKIP (missing reference)"; continue; }
    out="$($PY diff "$a" "$b" -n 5 2>&1)"
    if echo "$out" | grep -q "RESULT: NO DIVERGENCE"; then
        say "$s" "PASS  $(echo "$out" | grep census)"
    else
        say "$s" "FAIL"; echo "$out" | sed 's/^/    /'; fail=1
    fi
done

echo
echo "=== POSITIVE CONTROL: the differ must DETECT known-different pairs ==="
# dev -> ctx is exactly one cuCtxCreate/cuCtxDestroy pair.
out="$($PY diff "$REF/dev_r1.jsonl.zst" "$REF/ctx_r1.jsonl.zst" -n 1 2>&1)"
n="$(echo "$out" | sed -n 's/.*EXTRA.: \([0-9]*\).*/\1/p')"
first="$(echo "$out" | grep -m1 'EXTRA ' | awk '{print $3}')"
if [ "${n:-0}" -gt 100 ] && [ -n "$first" ]; then
    say "dev vs ctx detects cuCtxCreate" "PASS  ${n} EXTRA, first = $first"
else
    say "dev vs ctx detects cuCtxCreate" "FAIL"; echo "$out" | sed 's/^/    /'; fail=1
fi
# ctx -> alloc is exactly the first cuMemAlloc.
out="$($PY diff "$REF/ctx_r1.jsonl.zst" "$REF/alloc_r1.jsonl.zst" -n 1 2>&1)"
n="$(echo "$out" | sed -n 's/.*EXTRA.: \([0-9]*\).*/\1/p')"
if [ "${n:-0}" -ge 1 ] && [ "${n:-0}" -le 20 ]; then
    say "ctx vs alloc detects cuMemAlloc" "PASS  ${n} EXTRA"
else
    say "ctx vs alloc detects cuMemAlloc" "FAIL"; echo "$out" | sed 's/^/    /'; fail=1
fi

echo
[ $fail -eq 0 ] && echo "SELFTEST PASS" || echo "SELFTEST FAIL"
exit $fail
