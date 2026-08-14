#!/usr/bin/env bash
# ★★★★★ w304 — CONFIRM THE FIVE INERT RELAXATIONS AT n=2, THEN DELETE THEM.
#
#   usage: w304_confirm.sh <label> [VAR=VALUE ...]
#
# `w298_ablate.sh` measured each of the eleven ONCE. One boot is not a basis for deleting
# code from master, so every arm this script runs is the SECOND measurement of a cell w298
# already filled. The pre-registered reading is w298's, unchanged:
#
#   (A) `^CUP3_VAL=43` with the variable OFF  ⇒ CONFIRMED INERT. Delete it.
#   (B) `^CUP3_VAL=` != 43 with it OFF        ⇒ ★★★ w298's CELL WAS WRONG. Say so, loudly,
#       and say WHAT it was holding up by identity. This is the more valuable outcome.
#   (C) `^CUP3_VAL=` ABSENT                   ⇒ ⊘ UNMEASURED. Not 0, not a failure value.
#   (D) baseline != 43                        ⇒ ★★★ STOP — nothing below it means anything.
#
# ## ★★★ WHAT THIS ADDS OVER `w298_ablate.sh`: A DEVICE-SIDE WITNESS FOR ALL FIVE ARMS
#
# w298's `ARM IN FORCE` row read three emissions (`VAS-PUBLISH arm=`, `OPERAND-JOIN arm=`,
# `PT-SWEEP ... ran=`). Three of the five arms under test here — `GUEST_PUSHBUF`,
# `GUEST_SEMA`, `GUEST_OPERAND` — emit NONE of those, so on w298's rows their arm was
# witnessed only by the script's own record of intent. That is precisely the failure w298
# existed to end: *an arm you set is not an arm in force.*
#
# ★ Each of the three DOES have a device-side signature, measured from w298's own qemu logs:
#
#   | arm off             | baseline | with the arm off |
#   |---------------------|----------|------------------|
#   | `GUEST_PUSHBUF=off` | PB-PIN   1142 | PB-PIN   **0**   |
#   | `GUEST_SEMA=off`    | SEMA-PIN  458 | SEMA-PIN **0**   |
#   | `GUEST_OPERAND=off` | OPERAND-PIN 325 | OPERAND-PIN **0** |
#   | `PT_SWEEP=off`      | PT-SWEEP  230 | PT-SWEEP   **1** |
#
# ⊘⊘ **CORRECTED MID-RUNG, AND THE CORRECTION IS THE LESSON.** This table first said
#   `GUEST_OPERAND=off` shows as **`PB-PIN` 1142 → 637**, inferred from w298's single pair of
#   aggregate counts. That is WRONG. The operand pin's own label is `OPERAND-PIN`, and the
#   real witness is `OPERAND-PIN` **325 → 0** — measured on w298goperand AND w304goperand,
#   both. ⚠ `PB-PIN` is not a witness for this arm at all: across six GREEN w304 boots it read
#   1142, 1142, 458, 0, 1142, 1142 — it moves with the workload, so a "count change" in it
#   would have confirmed nothing. `a_count_cannot_see_a_substitution`, committed by the very
#   file written to avoid it.
set -uo pipefail

LABEL=${1:-}
[ -n "$LABEL" ] || { echo "usage: $0 <label> [VAR=VALUE ...]" >&2; exit 64; }
shift

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w304}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w304}
export KAYFABE_TAG="w304${LABEL}"
export GQ_TIMEOUT=${GQ_TIMEOUT:-900}

