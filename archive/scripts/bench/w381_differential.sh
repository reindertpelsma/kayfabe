#!/usr/bin/env bash
# ★★★★★ w381 — THE DIFFERENTIAL, AS ONE COMMAND. Same binary, native arm and Mode-2 guest
# arm, graded as a PAIR.
#
#   usage: KAYFABE_REPO=<tree> CARGO_TARGET_DIR=<dir> scripts/bench/w381_differential.sh [tag]
#
# ## ★★★ WHY A PAIR AND NOT TWO RUNS
#
#   A GUEST-ONLY RED CANNOT DISTINGUISH "we are broken" FROM "the probe is wrong."
#
# The native arm is the CONTROL: it says the host driver permits exactly what the guest arm
# asks for. Only the two rows together are a result. Both arms run the **same musl binary**
# and its md5 is printed on both sides, so "the same program" is a measurement rather than an
# assumption.
#
# ## THE THREE ROWS THIS PRINTS
#
#   native  / launch-dma   — bare metal, the new primitive. ★ Also the probe's own control:
#                            if the copy probe cannot reproduce the sem-release arm's result
#                            natively, the substitution is unsound and the guest row means
#                            nothing.
#   native  / sem-release   — bare metal, the w379 primitive, for continuity with w379's
#                            committed result.
#   guest   / launch-dma   — the deliverable.
#
# ⊘ **`--missing-page-fault` PROVOKES A REAL `Xid 31`** on each arm and kills its victim
#   channel. That is the measurement, and each arm's bystander is what proves the fault took
#   nothing else with it.
# ⊘ **GRADED ON PRINTED LINES, NEVER ON EXIT STATUS.**
set -uo pipefail
REPO=${KAYFABE_REPO:-/root/kayfabe}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w381}
TAG=${1:-w381}
BIN=$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder
OUT=/workspace/${TAG}_differential.log

{
echo "=== ★★★★★ W381 DIFFERENTIAL START $(date -Is) ==="
echo "=== HEAD=[$(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo '⊘ NO REPO — UNATTRIBUTABLE')]"
echo "=== DIRT=[$(git -C "$REPO" status --porcelain --untracked-files=no 2>/dev/null | head -3)]"
if [ ! -x "$BIN" ]; then
  echo "W381_DIFF_OUTCOME=(E) ⊘ UNMEASURED_NO_BINARY — no $BIN"
  echo "    build: cargo build --release --target x86_64-unknown-linux-musl --bin kayfabe-rm-ladder"
  exit 0
fi
echo "=== BIN=$BIN md5=$(md5sum < "$BIN" | cut -d' ' -f1) $(stat -c %s "$BIN") bytes"

echo ""
echo "################ ARM 1/3 — NATIVE, launch-dma (the probe's own control) ################"
W381_ARM=native KAYFABE_TAG=${TAG}_ndma W381_RUNGS="--w381" \
  bash "$REPO/scripts/bench/w381_guest_servable.sh"

echo ""
echo "################ ARM 2/3 — NATIVE, sem-release (continuity with w379) ################"
W381_ARM=native KAYFABE_TAG=${TAG}_nsem \
  W381_RUNGS="--w379 --rpc-mixed-allocs --cross-client-leak --probe-sem-release" \
  bash "$REPO/scripts/bench/w381_guest_servable.sh"

echo ""
echo "################ ARM 3/3 — MODE-2 GUEST, launch-dma ################"
# ⚠ `KAYFABE_CE_EXECUTOR=local` is the arm under test and it is set EXPLICITLY, not left to a
#   default: with `local` the shell's CPU copy engine serves every CE doorbell, which is the
#   executor whose `SemRelease` refusal this whole lane exists to route around. With `host`
#   the same doorbells go to the forwarding plane and the rungs would be measuring something
#   else entirely under the same name.
# ⊘ `w290p_run.sh` refuses a dirty tree and gates on the shim's own revision stamp, so a
#   guest row can never be attributed to a revision that was not built.
KAYFABE_TAG=${TAG}_guest \
KAYFABE_CE_EXECUTOR=local \
GQ_TIMEOUT=${GQ_TIMEOUT:-900} \
KAYFABE_W381_BIN="$BIN" \
KAYFABE_W381_ARGS="--w381" \
POST_CAPTURE_HOOK="$REPO/scripts/bench/w381_hook.sh" \
  bash "$REPO/scripts/bench/w290p_run.sh" drain
echo "=== guest arm inner rc=$? ==="
# ⊘ The GUEST row lives in `boot_capture.sh`'s PROBE log, because that is where a
#   POST_CAPTURE_HOOK's output is written — NOT in `w290p_run.sh`'s own `/workspace/<tag>.log`.
#   Reading the wrong one of those two is how this table printed `⊘ NO TABLE ROW` over a
#   guest arm that had graded 6/7.
GOUT=/workspace/bench/run_${TAG}_guest_probe.log

echo ""
echo "================================================================================"
echo "=== ★★★★★ THE DIFFERENTIAL TABLE — the deliverable, one row per arm"
echo "================================================================================"
printf '    %-24s %-12s %s\n' ARM PROBE VERDICTS
row() { # $1 label, $2 log
  local r
  r=$(grep -ah 'W381_TABLE_ROW' "$2" 2>/dev/null | tail -1)
  printf '    %-24s %s\n' "$1" "${r:-⊘ NO TABLE ROW — this arm produced no grade at all}"
}
row "native/launch-dma"  /workspace/bench/run_${TAG}_ndma_native.log
row "native/sem-release" /workspace/bench/run_${TAG}_nsem_native.log
row "guest/launch-dma"   "$GOUT"

echo ""
echo "=== ★ PER-RUNG, SIDE BY SIDE (the row that matters; ⊘ NONE means no verdict line)"
printf '    %-18s %-10s %-10s %-10s\n' RUNG native-dma native-sem guest-dma
for r in map_propagation alias_two_vas alias_unmap missing_page map_stress rpc_mixed cross_client; do
  a=$(sed -n "s/^RUNG_${r}=//p" /workspace/bench/run_${TAG}_ndma_native.log 2>/dev/null | tail -1)
  b=$(sed -n "s/^RUNG_${r}=//p" /workspace/bench/run_${TAG}_nsem_native.log 2>/dev/null | tail -1)
  c=$(grep -ah "RUNG=" "$GOUT" 2>/dev/null | sed -n "s/^ *${r} *RUNG=\([A-Z]*\).*/\1/p" | tail -1)
  printf '    %-18s %-10s %-10s %-10s\n' "$r" "${a:-NONE}" "${b:-NONE}" "${c:-NONE}"
done

echo ""
echo "=== ★★★ THE PAIRING RULE, applied"
echo "    * native PASS + guest PASS  ⇒ the emulated path does what the driver does."
echo "    * native PASS + guest FAIL  ⇒ ★★★★★ OURS. Attributable, because the control passed."
echo "    * native FAIL               ⇒ ⊘ the PROBE or the HOST, never the guest. Fix that first."
echo "    * native PASS + guest NOTRUN ⇒ ⊘ the guest arm never reached its control."
echo "=== W381 DIFFERENTIAL EXIT at $(date -Is) ==="
} 2>&1 | tee "$OUT"
