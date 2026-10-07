#!/usr/bin/env bash
# ★★★★★ THE HARDWARE GATE'S OWN KNOWN-POSITIVE AND KNOWN-NEGATIVE — no GPU, no guest.
#
# > A gate whose failure mode is PASSING EVERYTHING must be re-checked against input that MUST
# > produce a finding, every time it is edited.
#
# ⊘ This exists because the inline version of `stranded_tokens.awk` rotted in exactly that
# direction and the only thing that caught it was a hardware control run — expensive, easy to
# skip, and it had already been skipped once. Run this after ANY edit to the gate.
set -uo pipefail
AWK="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/stranded_tokens.awk"
fails=0
check() { # name expected actual
    if [ "$2" = "$3" ]; then echo "  ok   $1"; else
        echo "  FAIL $1: expected [$2] got [$3]"; fails=$((fails+1)); fi
}

# ── KNOWN-NEGATIVE: rung and never forwarded ⇒ MUST be reported ──────────────────────────
out=$(printf 'kayfabe: DOORBELL-LEDGER tok=0x00000003 passthrough=0 emulated=6 other=0 forwarded=0\n' | awk -f "$AWK")
check "a rung, unforwarded token is stranded" "tok=0x00000003" "$out"

# ── KNOWN-POSITIVE: forwarded ⇒ MUST be silent ───────────────────────────────────────────
out=$(printf 'kayfabe: DOORBELL-LEDGER tok=0x00000003 passthrough=0 emulated=1 other=0 forwarded=5\n' | awk -f "$AWK")
check "a forwarded token is not stranded" "" "$out"

# ⊘ Never rung at all is NOT stranded — it is unmeasured, and reporting it would make every
# idle token a hardware failure.
out=$(printf 'kayfabe: DOORBELL-LEDGER tok=0x00000003 passthrough=0 emulated=0 other=0 forwarded=0\n' | awk -f "$AWK")
check "a token that was never rung is not stranded" "" "$out"

# ⊘ The regression itself: the row's PREFIX must not be mistaken for the token.
out=$(printf 'kayfabe: DOORBELL-LEDGER tok=0x00000009 passthrough=0 emulated=2 other=0 forwarded=0\n' | awk -f "$AWK")
check "the token field is read, not the row prefix" "tok=0x00000009" "$out"

# ⊘ Mixed input: only the stranded rows come back, in order.
out=$(printf 'DOORBELL-LEDGER tok=0x00000003 emulated=6 forwarded=0\nDOORBELL-LEDGER tok=0x00000009 emulated=1 forwarded=3\nDOORBELL-LEDGER tok=0x00000004 emulated=6 forwarded=0\n' | awk -f "$AWK" | tr '\n' ' ')
check "only stranded rows are returned" "tok=0x00000003 tok=0x00000004 " "$out"

# ⊘ Field order must not matter: they are matched by name, so a reordered ledger still works.
out=$(printf 'DOORBELL-LEDGER forwarded=0 emulated=2 tok=0x00000005\n' | awk -f "$AWK")
check "fields are matched by name, not position" "tok=0x00000005" "$out"

if [ "$fails" -eq 0 ]; then echo "GATE_SELFTEST=PASS"; exit 0; fi
echo "GATE_SELFTEST=FAIL ($fails)"; exit 1