NOVR=0
OVR_DESC="(none — BASELINE)"
DESCS=""
for kv in "$@"; do
  case "$kv" in
    *=*) ;;
    *) echo "★ not a VAR=VALUE override: [$kv]" >&2; exit 64 ;;
  esac
  k=${kv%%=*}; v=${kv#*=}
  NOVR=$((NOVR + 1))
  DESCS="$DESCS $k=$v"
  if [ "$k" = "KAYFABE_VAS_PUBLISH" ]; then
    export W298_ARM="$v"
  else
    export "$k=$v"
  fi
done
[ "$NOVR" -eq 0 ] || OVR_DESC="$DESCS"
if [ "$NOVR" -gt 1 ] && [ "${W298_MULTI:-0}" != "1" ]; then
  echo "★ $NOVR overrides in one boot makes the outcome unattributable. Set W298_MULTI=1 if" >&2
  echo "  that is genuinely what you want (the w304 JOINT boot does, deliberately)." >&2
  exit 64
fi

OUT=/workspace/${KAYFABE_TAG}.log
S=/workspace/w304_summary.txt
Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log

echo "=== W304 START label=$LABEL overrides=[$OVR_DESC] $(date -Is) pid=$$ ===" | tee -a "$S"

"$REPO/scripts/bench/w297_cup3.sh"
ARC=$?

gc() { if [ -e "$2" ]; then grep -c -- "$1" "$2" 2>/dev/null || true; else echo "⊘UNMEASURED"; fi; }
gco() { if [ -e "$2" ]; then grep -o -- "$1" "$2" 2>/dev/null | wc -l; else echo "⊘UNMEASURED"; fi; }

VAL=$(grep -oE '^CUP3_VAL=[0-9]+' "$P" 2>/dev/null | tail -1)
VAL=${VAL:-⊘ABSENT-UNMEASURED}
RC=$(grep -oE '^CUP3_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
FIRSTX=$(grep -E '^    ✘ ' "$P" 2>/dev/null | head -1 | sed 's/^ *//')
LADDER_OK=$(grep -cE '^    ✔ ' "$P" 2>/dev/null || true)
XID=$(gc Xid "$D")
XIDLINE=$(grep -oE 'Xid[^,]*, pid=[0-9]*[^,]*' "$D" 2>/dev/null | head -1)
FAULTLVL=$(grep -oE 'FAULT_(PDE|PTE)[0-9]*' "$D" 2>/dev/null | sort -u | tr '\n' ' ')
FENG=$(grep -oE 'ENGINE [A-Z0-9_]+|HUBCLIENT_[A-Z0-9_]+|ACCESS_TYPE_[A-Z_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')
FVA=$(grep -oE 'faulted @ 0x[0-9a-f_]+' "$D" 2>/dev/null | tail -1 | grep -oE '0x[0-9a-f_]+')
# ⚠ NUMERIC sort. w298 published `4 of 13348` as a peak by sorting these as strings and had
#   to withdraw a mechanism claim built on it.
HMAX=$(grep -oE 'host_rows=[0-9]+ of [0-9]+' "$Q" 2>/dev/null | cut -d= -f2 | cut -d' ' -f1 | sort -n | tail -1)
HLAST=$(grep -oE 'host_rows=[0-9]+ of [0-9]+' "$Q" 2>/dev/null | tail -1)
HPUB=$(gc 'HOST-PUBLISHED' "$Q")

# ★ THE ARM, READ FROM THE DEVICE'S OWN EMISSIONS — three named lines plus four counts.
A_VAS=$(grep -oE 'VAS-PUBLISH arm=[a-z]+' "$Q" 2>/dev/null | head -1)
A_OPJ=$(grep -oE 'OPERAND-JOIN arm=[a-z]+' "$Q" 2>/dev/null | head -1)
A_SWP=$(grep -oE 'PT-SWEEP tasks=[0-9]+ skipped=[0-9]+ ran=[0-9]+' "$Q" 2>/dev/null | tail -1)
N_PB=$(gco 'PB-PIN' "$Q")
N_OP=$(gco 'OPERAND-PIN' "$Q")
N_SEMA=$(gco 'SEMA-PIN' "$Q")
N_SWEEP=$(gco 'PT-SWEEP' "$Q")
N_OPJT=$(gco 'OPERAND-JOIN-TABLE:' "$Q")

DOOR=$(grep -o 'by engine: .*' "$Q" 2>/dev/null | tail -1)
JIT=$(grep -oE 'CUP3_JIT_PRESENT=(yes|no)' "$P" 2>/dev/null | tail -1)
UNSERV=$(grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$Q" 2>/dev/null | sort -u | wc -l)
REFUSED=$(grep -oE '⊘REFUSED `[^`]*`' "$Q" 2>/dev/null | sort | uniq -c | tr '\n' ' ')
PINNED=$(grep -oE 'DRAIN\[[^]]*\]' "$Q" 2>/dev/null | tail -1)
DRAINED=$(gco '★DRAINED' "$Q")
RELAX=$(grep -cE '^    KAYFABE_[A-Z_]+ = \[KAYFABE_[A-Z_]+=[a-z]+\]' "$OUT" 2>/dev/null || true)

{
echo "--------------------------------------------------------------------------------"
echo "W304 ROW  label=$LABEL  overrides=[$OVR_DESC]  arc=$ARC  $(date -Is)"
echo "  ^CUP3_VAL      = $VAL          ^CUP3_RC = ${RC:-⊘ABSENT}"
echo "  JIT precond    = ${JIT:-⊘ABSENT-UNMEASURED}"
echo "  ladder ✔count  = ${LADDER_OK}   first ✘ = ${FIRSTX:-<none>}"
echo "  Xid count      = $XID   [${XIDLINE:-no Xid line}]"
echo "  fault level    = ${FAULTLVL:-⊘none printed}"
echo "  fault engine   = ${FENG:-⊘none printed}"
echo "  fault VA       = ${FVA:-⊘none}"
echo "  host_rows MAX  = ${HMAX:-⊘ABSENT-UNMEASURED}   (numeric sort)   last=${HLAST:-⊘ABSENT}"
echo "  HOST-PUBLISHED lines = $HPUB   ⊘ 0 means the line NEVER PRINTED — unmeasured, not zero"
echo "  ARM (device)   = ${A_VAS:-⊘no VAS-PUBLISH arm line} ${A_OPJ:-⊘no OPERAND-JOIN arm line} ${A_SWP:-⊘no PT-SWEEP ran line}"
echo "  ARM (counts)   = PB-PIN=$N_PB  SEMA-PIN=$N_SEMA  OPERAND-PIN=$N_OP  PT-SWEEP=$N_SWEEP  OPERAND-JOIN-TABLE=$N_OPJT"
echo "                   ⊘ baseline: PB-PIN 1142 / SEMA-PIN 458 / OPERAND-PIN 325 / PT-SWEEP 230 / OJT 96"
echo "                   ⚠ GUEST_OPERAND=off is witnessed by OPERAND-PIN 325→0. ⊘ NOT by PB-PIN,"
 echo "                     which reads 1142/458/0/1142 across GREEN boots — it tracks the workload"
echo "  drain          = ${PINNED:-⊘ABSENT}   ★DRAINED rows = $DRAINED"
echo "  doorbells      = ${DOOR:-⊘ABSENT-UNMEASURED}"
echo "  unserviced ids = $UNSERV"
echo "  refused pins   = ${REFUSED:-none}"
echo "  relaxation report rows = $RELAX"
echo "  logs: $OUT  Q=$(stat -c%s "$Q" 2>/dev/null || echo MISSING)b  P=$(stat -c%s "$P" 2>/dev/null || echo MISSING)b"
echo "=== W304 ROW EXIT label=$LABEL rc=$ARC $(date -Is) ==="
} | tee -a "$S"

exit "$ARC"
